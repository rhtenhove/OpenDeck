use super::{send_to_all_plugins, send_to_plugin};

use crate::encoder_layouts::generate_encoder_image;
use crate::plugins::{DEVICE_NAMESPACES, info_param::DeviceInfo};

use base64::Engine;
use serde::Serialize;
use std::io::Cursor;

#[derive(Serialize)]
#[allow(non_snake_case)]
struct DeviceDidConnectEvent {
	event: &'static str,
	device: String,
	deviceInfo: DeviceInfo,
}

pub async fn device_did_connect(id: &str, info: DeviceInfo) -> Result<(), anyhow::Error> {
	send_to_all_plugins(&DeviceDidConnectEvent {
		event: "deviceDidConnect",
		device: id.to_owned(),
		deviceInfo: info,
	})
	.await
}

#[derive(Serialize)]
struct DeviceDidDisconnectEvent {
	event: &'static str,
	device: String,
}

pub async fn device_did_disconnect(id: &str) -> Result<(), anyhow::Error> {
	send_to_all_plugins(&DeviceDidDisconnectEvent {
		event: "deviceDidDisconnect",
		device: id.to_owned(),
	})
	.await
}

#[derive(Serialize)]
struct SetImageEvent {
	event: &'static str,
	device: String,
	controller: Option<String>,
	position: Option<u8>,
	image: Option<String>,
}

/// FORK DEVIATION 7: the last image sent to each key, to drop the frontend's
/// copy of an image the backend already forwarded. Only within a short window,
/// so a deliberate resend later (reconnect, wake) always goes through.
static RECENT: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, (u64, std::time::Instant)>>> =
	std::sync::LazyLock::new(Default::default);
const DUPLICATE_WINDOW: std::time::Duration = std::time::Duration::from_secs(2);

fn is_recent_duplicate(context: &crate::shared::Context, image: &Option<String>) -> bool {
	use std::hash::{Hash, Hasher};
	let key = format!("{}.{}.{}.{}", context.device, context.profile, context.controller, context.position);
	let mut recent = RECENT.lock().unwrap();
	let Some(image) = image else {
		recent.remove(&key);
		return false;
	};
	let mut h = std::collections::hash_map::DefaultHasher::new();
	image.hash(&mut h);
	let hash = h.finish();
	let now = std::time::Instant::now();
	if let Some((last, at)) = recent.get(&key)
		&& *last == hash
		&& now.duration_since(*at) < DUPLICATE_WINDOW
	{
		return true;
	}
	recent.insert(key, (hash, now));
	false
}

pub async fn update_image(context: crate::shared::Context, image: Option<String>) -> Result<(), anyhow::Error> {
	if context.controller == "Keypad" && is_recent_duplicate(&context, &image) {
		return Ok(());
	}
	if let Some(plugin) = DEVICE_NAMESPACES.read().await.get(&context.device[..2]) {
		let image = match (context.controller.as_str(), image) {
			("Encoder", Some(img)) => Some(to_encoder_jpeg_data_uri(&context, &img).await?),
			(_, img) => img,
		};

		send_to_plugin(
			plugin,
			&SetImageEvent {
				event: "setImage",
				device: context.device,
				controller: Some(context.controller),
				position: Some(context.position),
				image,
			},
		)
		.await?;
	} else if context.device.starts_with("sd-") {
		crate::elgato::update_image(&context, image.as_deref()).await?;
	}

	Ok(())
}

async fn to_encoder_jpeg_data_uri(context: &crate::shared::Context, image: &str) -> Result<String, anyhow::Error> {
	let data = image.split_once(',').unwrap().1;
	let bytes = base64::engine::general_purpose::STANDARD.decode(data)?;

	let img = generate_encoder_image(context, &bytes).await?;

	// FORK DEVIATION: encode at maximum quality instead of the image crate's
	// default (75). The device plugin only accepts JPEG, and mirajazz re-encodes
	// at quality 90 on the way to the panel, so the strip was being compressed
	// twice, the first time badly -- very visible as ringing around numerals on
	// a 200x100 canvas. Quality 100 makes the first pass near-lossless and
	// leaves mirajazz's 90 as the only real loss.
	let mut buf = Vec::new();
	{
		use image::codecs::jpeg::JpegEncoder;
		// Cursor must outlive the encoder that borrows it.
		let mut cursor = Cursor::new(&mut buf);
		let mut encoder = JpegEncoder::new_with_quality(&mut cursor, 100);
		encoder.encode_image(&img)?;
	}
	let encoded = base64::engine::general_purpose::STANDARD.encode(&buf);

	Ok(format!("data:image/jpeg;base64,{encoded}"))
}

pub async fn clear_screen(device: String) -> Result<(), anyhow::Error> {
	if let Some(plugin) = DEVICE_NAMESPACES.read().await.get(&device[..2]) {
		send_to_plugin(
			plugin,
			&SetImageEvent {
				event: "setImage",
				device,
				controller: None,
				position: None,
				image: None,
			},
		)
		.await?;
	} else if device.starts_with("sd-") {
		crate::elgato::clear_screen(&device).await?;
	}

	Ok(())
}

#[derive(Serialize)]
struct SetBrightnessEvent {
	event: &'static str,
	device: String,
	brightness: u8,
}

/// Set the brightness for all devices.
pub async fn set_brightness(brightness: u8) -> Result<(), anyhow::Error> {
	for device in crate::shared::DEVICES.iter() {
		set_device_brightness(&device.id, brightness).await?;
	}

	Ok(())
}

/// Set the brightness for a specific device.
pub async fn set_device_brightness(device: &str, brightness: u8) -> Result<(), anyhow::Error> {
	if crate::device_sleep::is_device_sleeping(device) {
		return Ok(());
	}

	if let Some(plugin) = DEVICE_NAMESPACES.read().await.get(&device[..2]) {
		send_to_plugin(
			plugin,
			&SetBrightnessEvent {
				event: "setBrightness",
				device: device.to_owned(),
				brightness,
			},
		)
		.await?;
	} else if device.starts_with("sd-") {
		crate::elgato::set_brightness(device, brightness).await;
	}

	Ok(())
}
