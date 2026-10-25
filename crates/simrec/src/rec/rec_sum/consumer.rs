use super::BucketAccumulator;
use crate::metrics::FlushHistogram;
use crate::Result;
use lib_common::event_base::SyncSpscRx;
use lib_common::Tick;
use lib_db::{create_sum, DbRw, SumRow, SumWriter};
use simple_fs::SPath;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Wait used while the sum ring is empty.
///
/// The summary never reads or sets the shared cancellation flag, it ends on
/// sender disconnection only, so waiting is a plain short sleep.
const EMPTY_WAIT: Duration = Duration::from_millis(1);

/// Width of one summary bucket: half an epoch second in microseconds.
pub const SUM_BUCKET_US: i64 = 500_000;

/// Rows buffered before a commit. Allocated once and reused for the whole run.
const ROWS_CAPACITY: usize = 64;

pub const SUM_DB_FILE_NAME: &str = "capture-sum.sqlite";

#[derive(Default)]
pub struct SumStats {
	/// Ticks folded into rows by this consumer.
	pub ticks_received: u64,
	pub rows_written: u64,
	/// Sum of `tick_count` over written rows.
	pub row_ticks_written: u64,
	/// Rows whose `tick_count` is below a full bucket.
	pub partial_rows: u64,
	pub flushes: FlushHistogram,
	pub wall_time_s: f64,
	pub finish_ms: f64,
	pub output_bytes: u64,
}

/// Creates `capture-sum.sqlite` inside `directory`, with the summary and meta tables.
pub fn create_sum_writer(directory: &SPath, start_epoch_us: i64) -> Result<SumWriter> {
	let database_path = directory.join(SUM_DB_FILE_NAME);
	let mut db = DbRw::create_new(&database_path)?;
	create_sum(&mut db, SUM_BUCKET_US, start_epoch_us)?;

	Ok(SumWriter::new(db))
}

/// Folds `Arc<Vec<Tick>>` batches into per-second rows until the ring disconnects.
///
/// Rows are committed at most every `flush_interval` and once more before the
/// writer closes. The loop holds no cancellation guard and touches no shared
/// state, so it can only end through sender disconnection or an error. Returning
/// on error drops the receiver, which makes the producer see the ring as
/// disconnected.
pub fn consume(
	mut consumer: SyncSpscRx<Arc<Vec<Tick>>>,
	mut writer: SumWriter,
	flush_interval: Duration,
) -> Result<SumStats> {
	let mut stats = SumStats::default();
	let mut acc = BucketAccumulator::new(SUM_BUCKET_US);
	let mut rows: Vec<SumRow> = Vec::with_capacity(ROWS_CAPACITY);
	let started = Instant::now();
	let mut last_flush = started;
	let full_bucket_ticks = (SUM_BUCKET_US / crate::sim::TICK_PERIOD_US) as u32;

	loop {
		match consumer.try_recv() {
			Ok(Some(batch)) => {
				for tick in batch.iter() {
					stats.ticks_received += 1;
					if let Some(row) = acc.push(tick) {
						rows.push(row);
					}
				}
				if last_flush.elapsed() >= flush_interval {
					commit(&mut writer, &mut rows, &mut stats, full_bucket_ticks)?;
					last_flush = Instant::now();
				}
			}
			Ok(None) => {
				if !rows.is_empty() && last_flush.elapsed() >= flush_interval {
					commit(&mut writer, &mut rows, &mut stats, full_bucket_ticks)?;
					last_flush = Instant::now();
				}
				std::thread::sleep(EMPTY_WAIT);
			}
			Err(error) if error.is_disconnected() => break,
			Err(error) => return Err(error.into()),
		}
	}

	if let Some(row) = acc.finish() {
		rows.push(row);
	}
	commit(&mut writer, &mut rows, &mut stats, full_bucket_ticks)?;
	let finish_started = Instant::now();
	let database_path = writer.path().to_path_buf();
	writer.close()?;
	stats.output_bytes = lib_db::db_file_bytes(&database_path)?;
	stats.finish_ms = finish_started.elapsed().as_secs_f64() * 1000.0;
	stats.wall_time_s = started.elapsed().as_secs_f64();

	Ok(stats)
}

// region:    --- Support

fn commit(writer: &mut SumWriter, rows: &mut Vec<SumRow>, stats: &mut SumStats, full_bucket_ticks: u32) -> Result<()> {
	if rows.is_empty() {
		return Ok(());
	}

	let started = Instant::now();
	writer.write_rows(rows)?;
	stats.flushes.record(started.elapsed());
	stats.rows_written += rows.len() as u64;
	stats.row_ticks_written += rows.iter().map(|row| u64::from(row.tick_count)).sum::<u64>();
	stats.partial_rows += rows.iter().filter(|row| row.tick_count < full_bucket_ticks).count() as u64;
	rows.clear();

	Ok(())
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

	use super::*;
	use crate::_test_support::fixture_directory;
	use lib_common::channels::CHANNEL_COUNT;
	use lib_common::event_base::{new_spsc_bounded, SyncSpscTx};

	#[test]
	fn test_rec_sum_consume_final_drain() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_directory("test_rec_sum_consume_final_drain")?;
		let (mut producer, consumer) = new_spsc_bounded("rec-sum-final-drain", 1)?;
		queue_ticks(&mut producer, vec![tick(0), tick(1_000_000)])?;
		drop(producer);
		let writer = create_sum_writer(&directory, 0)?;

		// -- Exec
		let stats = consume(consumer, writer, Duration::from_secs(1))?;

		// -- Check
		assert_eq!(stats.ticks_received, 2);
		assert_eq!(stats.rows_written, 2);
		assert_eq!(stats.row_ticks_written, 2);
		assert_eq!(stats.partial_rows, 2);
		assert_eq!(stats.flushes.count(), 1);
		assert!(stats.output_bytes > 0);
		Ok(())
	}

	#[test]
	fn test_rec_sum_consume_flush_interval_groups_rows() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_directory("test_rec_sum_consume_flush_interval_groups_rows")?;
		let (mut producer, consumer) = new_spsc_bounded("rec-sum-flush-interval", 4)?;
		for index in 0..3_i64 {
			queue_ticks(&mut producer, vec![tick(index * 1_000_000)])?;
		}
		drop(producer);
		let writer = create_sum_writer(&directory, 0)?;

		// -- Exec
		let stats = consume(consumer, writer, Duration::ZERO)?;

		// -- Check
		assert_eq!(stats.ticks_received, 3);
		assert_eq!(stats.rows_written, 3);
		assert_eq!(stats.flushes.count(), 3);
		Ok(())
	}

	#[test]
	fn test_rec_sum_consume_write_error_duplicate_bucket() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_directory("test_rec_sum_consume_write_error_duplicate_bucket")?;
		let (mut producer, consumer) = new_spsc_bounded("rec-sum-duplicate", 4)?;
		queue_ticks(&mut producer, vec![tick(0)])?;
		queue_ticks(&mut producer, vec![tick(2_000_000)])?;
		queue_ticks(&mut producer, vec![tick(0)])?;
		drop(producer);
		let writer = create_sum_writer(&directory, 0)?;

		// -- Exec
		let result = consume(consumer, writer, Duration::ZERO);

		// -- Check
		let error = result.err().ok_or("expected primary key rejection")?;
		assert!(error.to_string().contains("UNIQUE"), "{error}");
		Ok(())
	}

	// -- Test Support

	fn queue_ticks(producer: &mut SyncSpscTx<Arc<Vec<Tick>>>, ticks: Vec<Tick>) -> Result<()> {
		if producer.try_send(Arc::new(ticks))?.is_some() {
			return Err("fixture sum ring unexpectedly full".into());
		}
		Ok(())
	}

	fn tick(epoch_micro: i64) -> Tick {
		Tick {
			epoch_micro,
			v: [1.0; CHANNEL_COUNT],
		}
	}
}

// endregion: --- Tests
