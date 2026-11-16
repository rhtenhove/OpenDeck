use super::{ContextEvent, PayloadEvent};

use tauri::{Emitter, Manager};

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct OpenUrlEvent {
	pub url: String,
}

pub async fn open_url(event: PayloadEvent<OpenUrlEvent>) -> Result<(), anyhow::Error> {
	log::debug!("Opening URL {}", event.payload.url);
	open::that_detached(event.payload.url)?;
	Ok(())
}

#[derive(Deserialize)]
pub struct LogMessageEvent {
	pub message: String,
}

pub async fn log_message(uuid: Option<&str>, mut event: PayloadEvent<LogMessageEvent>) -> Result<(), anyhow::Error> {
	if let Some(uuid) = uuid
		&& let Ok(manifest) = crate::plugins::manifest::read_manifest(&crate::shared::config_dir().join("plugins").join(uuid))
	{
		event.payload.message = format!("[{}] {}", manifest.name, event.payload.message);
	}
	log::info!("{}", event.payload.message.trim());
	Ok(())
}

pub async fn show_alert(event: ContextEvent) -> Result<(), anyhow::Error> {
	let app = crate::APP_HANDLE.get().unwrap();
	// FORK DEVIATION 8: overlays are drawn by the frontend; skipped in the tray.
	if let Some(window) = app.get_webview_window("main") {
		window.emit("show_alert", event.context)?;
	}
	Ok(())
}

pub async fn show_ok(event: ContextEvent) -> Result<(), anyhow::Error> {
	let app = crate::APP_HANDLE.get().unwrap();
	if let Some(window) = app.get_webview_window("main") {
		window.emit("show_ok", event.context)?;
	}
	Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SwitchProfileEvent {
	device: String,
	profile: String,
}

pub async fn switch_profile(event: SwitchProfileEvent) -> Result<(), anyhow::Error> {
	let app_handle = crate::APP_HANDLE.get().unwrap();
	// FORK DEVIATION 8: with the window open the frontend switches (as upstream);
	// without one, switch here -- it is what the frontend would have invoked.
	match app_handle.get_webview_window("main") {
		Some(window) => window.emit("switch_profile", event)?,
		None => crate::events::frontend::profiles::set_selected_profile(event.device, event.profile).await?,
	}
	Ok(())
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DeviceBrightnessEvent {
	action: String,
	value: u8,
}

pub async fn device_brightness(event: DeviceBrightnessEvent) -> Result<(), anyhow::Error> {
	let app_handle = crate::APP_HANDLE.get().unwrap();
	// FORK DEVIATION 8: brightness requests go through the settings page; not
	// applied while OpenDeck is in the tray.
	if let Some(window) = app_handle.get_webview_window("main") {
		window.emit("device_brightness", event)?;
	} else {
		log::info!("Ignoring a device brightness request while the window is closed");
	}
	Ok(())
}
