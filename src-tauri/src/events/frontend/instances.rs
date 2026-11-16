use super::Error;

use crate::shared::{Action, ActionContext, ActionInstance, ActionState, Context, config_dir};
use crate::store::profiles::{LocksMut, acquire_locks, acquire_locks_mut, get_instance_mut, get_slot, get_slot_mut, save_profile_now};

use tauri::{AppHandle, Emitter, Manager, command};
use tokio::fs::remove_dir_all;

#[command]
pub async fn create_instance(app: AppHandle, mut action: Action, context: Context) -> Result<Option<ActionInstance>, Error> {
	if !action.controllers.contains(&context.controller) {
		return Ok(None);
	}

	if context.controller == "Encoder" {
		if let Err(error) = crate::shared::initialise_encoder_layout(&mut action, None) {
			log::warn!("Failed to initialise encoder layout: {error}");
		}
	}

	let mut locks = acquire_locks_mut().await;
	let slot = get_slot_mut(&context, &mut locks).await?;

	if let Some(parent) = slot {
		let Some(children) = &mut parent.children else { return Ok(None) };
		let index = match children.last() {
			None => 1,
			Some(instance) => instance.context.index + 1,
		};

		let instance = ActionInstance {
			action: action.clone(),
			context: ActionContext::from_context(context.clone(), index),
			states: action.states.clone(),
			current_state: 0,
			settings: serde_json::Value::Object(serde_json::Map::new()),
			children: None,
		};
		children.push(instance.clone());

		if parent.action.uuid == "opendeck.toggleaction" && parent.states.len() < children.len() {
			parent.states.push(crate::shared::ActionState {
				image: "opendeck/toggle-action.png".to_owned(),
				..Default::default()
			});
			let _ = update_state(&app, parent.context.clone(), &mut locks).await;
		}

		save_profile_now(&context.device, &mut locks).await?;
		drop(locks);
		let _ = crate::events::outbound::will_appear::will_appear(&instance).await;

		let locks = acquire_locks().await;
		let slot = get_slot(&context, &locks).await?.clone();
		Ok(slot)
	} else {
		let instance = ActionInstance {
			action: action.clone(),
			context: ActionContext::from_context(context.clone(), 0),
			states: action.states.clone(),
			current_state: 0,
			settings: serde_json::Value::Object(serde_json::Map::new()),
			children: if matches!(action.uuid.as_str(), "opendeck.multiaction" | "opendeck.toggleaction") {
				Some(vec![])
			} else {
				None
			},
		};

		*slot = Some(instance.clone());
		let slot = slot.clone();

		save_profile_now(&context.device, &mut locks).await?;
		let _ = crate::events::outbound::will_appear::will_appear(&instance).await;

		Ok(slot)
	}
}

fn instance_images_dir(context: &ActionContext) -> std::path::PathBuf {
	config_dir()
		.join("images")
		.join(&context.device)
		.join(&context.profile)
		.join(format!("{}.{}.{}", context.controller, context.position, context.index))
}

#[command]
pub async fn move_instance(source: Context, destination: Context, retain: bool) -> Result<Option<ActionInstance>, Error> {
	if source.controller != destination.controller {
		return Ok(None);
	}

	{
		let locks = crate::store::profiles::acquire_locks().await;
		let dst = crate::store::profiles::get_slot(&destination, &locks).await?;
		if dst.is_some() {
			return Ok(None);
		}
	}

	let mut locks = acquire_locks_mut().await;
	let src = get_slot_mut(&source, &mut locks).await?;

	let Some(mut new) = src.clone() else {
		return Ok(None);
	};
	new.context = ActionContext::from_context(destination.clone(), 0);
	if let Some(children) = &mut new.children {
		for (index, instance) in children.iter_mut().enumerate() {
			instance.context = ActionContext::from_context(destination.clone(), index as u16 + 1);
			for (i, state) in instance.states.iter_mut().enumerate() {
				if !instance.action.states[i].image.is_empty() {
					state.image = instance.action.states[i].image.clone();
				} else {
					state.image = instance.action.icon.clone();
				}
			}
		}
	}

	let old_dir = instance_images_dir(&src.as_ref().unwrap().context);
	let new_dir = instance_images_dir(&new.context);
	let _ = tokio::fs::create_dir_all(&new_dir).await;
	if let Ok(files) = old_dir.read_dir() {
		for file in files.flatten() {
			let _ = tokio::fs::copy(file.path(), new_dir.join(file.file_name())).await;
		}
	}
	for state in new.states.iter_mut() {
		let path = std::path::Path::new(&state.image);
		if path.starts_with(&old_dir) {
			state.image = new_dir.join(path.strip_prefix(&old_dir).unwrap()).to_string_lossy().into_owned();
		}

		if let Some(image_override) = state.image_override.as_mut() {
			let override_path = std::path::Path::new(image_override);
			if override_path.starts_with(&old_dir) {
				*image_override = new_dir.join(override_path.strip_prefix(&old_dir).unwrap()).to_string_lossy().into_owned();
			}
		}
	}

	let dst = get_slot_mut(&destination, &mut locks).await?;
	*dst = Some(new.clone());

	if !retain {
		let src = get_slot_mut(&source, &mut locks).await?;
		if let Some(old) = src {
			let _ = crate::events::outbound::will_appear::will_disappear(old, true).await;
			let _ = remove_dir_all(instance_images_dir(&old.context)).await;
		}
		*src = None;
	}

	let _ = crate::events::outbound::will_appear::will_appear(&new).await;

	save_profile_now(&destination.device, &mut locks).await?;

	Ok(Some(new))
}

#[command]
pub async fn remove_instance(context: ActionContext) -> Result<(), Error> {
	let mut locks = acquire_locks_mut().await;
	let slot = get_slot_mut(&(&context).into(), &mut locks).await?;
	let Some(instance) = slot else {
		return Ok(());
	};

	if instance.context == context {
		let _ = crate::events::outbound::will_appear::will_disappear(instance, true).await;
		if let Some(children) = &instance.children {
			for child in children {
				let _ = crate::events::outbound::will_appear::will_disappear(child, true).await;
				let _ = remove_dir_all(instance_images_dir(&child.context)).await;
			}
		}
		let _ = remove_dir_all(instance_images_dir(&instance.context)).await;
		*slot = None;
	} else {
		let children = instance.children.as_mut().unwrap();
		for (index, child) in children.iter().enumerate() {
			if child.context == context {
				let _ = crate::events::outbound::will_appear::will_disappear(child, true).await;
				let _ = remove_dir_all(instance_images_dir(&child.context)).await;
				children.remove(index);

				if instance.action.uuid == "opendeck.multiaction"
					&& let Some(settings) = instance.settings.as_object_mut()
					&& let Some(delays) = settings.get_mut("delays").and_then(|v| v.as_array_mut())
				{
					if index == 0 {
						if !delays.is_empty() {
							delays.remove(0);
						}
					} else if index - 1 < delays.len() {
						delays.remove(index - 1);
					}
				}

				break;
			}
		}
		if instance.action.uuid == "opendeck.toggleaction" {
			if instance.current_state as usize >= children.len() {
				instance.current_state = if children.is_empty() { 0 } else { children.len() as u16 - 1 };
			}
			if !children.is_empty() {
				instance.states.pop();
				let _ = update_state(crate::APP_HANDLE.get().unwrap(), instance.context.clone(), &mut locks).await;
			}
		}
	}

	save_profile_now(&context.device, &mut locks).await?;

	Ok(())
}

#[derive(Clone, serde::Serialize)]
struct UpdateStateEvent {
	context: ActionContext,
	contents: Option<ActionInstance>,
}

pub async fn update_state(app: &AppHandle, context: ActionContext, locks: &mut LocksMut<'_>) -> Result<(), anyhow::Error> {
	let contents = get_instance_mut(&context, locks).await?.cloned();

	// FORK DEVIATION 7: a key whose image is a plugin-rendered JPEG with nothing
	// to draw over it goes to the device from here, without waiting for the
	// frontend to render it -- so keys keep updating whatever the WebView is
	// doing. The frontend still sends the same bytes a moment later (deviation
	// 6); outbound::devices::update_image drops that duplicate.
	if let Some(instance) = &contents {
		// Dial strips too: the frontend used to render the encoder's image and call
		// update_image, which composes the strip from the layout and feedback in
		// Rust. Ask for that directly, so strips update without a WebView
		// (deviation 8). The fallback image is only drawn for an encoder without a
		// layout, which is not what this path is for.
		// Only without a window: with one, the frontend still does it, and the
		// strips update every second or so -- no need to render and send twice.
		if instance.context.controller == "Encoder" && instance.action.encoder.is_some() && app.get_webview_window("main").is_none() {
			let device_context: crate::shared::Context = (&instance.context).into();
			tokio::spawn(async move {
				let selected = crate::store::profiles::DEVICE_STORES.write().await.get_selected_profile(&device_context.device).ok();
				if selected.as_ref() != Some(&device_context.profile) {
					return;
				}
				if let Err(error) = crate::events::outbound::devices::update_image(device_context, Some(BLACK_JPEG.to_owned())).await {
					log::warn!("Failed to update encoder image: {}", error);
				}
			});
		}
		if let Some(image) = passthrough_image(instance) {
			let device_context: crate::shared::Context = (&instance.context).into();
			tokio::spawn(async move {
				let selected = crate::store::profiles::DEVICE_STORES.write().await.get_selected_profile(&device_context.device).ok();
				if selected.as_ref() != Some(&device_context.profile) {
					return;
				}
				if let Err(error) = crate::events::outbound::devices::update_image(device_context, Some(image)).await {
					log::warn!("Failed to update device image: {}", error);
				}
			});
		}
	}

	// FORK DEVIATION 8: no window while OpenDeck is in the tray; the device was
	// already updated above (deviation 7).
	if let Some(window) = app.get_webview_window("main") {
		window.emit("update_state", UpdateStateEvent { contents, context })?;
	}
	Ok(())
}

/// FORK DEVIATION 7: a 1x1 black JPEG, the fallback for a strip rendered from the
/// backend (only drawn when an encoder has no layout).
const BLACK_JPEG: &str = "data:image/jpeg;base64,/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAMCAgMCAgMDAwMEAwMEBQgFBQQEBQoHBwYIDAoMDAsKCwsNDhIQDQ4RDgsLEBYQERMUFRUVDA8XGBYUGBIUFRT/2wBDAQMEBAUEBQkFBQkUDQsNFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBT/wAARCAABAAEDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwD8qqKKKAP/2Q==";

/// FORK DEVIATION 7: the same test as the frontend's pass-through (deviation 6),
/// minus what only the frontend knows (press effect, ok/alert overlays): a key
/// state whose image is a JPEG data URL with no title, scaling, background or
/// device rotation applied.
fn passthrough_image(instance: &ActionInstance) -> Option<String> {
	if instance.context.controller != "Keypad" || crate::store::get_settings().value.rotation != 0 {
		return None;
	}
	let state = instance.states.get(instance.current_state as usize)?;
	let image = state.image_override.as_ref().unwrap_or(&state.image);
	let has_title = state.show && (state.text_override.as_deref().is_some_and(|t| !t.is_empty()) || !state.text.is_empty());
	let scaled = state.image_scale != 0 && state.image_scale != 100;
	if !image.starts_with("data:image/jpeg;base64,") || has_title || scaled || !state.background_colour.starts_with("#000000") {
		return None;
	}
	Some(image.clone())
}

#[command]
pub async fn set_state(context: ActionContext, index: u16, state: ActionState) -> Result<(), Error> {
	let mut locks = acquire_locks_mut().await;
	let reference = get_instance_mut(&context, &mut locks).await?.unwrap();
	reference.states[index as usize] = state;
	let clone = reference.clone();
	save_profile_now(&context.device, &mut locks).await?;
	crate::events::outbound::states::title_parameters_did_change(&clone, index).await?;
	Ok(())
}

#[command]
pub async fn set_child_delay(parent_context: ActionContext, index: usize, delay_ms: u64) -> Result<serde_json::Value, Error> {
	let mut locks = acquire_locks_mut().await;
	let Some(parent) = get_instance_mut(&parent_context, &mut locks).await? else {
		return Ok(serde_json::Value::Null);
	};

	let delays = parent.settings.get_mut("delays").and_then(|v| v.as_array_mut());
	if let Some(arr) = delays {
		if arr.len() <= index {
			arr.resize(index + 1, serde_json::json!(100));
		}
		arr[index] = serde_json::json!(delay_ms);
	} else {
		if !parent.settings.is_object() {
			parent.settings = serde_json::Value::Object(serde_json::Map::new());
		}
		let map = parent.settings.as_object_mut().unwrap();
		let mut arr = vec![serde_json::json!(100); index + 1];
		arr[index] = serde_json::json!(delay_ms);
		map.insert("delays".to_string(), serde_json::Value::Array(arr));
	}
	let parent_settings = parent.settings.clone();

	save_profile_now(&parent_context.device, &mut locks).await?;
	Ok(parent_settings)
}

#[command]
pub async fn update_image(context: Context, image: Option<String>) {
	if Some(&context.profile) != crate::store::profiles::DEVICE_STORES.write().await.get_selected_profile(&context.device).ok().as_ref() {
		return;
	}

	if let Err(error) = crate::events::outbound::devices::update_image(context, image).await {
		log::warn!("Failed to update device image: {}", error);
	}
}

#[command]
pub async fn trigger_virtual_press(context: Context) -> Result<(), Error> {
	let event = || crate::events::inbound::PayloadEvent {
		payload: crate::events::inbound::devices::PressPayload {
			device: context.device.clone(),
			position: context.position,
		},
	};
	match context.controller.as_str() {
		"Keypad" => {
			crate::events::inbound::devices::key_down(event()).await?;
			tokio::time::sleep(std::time::Duration::from_millis(100)).await;
			crate::events::inbound::devices::key_up(event()).await?;
		}
		"Encoder" => {
			crate::events::inbound::devices::encoder_down(event()).await?;
			tokio::time::sleep(std::time::Duration::from_millis(100)).await;
			crate::events::inbound::devices::encoder_up(event()).await?;
		}
		_ => {}
	}

	Ok(())
}

#[derive(Clone, serde::Serialize)]
struct KeyMovedEvent {
	context: Context,
	pressed: bool,
}

pub async fn key_moved(app: &AppHandle, context: Context, pressed: bool) -> Result<(), anyhow::Error> {
	// FORK DEVIATION 8: editor feedback only; nothing to do without a window.
	if let Some(window) = app.get_webview_window("main") {
		window.emit("key_moved", KeyMovedEvent { context, pressed })?;
	}
	Ok(())
}
