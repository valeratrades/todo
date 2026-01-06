use std::cell::Cell;

use jiff::Timestamp;
use tracing::instrument;

thread_local! {
	static MOCK_TIMESTAMP: Cell<Option<Timestamp>> = const { Cell::new(None) };
}

pub struct MockTimestamp;

impl MockTimestamp {
	#[instrument(name = "MockTimestamp::now")]
	pub fn now() -> Timestamp {
		let ts = MOCK_TIMESTAMP.with(|ts| ts.get());
		tracing::debug!(?ts, "returning mock timestamp");
		ts.unwrap_or_else(Timestamp::now)
	}
}

#[instrument]
pub fn set_timestamp(timestamp: Timestamp) {
	MOCK_TIMESTAMP.with(|ts| ts.set(Some(timestamp)));
}
