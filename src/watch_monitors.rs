use std::{fs::File, io::BufWriter, thread, time::Duration};

use chrono::{Local, Utc};
use clap::Args;
use color_eyre::eyre::{Context, Result};
use libwayshot::WayshotConnection;

use crate::config::LiveSettings;

#[derive(Args, Debug)]
pub struct WatchMonitorsArgs {}

fn save_screenshot_png(image_buffer: &image::DynamicImage, path: &std::path::Path) -> Result<()> {
	let rgba = image_buffer.to_rgba8();
	let file = File::create(path).wrap_err(format!("Failed to create file: {}", path.display()))?;
	let writer = BufWriter::new(file);

	let mut encoder = png::Encoder::new(writer, rgba.width(), rgba.height());
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);

	let mut writer = encoder.write_header().wrap_err("Failed to write PNG header")?;
	writer.write_image_data(rgba.as_raw()).wrap_err("Failed to write PNG data")?;

	Ok(())
}

fn cleanup_old_screenshots(cache_dir: &std::path::Path) -> Result<()> {
	let now = Utc::now();
	let threshold = now - chrono::Duration::days(1);

	for entry in std::fs::read_dir(cache_dir)? {
		let entry = entry?;
		let path = entry.path();

		if path.is_dir() {
			// Try to parse directory name as date (YYYY-MM-DD format)
			if let Some(dir_name) = path.file_name().and_then(|n| n.to_str())
				&& let Ok(dir_date) = chrono::NaiveDate::parse_from_str(dir_name, "%Y-%m-%d")
			{
				let dir_datetime = dir_date.and_hms_opt(0, 0, 0).unwrap();
				let dir_datetime_utc = chrono::DateTime::<Utc>::from_naive_utc_and_offset(dir_datetime, Utc);

				if dir_datetime_utc < threshold {
					tracing::info!("Removing old screenshot directory: {}", path.display());
					std::fs::remove_dir_all(&path)?;
				}
			}
		}
	}

	Ok(())
}

pub fn main(_settings: &LiveSettings, _args: WatchMonitorsArgs) -> Result<()> {
	let cache_dir = v_utils::xdg_cache_dir!("watch_monitors");

	tracing::info!("Starting monitor watch daemon. Taking screenshots every 60 seconds.");

	loop {
		let now = Local::now();
		let date_dir = cache_dir.join(now.format("%Y-%m-%d").to_string());

		// Create the date directory if it doesn't exist
		std::fs::create_dir_all(&date_dir).wrap_err(format!("Failed to create directory: {}", date_dir.display()))?;

		// Create Wayshot connection
		let wayshot = match WayshotConnection::new() {
			Ok(w) => w,
			Err(e) => {
				tracing::error!("Failed to connect to Wayland compositor: {:?}", e);
				thread::sleep(Duration::from_secs(60));
				continue;
			}
		};

		// Get list of outputs
		let outputs = wayshot.get_all_outputs();

		if outputs.is_empty() {
			tracing::warn!("No outputs found");
			thread::sleep(Duration::from_secs(60));
			continue;
		}

		let timestamp = now.format("%H-%M-%S").to_string();

		// Capture all outputs
		for (i, output) in outputs.iter().enumerate() {
			let filename = format!("{timestamp}-s{i}.png");
			let screenshot_path = date_dir.join(filename);

			match wayshot.screenshot_single_output(output, false) {
				Ok(image_buffer) =>
					if let Err(e) = save_screenshot_png(&image_buffer, &screenshot_path) {
						tracing::error!("Failed to save screenshot to {}: {:?}", screenshot_path.display(), e);
					} else {
						tracing::debug!("Screenshot saved to: {}", screenshot_path.display());
					},
				Err(e) => {
					tracing::error!("Failed to capture screenshot from output {i}: {:?}", e);
				}
			}
		}

		// Cleanup old screenshots (run once per loop iteration)
		if let Err(e) = cleanup_old_screenshots(&cache_dir) {
			tracing::error!("Failed to cleanup old screenshots: {:?}", e);
		}

		// Sleep for 60 seconds
		thread::sleep(Duration::from_secs(60));
	}
}
