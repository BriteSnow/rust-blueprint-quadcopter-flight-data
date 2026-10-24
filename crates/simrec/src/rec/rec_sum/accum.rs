use lib_common::channels::CHANNEL_COUNT;
use lib_common::Tick;
use lib_db::SumRow;

/// Builds one [`SumRow`] per epoch second, keyed by the tick timestamp.
#[derive(Debug)]
pub struct BucketAccumulator {
	bucket_us: i64,
	current_us: Option<i64>,
	tick_count: u32,
	min: [f64; CHANNEL_COUNT],
	max: [f64; CHANNEL_COUNT],
	sum: [f64; CHANNEL_COUNT],
}

/// Accumulation
impl BucketAccumulator {
	pub fn new(bucket_us: i64) -> Self {
		Self {
			bucket_us,
			current_us: None,
			tick_count: 0,
			min: [f64::INFINITY; CHANNEL_COUNT],
			max: [f64::NEG_INFINITY; CHANNEL_COUNT],
			sum: [0.0; CHANNEL_COUNT],
		}
	}

	/// Adds one tick, returning the row closed when that tick starts a new second.
	pub fn push(&mut self, tick: &Tick) -> Option<SumRow> {
		let start_us = bucket_start(tick.epoch_micro, self.bucket_us);

		let closed = match self.current_us {
			Some(current) if current != start_us => self.take_row(),
			_ => None,
		};

		if self.current_us != Some(start_us) {
			self.current_us = Some(start_us);
		}

		self.add(tick);

		closed
	}

	/// Closes the current, possibly partial, second.
	pub fn finish(mut self) -> Option<SumRow> {
		self.take_row()
	}
}

// region:    --- Support

/// Start of the epoch second that holds `epoch_micro`, also correct before the epoch.
fn bucket_start(epoch_micro: i64, bucket_us: i64) -> i64 {
	epoch_micro - epoch_micro.rem_euclid(bucket_us)
}

impl BucketAccumulator {
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
		let ts_epoch_micro = self.current_us.take()?;
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

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

	use super::*;

	#[test]
	fn test_rec_sum_acc_push_whole_second() -> Result<()> {
		// -- Setup & Fixtures
		let mut acc = BucketAccumulator::new(1_000_000);
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
		assert_eq!(last.ts_epoch_micro, start_us + 1_000_000);
		assert_eq!(last.tick_count, 200);

		Ok(())
	}

	#[test]
	fn test_rec_sum_acc_push_mid_second() -> Result<()> {
		// -- Setup & Fixtures
		let mut acc = BucketAccumulator::new(1_000_000);
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
	fn test_rec_sum_acc_push_negative_epoch() -> Result<()> {
		// -- Setup & Fixtures
		let mut acc = BucketAccumulator::new(1_000_000);

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
	fn test_rec_sum_acc_push_min_max_avg() -> Result<()> {
		// -- Setup & Fixtures
		let mut acc = BucketAccumulator::new(1_000_000);
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

	// -- Test Support

	fn tick_const(epoch_micro: i64, value: f32) -> Tick {
		Tick {
			epoch_micro,
			v: [value; CHANNEL_COUNT],
		}
	}
}

// endregion: --- Tests
