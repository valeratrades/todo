use std::str::FromStr;

use chrono::Duration;
#[cfg(not(test))]
use chrono::Utc;
use color_eyre::eyre::{Report, Result, bail};

use crate::config::AppConfig;
#[cfg(test)]
use crate::mocks::Utc;

pub fn format_date(days_back: usize, config: &AppConfig) -> String {
	let date = Utc::now() - Duration::days(days_back as i64);
	let offset = same_day_buffer();

	let format_str = config.manual_stats.as_ref().map(|ms| ms.date_format.as_str()).unwrap_or("%Y-%m-%d");
	(date - offset).format(format_str).to_string()
}

/// Ends of each day-section as offset to wake-time
#[derive(Clone, Copy, Debug, Default, derive_new::new)]
pub struct DaySectionBorders {
	pub morning_end: f32,
	pub day_end: f32,
	pub evening_end: f32,
}
impl std::str::FromStr for DaySectionBorders {
	type Err = Report;

	fn from_str(borders_str: &str) -> Result<Self> {
		let mut vec_offsets = Vec::with_capacity(3);
		for s in borders_str.split(":") {
			vec_offsets.push(s.parse::<f32>()?);
		}
		if vec_offsets.len() == 3 {
			Ok(Self {
				morning_end: vec_offsets[0],
				day_end: vec_offsets[1],
				evening_end: vec_offsets[2],
			})
		} else {
			bail!("invalid dimensions");
		}
	}
}

/// Diff of sleep time from 00:00 utc
pub fn same_day_buffer() -> chrono::TimeDelta {
	let waketime = std::env::var("WAKETIME").unwrap();
	let waketime = chrono::NaiveTime::parse_from_str(waketime.as_str(), "%H:%M").unwrap();

	let borders = DaySectionBorders::from_str(&std::env::var("DAY_SECTION_BORDERS").unwrap()).unwrap();
	let sleep_offset = chrono::Duration::minutes((borders.evening_end * 60.0) as i64);

	let bedtime = waketime + sleep_offset;
	let new_day = bedtime + chrono::Duration::hours(6);
	new_day - chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
}

#[cfg(test)]
mod tests {
	use chrono::TimeZone;

	use super::*;
	use crate::config::AppConfig;

	fn init_test(t: Option<(i32, u32, u32, u32, u32, u32)>) -> AppConfig {
		// SAFETY: This is only used in tests and doesn't cause race conditions in single-threaded test execution
		unsafe {
			std::env::set_var("WAKETIME", "05:00");
			std::env::set_var("DAY_SECTION_BORDERS", "2.5:10:16");
		}

		if let Some(t) = t {
			let mock_now = chrono::Utc.with_ymd_and_hms(t.0, t.1, t.2, t.3, t.4, t.5).unwrap();
			crate::mocks::set_timestamp(mock_now.timestamp());
		}

		AppConfig {
			manual_stats: Some(crate::config::ManualStats {
				date_format: "%Y-%m-%d".to_string(),
			}),
			..Default::default()
		}
	}

	#[test]
	fn test_same_day_buffer() {
		let _ = init_test(Some((2024, 5, 29, 12, 0, 0)));
		let offset = same_day_buffer();

		assert_eq!(offset, chrono::Duration::hours(3).checked_add(&chrono::Duration::minutes(0)).unwrap());
	}

	#[test]
	fn test_format_date() {
		let config = init_test(Some((2024, 5, 29, 12, 0, 0)));

		let formatted_date = format_date(1, &config);
		assert_eq!(formatted_date, "2024-05-28");
	}

	#[test]
	fn test_correct_day() {
		let config = init_test(Some((2024, 5, 29, 2, 59, 0)));
		let formatted_date = format_date(0, &config);
		assert_eq!(formatted_date, "2024-05-28");

		let config = init_test(Some((2024, 5, 29, 3, 1, 0)));
		let formatted_date = format_date(0, &config);

		assert_eq!(formatted_date, "2024-05-29");
	}
}
