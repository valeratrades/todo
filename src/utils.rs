use chrono::Duration;
#[cfg(not(test))]
use chrono::Utc;

use crate::config::AppConfig;
#[cfg(test)]
use crate::mocks::Utc;

pub fn format_date(days_back: usize, config: &AppConfig) -> String {
	let date = Utc::now() - Duration::days(days_back as i64);
	let offset = same_day_buffer();

	(date - offset).format(config.date_format.as_str()).to_string()
}

/// Diff of sleep time from 00:00 utc
pub fn same_day_buffer() -> chrono::TimeDelta {
	let waketime = std::env::var("WAKETIME").unwrap();
	let waketime = chrono::NaiveTime::parse_from_str(waketime.as_str(), "%H:%M").unwrap();

	// I don't know what happened here, but I was getting the "used while borrowed"
	let sleep_offset = std::env::var("DAY_SECTION_BORDERS").unwrap();
	let sleep_offset = sleep_offset.split(":").collect::<Vec<&str>>();
	let sleep_offset = sleep_offset.last().unwrap();
	let sleep_offset = sleep_offset.parse::<f64>().unwrap();
	let sleep_offset = chrono::Duration::minutes((sleep_offset * 60.0) as i64);

	let bedtime = waketime + sleep_offset;
	let new_day = bedtime + chrono::Duration::hours(6);
	new_day - chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
}

#[cfg(test)]
mod tests {
	use chrono::TimeZone;

	use super::*;
	use crate::config::AppConfig;

	fn init_test(waketime: &str, day_section_borders: &str, t: Option<(i32, u32, u32, u32, u32, u32)>) -> AppConfig {
		std::env::set_var("WAKETIME", waketime);
		std::env::set_var("DAY_SECTION_BORDERS", day_section_borders);

		if let Some(t) = t {
			let mock_now = chrono::Utc.with_ymd_and_hms(t.0, t.1, t.2, t.3, t.4, t.5).unwrap();
			crate::mocks::set_timestamp(mock_now.timestamp());
		}

		AppConfig {
			date_format: "%Y-%m-%d".to_string(),
			..Default::default()
		}
	}

	#[test]
	fn test_same_day_buffer() {
		let _config = init_test("05:00", "16", Some((2024, 5, 29, 12, 0, 0)));
		let offset = same_day_buffer();

		assert_eq!(offset, chrono::Duration::hours(3).checked_add(&chrono::Duration::minutes(0)).unwrap());
	}

	#[test]
	fn test_format_date() {
		let config = init_test("07:00", "22.5", Some((2024, 5, 29, 12, 0, 0)));

		let formatted_date = format_date(1, &config);
		assert_eq!(formatted_date, "2024-05-28");
	}

	#[test]
	fn test_correct_day() {
		let config = init_test("05:00", "16", Some((2024, 5, 29, 2, 59, 0)));
		let formatted_date = format_date(0, &config);
		assert_eq!(formatted_date, "2024-05-28");

		let config = init_test("05:00", "16", Some((2024, 5, 29, 3, 1, 0)));
		let formatted_date = format_date(0, &config);

		assert_eq!(formatted_date, "2024-05-29");
	}
}
