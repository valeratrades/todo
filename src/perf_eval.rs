use std::{fs::File, io::BufWriter, process::Command};

use ask_llm::{ImageContent, Message, Model, Role};
use chrono::Local;
use clap::Args;
use color_eyre::eyre::{Context, Result};
use libwayshot::WayshotConnection;

use crate::config::{AppConfig, CACHE_DIR};

#[derive(Args, Debug)]
pub struct PerfEvalArgs {}

fn save_screenshot_png(image_buffer: &image24::RgbaImage, path: &std::path::Path) -> Result<()> {
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

	// Get current blocker
	let blocker_output = Command::new(std::env::current_exe()?)
		.args(["blocker", "current", "-f"])
		.output()
		.wrap_err("Failed to execute blocker current")?;

	let current_blocker = String::from_utf8_lossy(&blocker_output.stdout).trim().to_string();

	if current_blocker.is_empty() {
		return Err(color_eyre::eyre::eyre!("No current blocker found. Set one with: todo blocker add <task>"));
	}

	// Get daily milestones
	let milestones_output = Command::new(std::env::current_exe()?)
		.args(["milestones", "get", "1d"])
		.output()
		.wrap_err("Failed to execute milestones get 1d")?;

	let daily_milestones = String::from_utf8_lossy(&milestones_output.stdout).trim().to_string();

	// Get static milestones
	let static_milestones_output = Command::new(std::env::current_exe()?)
		.args(["milestones", "get", "static"])
		.output()
		.wrap_err("Failed to execute milestones get static")?;

	let static_milestones = String::from_utf8_lossy(&static_milestones_output.stdout).trim().to_string();

	// Analyze all screenshots with LLM
	println!("\nAnalyzing screenshots...");
	let prompt = format!(
		r#"You are analyzing screenshots of a user's workspace to assess how relevant their current activity is to their stated goals.

Task identified as current blocker. You're also partially judging relevance of it against daily objectives.
// eg if we are configuring nvim as it's blocking us from codign efficiently, that's already not directly related. But if it gets set to coding an unrelated task or say playing some game, - that's completely off.
{current_blocker}

Daily objectives. Main reference point for judging relevance:
// normally, the `current_blocker` will be relevant to one specific point outlined here, so interpret that one more as a contextual guide as to what should be happening this very moment.
{daily_milestones}

Static task axis (activities judged as always useful, even if I'm procrastinating on the current blocker (but, obviously, reduced relevance weight)):
{static_milestones}

Please analyze the screenshots and rate the relevance of the user's current activity on a scale from 0 to 10, where:
- 0 = Completely unrelated or counterproductive
- 5 = Somewhat related
- 10 = Directly working at the goal; being productive

When scoring, consider:
1. Primary: Relevance to the current blocker and daily objectives (full weight)
2. Static: Relevance to the static task axis (1/3 weight)
3. If an activity is relevant to both primary goals and static activities, that should further increase the score

Provide a brief 1-2 sentence explanation.

Format your response EXACTLY as follows:
<score>N</score>
<explanation>Your explanation here</explanation>

Replace N with an integer from 0 to 10."#
	);

	let message = Message::new_with_text_and_images(Role::User, prompt, screenshot_images);
	tracing::debug!(?message);

	let mut conv = ask_llm::Conversation::new();
	conv.0.push(message);

	match ask_llm::conversation(&conv, Model::Medium, Some(4096), None).await {
		Ok(response) => {
			tracing::debug!("LLM response text: {}", response.text);

			// Parse score
			let score_raw = response.extract_html_tag("score").inspect_err(|_e| {
				eprintln!("Failed to extract <score> tag. Full response:\n{}\n", response.text);
			})?;

			let score_int: i32 = score_raw.trim().parse().wrap_err(format!("Failed to parse score as integer: '{}'", score_raw))?;

			if !(0..=10).contains(&score_int) {
				return Err(color_eyre::eyre::eyre!("Score out of range: {}", score_int));
			}

			let explanation = response.extract_html_tag("explanation").inspect_err(|_e| {
				eprintln!("Failed to extract <explanation> tag. Full response:\n{}\n", response.text);
			})?;

			println!("\nCurrent blocker: {}", current_blocker);
			println!("Relevance score: {}/10", score_int);
			println!("\nExplanation: {}", explanation.trim());

			tracing::info!("Cost: {:.4} cents", response.cost_cents);
		}
		Err(e) => {
			eprintln!("Error calling LLM: {:?}", e);
			return Err(e);
		}
	}

	Ok(())
}
