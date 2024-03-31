use crate::config::Config;
use chrono::Duration;

pub fn format_date(days_back: usize, config: &Config) -> String {
	let date: String = (chrono::Utc::now() - Duration::days(days_back as i64))
		.format(&config.date_format.as_str())
		.to_string();
	date
}
