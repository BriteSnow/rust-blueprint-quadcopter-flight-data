use crate::recorder::{CHANNEL_COUNT, Tick};
use lib_common::event_base::SyncSpscTx;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Width of one summary bucket: one epoch second in microseconds.
pub const BUCKET_US: i64 = 1_000_000;

/// Maximum rows held while the sum ring refuses them.
pub const MAX_PENDING_SUM_ROWS: usize = 3_600;

/// How long [`SumFeed::finish`] keeps retrying the last send.
const FINISH_SEND_TIMEOUT: Duration = Duration::from_secs(1);

/// Plain sleep between the final send attempts.
const FINISH_SEND_WAIT: Duration = Duration::from_millis(5);

/// Per-second summary of all channels, closed by [`SecAccumulator`].
#[derive(Debug, Clone, PartialEq)]
pub struct SumRow {
	/// Start of the epoch second that holds the summarized ticks.
	pub ts_epoch_micro: i64,

	/// Number of ticks in the row, 200 for a full second.
	pub tick_count: u32,

	pub min: [f64; CHANNEL_COUNT],
	pub max: [f64; CHANNEL_COUNT],
	pub avg: [f64; CHANNEL_COUNT],
}

/// Builds one [`SumRow`] per epoch second, keyed by the tick timestamp.
#[derive(Debug)]
pub struct SecAccumulator {
	bucket_us: Option<i64>,
	tick_count: u32,
	min: [f64; CHANNEL_COUNT],
	max: [f64; CHANNEL_COUNT],
	sum: [f64; CHANNEL_COUNT],
}

/// Accumulation
impl SecAccumulator {
	/// Adds one tick, returning the row closed when that tick starts a new second.
	pub fn push(&mut self, tick: &Tick) -> Option<SumRow> {
		let bucket_us = bucket_start(tick.epoch_micro);

		let closed = match self.bucket_us {
			Some(current) if current != bucket_us => self.take_row(),
			_ => None,
		};

		if self.bucket_us != Some(bucket_us) {
			self.bucket_us = Some(bucket_us);
		}

		self.add(tick);

		closed
	}

	/// Closes the current, possibly partial, second.
	pub fn finish(mut self) -> Option<SumRow> {
		self.take_row()
	}
}

impl Default for SecAccumulator {
	fn default() -> Self {
		Self {
			bucket_us: None,
			tick_count: 0,
			min: [f64::INFINITY; CHANNEL_COUNT],
			max: [f64::NEG_INFINITY; CHANNEL_COUNT],
			sum: [0.0; CHANNEL_COUNT],
		}
	}
}

/// Counters describing what the rec side did with its closed rows.
#[derive(Debug, Default, Clone, Copy)]
pub struct SumFeedStats {
	/// Rows closed by the accumulator.
	pub rows_emitted: u64,

	/// Rows that never reached the sum ring.
	pub rows_dropped: u64,

	/// Peak number of rows held while the ring refused them.
	pub max_pending_rows: usize,
}

/// Forwards closed seconds to the sum ring without ever waiting on it.
#[derive(Debug)]
pub struct SumFeed {
	tx: SyncSpscTx<Vec<SumRow>>,
	acc: SecAccumulator,
	pending: VecDeque<SumRow>,
	disconnected: bool,
	stats: SumFeedStats,
}

/// Feeding
impl SumFeed {
	pub fn new(tx: SyncSpscTx<Vec<SumRow>>) -> Self {
		Self {
			tx,
			acc: SecAccumulator::default(),
			pending: VecDeque::new(),
			disconnected: false,
			stats: SumFeedStats::default(),
		}
	}

	/// Records one tick, queueing the row it closes.
	pub fn push_tick(&mut self, tick: &Tick) {
		if let Some(row) = self.acc.push(tick) {
			self.queue(row);
		}
	}

	/// Offers every pending row to the sum ring as one message.
	pub fn try_send(&mut self) {
		if self.disconnected || self.pending.is_empty() {
			return;
		}

		let rows: Vec<SumRow> = self.pending.drain(..).collect();
		let row_count = rows.len() as u64;
		match self.tx.try_send(rows) {
			Ok(None) => {}
			Ok(Some(rows)) => self.pending.extend(rows),
			Err(_) => {
				// Disconnection is the only send failure, and the message is already lost.
				self.disconnected = true;
				self.stats.rows_dropped += row_count;
			}
		}
	}

	/// Closes the last second and retries the final send for a bounded time.
	pub fn finish(mut self) -> SumFeedStats {
		let acc = core::mem::take(&mut self.acc);
		if let Some(row) = acc.finish() {
			self.queue(row);
		}

		let deadline = Instant::now() + FINISH_SEND_TIMEOUT;
		while !self.pending.is_empty() {
			self.try_send();
			if self.pending.is_empty() || self.disconnected || Instant::now() >= deadline {
				break;
			}
			std::thread::sleep(FINISH_SEND_WAIT);
		}

		if !self.pending.is_empty() {
			self.stats.rows_dropped += self.pending.len() as u64;
			self.pending.clear();
		}

		self.stats
	}
}

// region:    --- Support

/// Start of the epoch second that holds `epoch_micro`, also correct before the epoch.
fn bucket_start(epoch_micro: i64) -> i64 {
	epoch_micro - epoch_micro.rem_euclid(BUCKET_US)
}

impl SecAccumulator {
	fn add(&mut self, tick: &Tick) {
		for (idx, value) in tick.v.iter().enumerate() {
			let value = f64::from(*value);

			if value < self.min[idx] {
				self.min[idx] = value;
			}
			if value > self.max[idx] {
				self.max[idx] = value;
			}

			self.sum[idx] += value;
		}

		self.tick_count += 1;
	}

	/// Takes the current bucket as a row and clears the accumulator state.
	fn take_row(&mut self) -> Option<SumRow> {
		let ts_epoch_micro = self.bucket_us.take()?;
		let tick_count = self.tick_count;

		let avg = core::array::from_fn(|idx| self.sum[idx] / f64::from(tick_count));

		let row = SumRow {
			ts_epoch_micro,
			tick_count,
			min: self.min,
			max: self.max,
			avg,
		};

		self.tick_count = 0;
		self.min = [f64::INFINITY; CHANNEL_COUNT];
		self.max = [f64::NEG_INFINITY; CHANNEL_COUNT];
		self.sum = [0.0; CHANNEL_COUNT];

		Some(row)
	}
}

impl SumFeed {
	/// Adds one closed row, honoring the pending bound and any known disconnection.
	fn queue(&mut self, row: SumRow) {
		self.stats.rows_emitted += 1;

		if self.disconnected {
			self.stats.rows_dropped += 1;
			return;
		}

		self.pending.push_back(row);
		if self.pending.len() > MAX_PENDING_SUM_ROWS {
			self.pending.pop_front();
			self.stats.rows_dropped += 1;
		}

		self.stats.max_pending_rows = self.stats.max_pending_rows.max(self.pending.len());
	}
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

	use super::*;
	use lib_common::event_base::{new_spsc_bounded, SyncSpscRx};

	#[test]
	fn test_sum_acc_push_whole_second() -> Result<()> {
		// -- Setup & Fixtures
		let mut acc = SecAccumulator::default();
		let start_us = 1_700_000_000_000_000i64;

		// -- Exec
		let mut rows = Vec::new();
		for i in 0..400i64 {
			if let Some(row) = acc.push(&tick_const(start_us + i * 5_000, 1.0)) {
				rows.push(row);
			}
		}
		if let Some(row) = acc.finish() {
			rows.push(row);
		}

		// -- Check
		assert_eq!(rows.len(), 2);

		let first = rows.first().ok_or("Should have a first row")?;
		assert_eq!(first.ts_epoch_micro, start_us);
		assert_eq!(first.tick_count, 200);

		let last = rows.last().ok_or("Should have a last row")?;
		assert_eq!(last.ts_epoch_micro, start_us + BUCKET_US);
		assert_eq!(last.tick_count, 200);

		Ok(())
	}

	#[test]
	fn test_sum_acc_push_mid_second() -> Result<()> {
		// -- Setup & Fixtures
		let mut acc = SecAccumulator::default();
		let start_us = 1_700_000_000_500_000i64;

		// -- Exec
		let mut rows = Vec::new();
		for i in 0..250i64 {
			if let Some(row) = acc.push(&tick_const(start_us + i * 5_000, 1.0)) {
				rows.push(row);
			}
		}
		if let Some(row) = acc.finish() {
			rows.push(row);
		}

		// -- Check
		assert_eq!(rows.len(), 2);

		let first = rows.first().ok_or("Should have a first row")?;
		assert_eq!(first.ts_epoch_micro, start_us - 500_000);
		assert_eq!(first.tick_count, 100);

		let last = rows.last().ok_or("Should have a last row")?;
		assert_eq!(last.ts_epoch_micro, start_us + 500_000);
		assert_eq!(last.tick_count, 150);

		assert_eq!(u64::from(first.tick_count) + u64::from(last.tick_count), 250);

		Ok(())
	}

	#[test]
	fn test_sum_acc_push_negative_epoch() -> Result<()> {
		// -- Setup & Fixtures
		let mut acc = SecAccumulator::default();

		// -- Exec
		let closed_mid = acc.push(&tick_const(-1, 1.0));
		let closed_same = acc.push(&tick_const(-1_000_000, 2.0));
		let closed_next = acc.push(&tick_const(-1_000_001, 3.0));

		// -- Check
		assert!(closed_mid.is_none());
		assert!(closed_same.is_none());

		let row = closed_next.ok_or("Should close the previous second")?;
		assert_eq!(row.ts_epoch_micro, -1_000_000);
		assert_eq!(row.tick_count, 2);
		assert_eq!(row.min[0], 1.0);
		assert_eq!(row.max[0], 2.0);

		Ok(())
	}

	#[test]
	fn test_sum_acc_push_min_max_avg() -> Result<()> {
		// -- Setup & Fixtures
		let mut acc = SecAccumulator::default();
		let start_us = 1_700_000_000_000_000i64;
		let offsets = [0.0f32, 4.0, 2.0];

		// -- Exec
		let mut rows = Vec::new();
		for (i, offset) in offsets.into_iter().enumerate() {
			let tick = Tick {
				epoch_micro: start_us + i as i64 * 5_000,
				v: core::array::from_fn(|ch| ch as f32 + offset),
			};

			if let Some(row) = acc.push(&tick) {
				rows.push(row);
			}
		}
		if let Some(row) = acc.finish() {
			rows.push(row);
		}

		// -- Check
		assert_eq!(rows.len(), 1);

		let row = rows.first().ok_or("Should have one row")?;
		assert_eq!(row.tick_count, 3);
		for ch in 0..CHANNEL_COUNT {
			assert_eq!(row.min[ch], ch as f64);
			assert_eq!(row.max[ch], ch as f64 + 4.0);
			assert_eq!(row.avg[ch], ch as f64 + 2.0);
		}

		Ok(())
	}

	#[test]
	fn test_sum_acc_feed_normal_run() -> Result<()> {
		// -- Setup & Fixtures
		let start_us = 1_700_000_000_000_000i64;
		let (tx, mut rx) = new_spsc_bounded("sum-feed-normal", 4)?;
		let mut feed = SumFeed::new(tx);

		// -- Exec
		for second in 0..3i64 {
			feed.push_tick(&tick_const(start_us + second * BUCKET_US, 1.0));
			feed.try_send();
		}
		let stats = feed.finish();
		let rows = drain_rows(&mut rx)?;

		// -- Check
		assert_eq!(stats.rows_emitted, 3);
		assert_eq!(stats.rows_dropped, 0);
		assert_eq!(stats.max_pending_rows, 1);
		assert_eq!(rows.len(), 3);
		assert_eq!(rows.first().map(|row| row.ts_epoch_micro), Some(start_us));
		assert_eq!(rows.last().map(|row| row.ts_epoch_micro), Some(start_us + 2 * BUCKET_US));
		Ok(())
	}

	#[test]
	fn test_sum_acc_feed_receiver_gone() -> Result<()> {
		// -- Setup & Fixtures
		let start_us = 1_700_000_000_000_000i64;
		let (tx, rx) = new_spsc_bounded("sum-feed-gone", 1)?;
		drop(rx);
		let mut feed = SumFeed::new(tx);

		// -- Exec
		for second in 0..4i64 {
			feed.push_tick(&tick_const(start_us + second * BUCKET_US, 1.0));
			feed.try_send();
		}
		let stats = feed.finish();

		// -- Check
		assert_eq!(stats.rows_emitted, 4);
		assert_eq!(stats.rows_dropped, 4);
		Ok(())
	}

	#[test]
	fn test_sum_acc_feed_full_ring_drops_oldest() -> Result<()> {
		// -- Setup & Fixtures
		let start_us = 1_700_000_000_000_000i64;
		let (tx, mut rx) = new_spsc_bounded("sum-feed-full", 1)?;
		let mut feed = SumFeed::new(tx);
		let push_count = MAX_PENDING_SUM_ROWS + 100;

		// -- Exec
		for second in 0..push_count as i64 {
			feed.push_tick(&tick_const(start_us + second * BUCKET_US, 1.0));
		}
		feed.try_send();
		let sent_first = drain_rows(&mut rx)?;
		let stats = feed.finish();
		let sent_last = drain_rows(&mut rx)?;

		// -- Check
		assert_eq!(stats.rows_emitted, push_count as u64);
		assert_eq!(stats.rows_dropped, push_count as u64 - 1 - MAX_PENDING_SUM_ROWS as u64);
		assert_eq!(stats.max_pending_rows, MAX_PENDING_SUM_ROWS);
		assert_eq!(sent_first.len(), MAX_PENDING_SUM_ROWS);
		assert_eq!(sent_last.len(), 1);
		Ok(())
	}

	// -- Test Support

	fn tick_const(epoch_micro: i64, value: f32) -> Tick {
		Tick {
			epoch_micro,
			v: [value; CHANNEL_COUNT],
		}
	}

	fn drain_rows(consumer: &mut SyncSpscRx<Vec<SumRow>>) -> Result<Vec<SumRow>> {
		let mut rows = Vec::new();
		loop {
			match consumer.try_recv() {
				Ok(Some(message)) => rows.extend(message),
				Ok(None) => break,
				Err(error) if error.is_disconnected() => break,
				Err(error) => return Err(error.into()),
			}
		}
		Ok(rows)
	}
}

// endregion: --- Tests
