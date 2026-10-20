use crate::metrics::FlushHistogram;
use crate::sum_acc::SumRow;
use crate::sum_sqlite::SumWriter;
use crate::Result;
use lib_common::event_base::SyncSpscRx;
use std::time::{Duration, Instant};

/// Wait used while the sum ring is empty.
///
/// The summary never reads or sets the shared cancellation flag, it ends on
/// sender disconnection only, so waiting is a plain short sleep.
const EMPTY_WAIT: Duration = Duration::from_millis(1);

#[derive(Default)]
pub struct SumStats {
	pub rows_written: u64,
	pub ticks_summarized: u64,
	pub flushes: FlushHistogram,
	pub wall_time_s: f64,
	pub finish_ms: f64,
	pub output_bytes: u64,
}

/// Drains `Vec<SumRow>` messages into committed batches until the ring disconnects.
///
/// Rows are committed at most every `flush_interval` and once more before
/// `finish`. The loop holds no cancellation guard and touches no shared state, so
/// it can only end through sender disconnection or an error. Returning on error
/// drops the receiver, which makes the producer see the ring as disconnected.
pub fn consume(
	mut consumer: SyncSpscRx<Vec<SumRow>>,
	mut writer: Box<dyn SumWriter>,
	flush_interval: Duration,
) -> Result<SumStats> {
	let mut stats = SumStats::default();
	let mut batch: Vec<SumRow> = Vec::new();
	let started = Instant::now();
	let mut last_flush = started;

	loop {
		match consumer.try_recv() {
			Ok(Some(rows)) => {
				batch.extend(rows);
				if last_flush.elapsed() >= flush_interval {
					commit(&mut *writer, &mut batch, &mut stats)?;
					last_flush = Instant::now();
				}
			}
			Ok(None) => {
				if !batch.is_empty() && last_flush.elapsed() >= flush_interval {
					commit(&mut *writer, &mut batch, &mut stats)?;
					last_flush = Instant::now();
				}
				std::thread::sleep(EMPTY_WAIT);
			}
			Err(error) if error.is_disconnected() => break,
			Err(error) => return Err(error.into()),
		}
	}

	commit(&mut *writer, &mut batch, &mut stats)?;
	let finish_started = Instant::now();
	stats.output_bytes = writer.finish()?;
	stats.finish_ms = finish_started.elapsed().as_secs_f64() * 1000.0;
	stats.wall_time_s = started.elapsed().as_secs_f64();

	Ok(stats)
}

// region:    --- Support

fn commit(writer: &mut dyn SumWriter, batch: &mut Vec<SumRow>, stats: &mut SumStats) -> Result<()> {
	if batch.is_empty() {
		return Ok(());
	}

	let started = Instant::now();
	writer.write_rows(batch)?;
	stats.flushes.record(started.elapsed());
	stats.rows_written += batch.len() as u64;
	stats.ticks_summarized += batch.iter().map(|row| u64::from(row.tick_count)).sum::<u64>();
	batch.clear();

	Ok(())
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

	use super::*;
	use crate::recorder::CHANNEL_COUNT;
	use lib_common::event_base::{new_spsc_bounded, SyncSpscTx};
	use std::sync::{Arc, Mutex};

	#[test]
	fn test_sum_consume_final_drain() -> Result<()> {
		// -- Setup & Fixtures
		let (mut producer, consumer) = new_spsc_bounded("sum-final-drain", 1)?;
		queue_rows(&mut producer, vec![sum_row(0, 200), sum_row(1_000_000, 150)])?;
		drop(producer);
		let calls = Arc::new(Mutex::new(Vec::new()));
		let writer = TestWriter {
			calls: Arc::clone(&calls),
			fail_write: false,
		};

		// -- Exec
		let stats = consume(consumer, Box::new(writer), Duration::from_secs(1))?;

		// -- Check
		let calls = calls.lock().map_err(|_| "test writer lock poisoned")?;
		assert_eq!(*calls, vec![2, 0]);
		assert_eq!(stats.rows_written, 2);
		assert_eq!(stats.ticks_summarized, 350);
		assert_eq!(stats.flushes.count(), 1);
		assert_eq!(stats.output_bytes, 0);
		Ok(())
	}

	#[test]
	fn test_sum_consume_flush_interval_groups_rows() -> Result<()> {
		// -- Setup & Fixtures
		let patient_interval = Duration::from_secs(86_400);
		let eager_interval = Duration::ZERO;

		// -- Exec
		let (patient_calls, patient_stats) = run_row_messages(patient_interval, 3)?;
		let (eager_calls, eager_stats) = run_row_messages(eager_interval, 3)?;

		// -- Check
		assert_eq!(patient_calls, vec![3, 0]);
		assert_eq!(patient_stats.rows_written, 3);
		assert_eq!(patient_stats.flushes.count(), 1);

		assert_eq!(eager_calls, vec![1, 1, 1, 0]);
		assert_eq!(eager_stats.rows_written, 3);
		assert_eq!(eager_stats.flushes.count(), 3);
		Ok(())
	}

	#[test]
	fn test_sum_consume_ticks_summarized() -> Result<()> {
		// -- Setup & Fixtures
		let (mut producer, consumer) = new_spsc_bounded("sum-ticks-summarized", 2)?;
		queue_rows(&mut producer, vec![sum_row(0, 200), sum_row(1_000_000, 200)])?;
		queue_rows(&mut producer, vec![sum_row(2_000_000, 150)])?;
		drop(producer);
		let calls = Arc::new(Mutex::new(Vec::new()));
		let writer = TestWriter {
			calls: Arc::clone(&calls),
			fail_write: false,
		};

		// -- Exec
		let stats = consume(consumer, Box::new(writer), Duration::from_secs(60))?;

		// -- Check
		let calls = calls.lock().map_err(|_| "test writer lock poisoned")?;
		assert_eq!(*calls, vec![3, 0]);
		assert_eq!(stats.rows_written, 3);
		assert_eq!(stats.ticks_summarized, 550);
		Ok(())
	}

	#[test]
	fn test_sum_consume_write_failure() -> Result<()> {
		// -- Setup & Fixtures
		let (mut producer, consumer) = new_spsc_bounded("sum-write-failure", 1)?;
		queue_rows(&mut producer, vec![sum_row(0, 200)])?;
		drop(producer);
		let calls = Arc::new(Mutex::new(Vec::new()));
		let writer = TestWriter {
			calls: Arc::clone(&calls),
			fail_write: true,
		};

		// -- Exec
		let result = consume(consumer, Box::new(writer), Duration::from_secs(1));

		// -- Check
		let error = result.err().ok_or("expected write failure")?;
		assert_eq!(error.to_string(), "Custom(\"test sum write failure\")");
		let calls = calls.lock().map_err(|_| "test writer lock poisoned")?;
		assert_eq!(*calls, vec![1]);
		Ok(())
	}

	// -- Test Support

	fn run_row_messages(flush_interval: Duration, message_count: u32) -> Result<(Vec<usize>, SumStats)> {
		let (mut producer, consumer) = new_spsc_bounded("sum-flush-interval", message_count as usize)?;
		for index in 0..message_count {
			queue_rows(&mut producer, vec![sum_row(i64::from(index), 10 + index)])?;
		}
		drop(producer);

		let calls = Arc::new(Mutex::new(Vec::new()));
		let writer = TestWriter {
			calls: Arc::clone(&calls),
			fail_write: false,
		};

		let stats = consume(consumer, Box::new(writer), flush_interval)?;

		let calls = calls.lock().map_err(|_| "test writer lock poisoned")?.clone();
		Ok((calls, stats))
	}

	fn queue_rows(producer: &mut SyncSpscTx<Vec<SumRow>>, rows: Vec<SumRow>) -> Result<()> {
		if producer.try_send(rows)?.is_some() {
			return Err("fixture sum ring unexpectedly full".into());
		}
		Ok(())
	}

	fn sum_row(ts_epoch_micro: i64, tick_count: u32) -> SumRow {
		SumRow {
			ts_epoch_micro,
			tick_count,
			min: [1.0; CHANNEL_COUNT],
			max: [2.0; CHANNEL_COUNT],
			avg: [1.5; CHANNEL_COUNT],
		}
	}

	struct TestWriter {
		calls: Arc<Mutex<Vec<usize>>>,
		fail_write: bool,
	}

	impl SumWriter for TestWriter {
		fn write_rows(&mut self, rows: &[SumRow]) -> crate::Result<()> {
			self.calls
				.lock()
				.map_err(|_| "test writer lock poisoned")?
				.push(rows.len());
			if self.fail_write {
				return Err("test sum write failure".into());
			}
			Ok(())
		}

		fn finish(self: Box<Self>) -> crate::Result<u64> {
			self.calls
				.lock()
				.map_err(|_| "test writer lock poisoned")?
				.push(0);
			Ok(0)
		}
	}
}

// endregion: --- Tests
