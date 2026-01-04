use std::cell::Cell;

use chrono::DateTime;
use tracing::instrument;

thread_local! {
	static TIMESTAMP: Cell<i64> = const { Cell::new(0) };
}

pub struct Utc;

impl Utc {
	#[instrument(name = "MockUtc::now")]
	pub fn now() -> DateTime<chrono::Utc> {
		let ts = TIMESTAMP.with(|ts| ts.get());
		tracing::debug!(timestamp = ts, "returning mock timestamp");
		DateTime::from_timestamp(ts, 0).unwrap()
	}
}

#[instrument]
pub fn set_timestamp(timestamp: i64) {
	TIMESTAMP.with(|ts| ts.set(timestamp));
}
