use std::{fs::File, io::BufWriter};

use ask_llm::{ImageContent, Message, Model, Role};
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

pub async fn main(_config: AppConfig, _args: PerfEvalArgs) -> Result<()> {
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
	let mut screenshot_images = Vec::new();

	// Capture all outputs
	for (i, output) in outputs.iter().enumerate() {
		let filename = format!("{timestamp}-s{i}.png");
		let screenshot_path = date_dir.join(filename);

		let image_buffer = wayshot
			.screenshot_single_output(output, false)
			.map_err(|e| color_eyre::eyre::eyre!("Failed to capture screenshot from output {i}: {e:?}"))?;

		save_screenshot_png(&image_buffer, &screenshot_path)?;

		// Convert to base64 for LLM
		let png_bytes = std::fs::read(&screenshot_path).wrap_err("Failed to read saved screenshot")?;
		let base64_data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_bytes);

		screenshot_images.push(ImageContent {
			base64_data,
			media_type: "image/png".to_string(),
		});

		tracing::debug!("Screenshot saved to: {}", screenshot_path.display());
	}

	// Analyze all screenshots with LLM
	println!("\nAnalyzing screenshots...");
	let prompt = "Describe what the user is doing on each screen in 1-3 sentences. Be concise and focus on the main activity.";

	let message = Message::new_with_text_and_images(Role::User, prompt.to_string(), screenshot_images);

	let mut conv = ask_llm::Conversation::new();
	conv.0.push(message);

	match ask_llm::conversation(&conv, Model::Medium, Some(4096), None).await {
		Ok(response) => {
			if response.text.is_empty() {
				eprintln!("Warning: Got empty response from LLM");
			} else {
				println!("\n{}", response.text);
			}
			tracing::info!("Cost: {:.4} cents", response.cost_cents);
		}
		Err(e) => {
			eprintln!("Error calling LLM: {:?}", e);
			return Err(e);
		}
	}

	Ok(())
}
