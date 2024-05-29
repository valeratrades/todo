use chrono::DateTime;
use std::cell::Cell;

thread_local! {
	static TIMESTAMP: Cell<i64> = const { Cell::new(0) };
}

pub struct Utc;

impl Utc {
	pub fn now() -> DateTime<chrono::Utc> {
		DateTime::from_timestamp(TIMESTAMP.with(|ts| ts.get()), 0).unwrap()
	}
}

pub fn set_timestamp(timestamp: i64) {
	TIMESTAMP.with(|ts| ts.set(timestamp));
}
