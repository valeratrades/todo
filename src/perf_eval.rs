use std::{fs::File, io::BufWriter};

use chrono::Local;
use clap::Args;
use color_eyre::eyre::{Context, Result};
use libwayshot::WayshotConnection;

use crate::config::{AppConfig, CACHE_DIR};

#[derive(Args, Debug)]
pub struct PerfEvalArgs {}

fn save_screenshot_png(image_buffer: &image::RgbaImage, path: &std::path::Path) -> Result<()> {
	let file = File::create(path).wrap_err(format!("Failed to create file: {}", path.display()))?;
	let writer = BufWriter::new(file);

	let mut encoder = png::Encoder::new(writer, image_buffer.width(), image_buffer.height());
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);

	let mut writer = encoder.write_header().wrap_err("Failed to write PNG header")?;
	writer.write_image_data(image_buffer.as_raw()).wrap_err("Failed to write PNG data")?;

	Ok(())
}

pub fn main(_config: AppConfig, _args: PerfEvalArgs) -> Result<()> {
	let cache_dir = CACHE_DIR.get().ok_or_else(|| color_eyre::eyre::eyre!("CACHE_DIR not initialized"))?;

	let now = Local::now();
	let date_dir = cache_dir.join(now.format("%Y-%m-%d").to_string());

	// Create the date directory if it doesn't exist
	std::fs::create_dir_all(&date_dir).wrap_err(format!("Failed to create directory: {}", date_dir.display()))?;

	// Create Wayshot connection
	let wayshot = WayshotConnection::new().wrap_err("Failed to connect to Wayland compositor. Are you running a wlroots-based compositor (Sway, Hyprland, etc.)?")?;

	// Get list of outputs
	let outputs = wayshot.get_all_outputs();

	if outputs.is_empty() {
		return Err(color_eyre::eyre::eyre!("No outputs found"));
	}

	let timestamp = now.format("%H-%M-%S").to_string();

	// Capture all outputs
	for (i, output) in outputs.iter().enumerate() {
		let filename = format!("{timestamp}-s{i}.png");
		let screenshot_path = date_dir.join(filename);

		let image_buffer = wayshot
			.screenshot_single_output(output, false)
			.map_err(|e| color_eyre::eyre::eyre!("Failed to capture screenshot from output {i}: {e:?}"))?;

		save_screenshot_png(&image_buffer, &screenshot_path)?;

		println!("Screenshot saved to: {}", screenshot_path.display());
	}

	Ok(())
}
