use super::PayloadEvent;

use crate::plugins::DEVICE_NAMESPACES;
use crate::shared::DEVICES;
use crate::store::profiles::get_device_profiles;

use serde::Deserialize;

pub async fn register_device(uuid: &str, mut event: PayloadEvent<crate::shared::DeviceInfo>) -> Result<(), anyhow::Error> {
	if uuid.is_empty() || Some(uuid) == DEVICE_NAMESPACES.read().await.get(&event.payload.id[..2]).map(|x| x.as_str()) {
		if let Ok(profiles) = get_device_profiles(&event.payload.id) {
			let mut profile_stores = crate::store::profiles::PROFILE_STORES.write().await;
			for profile in profiles {
				// This is called to initialise the store for each profile when the device is registered.
				if let Err(e) = profile_stores.get_profile_store_mut(&event.payload, &profile).await {
					log::error!("{}", e);
				}
			}
		}

		event.payload.plugin = uuid.to_owned();
		let _ = crate::events::outbound::devices::device_did_connect(&event.payload.id, (&event.payload).into()).await;
		DEVICES.insert(event.payload.id.clone(), event.payload.clone());
		let _ = crate::events::outbound::devices::set_device_brightness(&event.payload.id, crate::store::get_settings().value.brightness).await;
		let _ = crate::device_sleep::apply_initial_device_sleep(&event.payload.id).await;
		crate::events::frontend::update_devices().await;

		let mut locks = crate::store::profiles::acquire_locks_mut().await;
		let selected_profile = locks.device_stores.get_selected_profile(&event.payload.id)?;
		let profile = locks.profile_stores.get_profile_store(&DEVICES.get(&event.payload.id).unwrap(), &selected_profile)?;
		for instance in profile
			.value
			.keys
			.iter()
			.flatten()
			.chain(profile.value.sliders.iter().flatten())
			.chain(profile.value.infobars.iter().flatten())
		{
			let _ = crate::events::outbound::will_appear::will_appear(instance).await;
		}
		drop(locks);

		// FORK DEVIATION 8: without a window, wake and redraw the panel once it
		// has settled. Measured on the MSD-Pro (Linux, 2026-11-16): after the
		// shutdown command akp05 sends when OpenDeck stops, the panel ignores the
		// brightness sent at registration above and stays dark although every
		// image reaches it. The window's frontend used to cover this by sending
		// the brightness again when it loaded. Strips drawn while it was dark
		// stay blank until they next change, so the selected profile is drawn
		// again too (deviation 7's backend path). After 3 s: the outbound
		// duplicate filter drops an identical key image within 2 s. The window
		// test comes after the wait: the window destroyed at a --hide start still
		// counts as present for a moment, and the device registers inside it.
		{
			let device = event.payload.id.clone();
			tokio::spawn(async move {
				use tauri::Manager;
				tokio::time::sleep(std::time::Duration::from_secs(3)).await;
				let Some(app) = crate::APP_HANDLE.get() else { return };
				if app.get_webview_window("main").is_some() || !DEVICES.contains_key(&device) {
					return;
				}
				log::info!("No window: waking {device} and redrawing its profile from the backend");
				let _ = crate::events::outbound::devices::set_device_brightness(&device, crate::store::get_settings().value.brightness).await;
				let mut locks = crate::store::profiles::acquire_locks_mut().await;
				let Ok(selected) = locks.device_stores.get_selected_profile(&device) else { return };
				let Some(info) = DEVICES.get(&device).map(|d| d.clone()) else { return };
				let Ok(profile) = locks.profile_stores.get_profile_store(&info, &selected) else { return };
				let contexts: Vec<_> = profile
					.value
					.keys
					.iter()
					.flatten()
					.chain(profile.value.sliders.iter().flatten())
					.chain(profile.value.infobars.iter().flatten())
					.map(|instance| instance.context.clone())
					.collect();
				for context in contexts {
					let _ = crate::events::frontend::instances::update_state(app, context, &mut locks).await;
				}
			});
		}

		use tauri_plugin_aptabase::EventTracker;
		let _ = crate::APP_HANDLE
			.get()
			.unwrap()
			.track_event("device_registered", Some(serde_json::json!({ "name": event.payload.name })));

		Ok(())
	} else {
		Err(anyhow::anyhow!("plugin {uuid} is not registered for device namespace {}", &event.payload.id[..2]))
	}
}

pub async fn deregister_device(uuid: &str, event: PayloadEvent<String>) -> Result<(), anyhow::Error> {
	if uuid.is_empty() || Some(uuid) == DEVICE_NAMESPACES.read().await.get(&event.payload[..2]).map(|x| x.as_str()) {
		if !DEVICES.contains_key(&event.payload) {
			return Ok(());
		}

		let mut locks = crate::store::profiles::acquire_locks_mut().await;

		let selected_profile = locks.device_stores.get_selected_profile(&event.payload)?;
		let profile = locks.profile_stores.get_profile_store(&DEVICES.get(&event.payload).unwrap(), &selected_profile)?;
		for instance in profile
			.value
			.keys
			.iter()
			.flatten()
			.chain(profile.value.sliders.iter().flatten())
			.chain(profile.value.infobars.iter().flatten())
		{
			let _ = crate::events::outbound::will_appear::will_disappear(instance, false).await;
		}

		// Flush any pending profile writes before removing the device.
		if let Err(error) = crate::store::profiles::save_profile_now(&event.payload, &mut locks).await {
			log::error!("Failed to flush profile for device {}: {error}", event.payload);
		}

		if let Ok(profiles) = get_device_profiles(&event.payload) {
			for profile in profiles {
				locks.profile_stores.remove_profile(&event.payload, &profile);
			}
		}

		drop(locks);

		let _ = crate::events::outbound::devices::device_did_disconnect(&event.payload).await;
		DEVICES.remove(&event.payload);
		crate::device_sleep::deregister_device(&event.payload);
		crate::events::frontend::update_devices().await;

		Ok(())
	} else {
		Err(anyhow::anyhow!("plugin {uuid} is not registered for device namespace {}", &event.payload[..2]))
	}
}

#[derive(Deserialize)]
pub struct PressPayload {
	pub device: String,
	pub position: u8,
}

pub async fn key_down(event: PayloadEvent<PressPayload>) -> Result<(), anyhow::Error> {
	if crate::device_sleep::note_activity(&event.payload.device).await.unwrap_or(false) {
		return Ok(());
	}
	crate::events::outbound::keypad::key_down(&event.payload.device, event.payload.position).await
}

pub async fn key_up(event: PayloadEvent<PressPayload>) -> Result<(), anyhow::Error> {
	if crate::device_sleep::note_activity(&event.payload.device).await.unwrap_or(false) {
		return Ok(());
	}
	crate::events::outbound::keypad::key_up(&event.payload.device, event.payload.position).await
}

#[derive(Deserialize)]
pub struct TicksPayload {
	pub device: String,
	pub position: u8,
	pub ticks: i16,
}

pub async fn encoder_change(event: PayloadEvent<TicksPayload>) -> Result<(), anyhow::Error> {
	if crate::device_sleep::note_activity(&event.payload.device).await.unwrap_or(false) {
		return Ok(());
	}
	crate::events::outbound::encoder::dial_rotate(&event.payload.device, event.payload.position, event.payload.ticks).await
}

pub async fn encoder_down(event: PayloadEvent<PressPayload>) -> Result<(), anyhow::Error> {
	if crate::device_sleep::note_activity(&event.payload.device).await.unwrap_or(false) {
		return Ok(());
	}
	crate::events::outbound::encoder::dial_press(&event.payload.device, "dialDown", event.payload.position).await
}

pub async fn encoder_up(event: PayloadEvent<PressPayload>) -> Result<(), anyhow::Error> {
	if crate::device_sleep::note_activity(&event.payload.device).await.unwrap_or(false) {
		return Ok(());
	}
	crate::events::outbound::encoder::dial_press(&event.payload.device, "dialUp", event.payload.position).await
}

#[derive(Deserialize)]
pub struct TouchscreenPressPayload {
	pub device: String,
	pub position: u8,
	pub x: u16,
	pub y: u16,
	#[serde(default)]
	pub hold: bool,
}

pub async fn touchscreen_press(event: PayloadEvent<TouchscreenPressPayload>) -> Result<(), anyhow::Error> {
	if crate::device_sleep::note_activity(&event.payload.device).await.unwrap_or(false) {
		return Ok(());
	}
	crate::events::outbound::encoder::touch_tap(&event.payload.device, event.payload.position, event.payload.x, event.payload.y, event.payload.hold).await
}

pub async fn rerender_images(_event: PayloadEvent<String>) -> Result<(), anyhow::Error> {
	crate::events::frontend::profiles::rerender_images(crate::APP_HANDLE.get().unwrap()).await?;
	Ok(())
}
