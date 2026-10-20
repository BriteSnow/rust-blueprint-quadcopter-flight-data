use crate::metrics::FlushHistogram;
use crate::recorder::{Recorder, Tick};
use crate::sum_acc::{SumFeed, SumFeedStats};
use crate::support::{check_cancelled, idle_wait};
use crate::Result;
use lib_common::event_base::SyncSpscRx;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct ConsumerStats {
	pub ticks_total: u64,
	pub flushes: FlushHistogram,
	pub finish_ms: f64,
	pub output_bytes: u64,

	/// Summary feed counters, absent when the summary is disabled.
	pub sum: Option<SumFeedStats>,
}

/// Flush triggers shared by the consume loop and its message handler.
#[derive(Clone, Copy)]
struct FlushPolicy {
	interval: Duration,
	max_batch: Option<usize>,
}

pub const MAX_BATCH_TICKS: usize = 8192;

pub fn consume(
	mut consumer: SyncSpscRx<Vec<Tick>>,
	mut recorder: Box<dyn Recorder>,
	mut sum: Option<SumFeed>,
	interval: Duration,
	max_batch: Option<usize>,
	producer_done: &AtomicBool,
	cancelled: &AtomicBool,
) -> Result<ConsumerStats> {
	let mut stats = ConsumerStats::default();
	let mut batch = Vec::with_capacity(MAX_BATCH_TICKS);
	let mut last_flush = Instant::now();
	let policy = FlushPolicy { interval, max_batch };

	loop {
		check_cancelled(cancelled)?;
		let message = match consumer.try_recv() {
			Ok(message) => message,
			Err(error) if error.is_disconnected() => None,
			Err(error) => return Err(error.into()),
		};
		match message {
			Some(message) => {
				consume_message(
					message,
					&mut *recorder,
					&mut sum,
					&mut batch,
					&mut stats,
					policy,
					&mut last_flush,
				)?;
			}
			None => {
				if producer_done.load(Ordering::Acquire) {
					// Recheck after acquiring completion so the final push cannot be missed.
					match consumer.try_recv() {
						Ok(Some(message)) => {
							consume_message(
								message,
								&mut *recorder,
								&mut sum,
								&mut batch,
								&mut stats,
									policy,
								&mut last_flush,
							)?;
							continue;
						}
						Ok(None) => break,
						Err(error) if error.is_disconnected() => break,
						Err(error) => return Err(error.into()),
					}
				}
				if !batch.is_empty() && last_flush.elapsed() >= interval {
					flush(&mut *recorder, &mut batch, &mut stats, &mut sum)?;
					last_flush = Instant::now();
				}
				idle_wait(cancelled)?;
			}
		}
	}

	flush(&mut *recorder, &mut batch, &mut stats, &mut sum)?;
	stats.sum = sum.map(SumFeed::finish);
	let started = Instant::now();
	stats.output_bytes = recorder.finish()?;
	stats.finish_ms = started.elapsed().as_secs_f64() * 1000.0;
	Ok(stats)
}

// region:    --- Support

fn consume_message(
	message: Vec<Tick>,
	recorder: &mut dyn Recorder,
	sum: &mut Option<SumFeed>,
	batch: &mut Vec<Tick>,
	stats: &mut ConsumerStats,
	policy: FlushPolicy,
	last_flush: &mut Instant,
) -> Result<()> {
	for tick in message {
		if let Some(sum) = sum.as_mut() {
			sum.push_tick(&tick);
		}
		batch.push(tick);
		if policy.max_batch.is_some_and(|limit| batch.len() == limit) || last_flush.elapsed() >= policy.interval {
			flush(recorder, batch, stats, sum)?;
			*last_flush = Instant::now();
		}
	}
	Ok(())
}

fn flush(
	recorder: &mut dyn Recorder,
	batch: &mut Vec<Tick>,
	stats: &mut ConsumerStats,
	sum: &mut Option<SumFeed>,
) -> Result<()> {
	if batch.is_empty() {
		return Ok(());
	}
	let started = Instant::now();
	recorder.write_batch(batch)?;
	stats.flushes.record(started.elapsed());
	if let Some(sum) = sum.as_mut() {
		sum.try_send();
	}
	stats.ticks_total += batch.len() as u64;
	batch.clear();
	Ok(())
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

	use super::*;
	use crate::recorder::{CHANNEL_COUNT, NoopRecorder};
	use lib_common::event_base::{new_spsc_bounded, SyncSpscTx};
	use std::sync::{Arc, Mutex};

	#[test]
	fn test_rec_consume_final_drain() -> Result<()> {
		// -- Setup & Fixtures
		let (mut producer, consumer) = new_spsc_bounded("rec-final-drain", 1)?;
		let ticks = vec![
			Tick {
				epoch_micro: 0,
				v: [0.0; CHANNEL_COUNT],
			},
			Tick {
				epoch_micro: 5_000,
				v: [1.0; CHANNEL_COUNT],
			},
		];
		queue_ticks(&mut producer, ticks)?;
		let producer_done = AtomicBool::new(true);
		let cancelled = AtomicBool::new(false);

		// -- Exec
		let stats = consume(
			consumer,
			Box::new(NoopRecorder),
			None,
			Duration::from_secs(60),
			Some(MAX_BATCH_TICKS),
			&producer_done,
			&cancelled,
		)?;

		// -- Check
		assert_eq!(stats.ticks_total, 2);
		assert_eq!(stats.flushes.count(), 1);
		Ok(())
	}

	#[test]
	fn test_rec_consume_cancelled() -> Result<()> {
		// -- Setup & Fixtures
		let (_producer, consumer) = new_spsc_bounded("rec-cancelled", 1)?;
		let producer_done = AtomicBool::new(false);
		let cancelled = AtomicBool::new(true);

		// -- Exec
		let result = consume(
			consumer,
			Box::new(NoopRecorder),
			None,
			Duration::from_millis(10),
			Some(MAX_BATCH_TICKS),
			&producer_done,
			&cancelled,
		);

		// -- Check
		assert!(matches!(result, Err(crate::Error::Cancelled)));
		Ok(())
	}

	#[test]
	fn test_rec_consume_batch_limit() -> Result<()> {
		// -- Setup & Fixtures
		let (mut producer, consumer) = new_spsc_bounded("rec-batch-limit", 9)?;
		let tick = Tick {
			epoch_micro: 0,
			v: [1.0; CHANNEL_COUNT],
		};
		for _ in 0..8 {
			queue_ticks(&mut producer, vec![tick; crate::sim::MAX_PENDING_TICKS])?;
		}
		queue_ticks(&mut producer, vec![tick])?;
		let calls = Arc::new(Mutex::new(Vec::new()));
		let recorder = TestRecorder {
			calls: Arc::clone(&calls),
			fail_write: false,
			fail_finish: false,
		};
		let producer_done = AtomicBool::new(true);
		let cancelled = AtomicBool::new(false);

		// -- Exec
		let stats = consume(
			consumer,
			Box::new(recorder),
			None,
			Duration::from_secs(86_400),
			Some(MAX_BATCH_TICKS),
			&producer_done,
			&cancelled,
		)?;

		// -- Check
		let calls = calls.lock().map_err(|_| "test recorder lock poisoned")?;
		assert_eq!(*calls, vec![MAX_BATCH_TICKS, 1, 0]);
		assert_eq!(stats.ticks_total, MAX_BATCH_TICKS as u64 + 1);
		assert_eq!(stats.flushes.count(), 2);
		Ok(())
	}

	#[test]
	fn test_rec_consume_age_only_no_size_flush() -> Result<()> {
		// -- Setup & Fixtures
		let (mut producer, consumer) = new_spsc_bounded("rec-age-only", 1)?;
		let tick = Tick {
			epoch_micro: 0,
			v: [1.0; CHANNEL_COUNT],
		};
		queue_ticks(&mut producer, vec![tick; MAX_BATCH_TICKS + 1])?;
		let calls = Arc::new(Mutex::new(Vec::new()));
		let recorder = TestRecorder {
			calls: Arc::clone(&calls),
			fail_write: false,
			fail_finish: false,
		};
		let producer_done = AtomicBool::new(true);
		let cancelled = AtomicBool::new(false);

		// -- Exec
		let stats = consume(
			consumer,
			Box::new(recorder),
			None,
			Duration::from_secs(86_400),
			None,
			&producer_done,
			&cancelled,
		)?;

		// -- Check
		let calls = calls.lock().map_err(|_| "test recorder lock poisoned")?;
		assert_eq!(*calls, vec![MAX_BATCH_TICKS + 1, 0]);
		assert_eq!(stats.ticks_total, MAX_BATCH_TICKS as u64 + 1);
		assert_eq!(stats.flushes.count(), 1);
		Ok(())
	}

	#[test]
	fn test_rec_consume_write_failure() -> Result<()> {
		// -- Setup & Fixtures
		let (mut producer, consumer) = new_spsc_bounded("rec-write-failure", 1)?;
		queue_ticks(
			&mut producer,
			vec![Tick {
				epoch_micro: 0,
				v: [1.0; CHANNEL_COUNT],
			}],
		)?;
		let calls = Arc::new(Mutex::new(Vec::new()));
		let recorder = TestRecorder {
			calls: Arc::clone(&calls),
			fail_write: true,
			fail_finish: false,
		};
		let producer_done = AtomicBool::new(true);
		let cancelled = AtomicBool::new(false);

		// -- Exec
		let result = consume(
			consumer,
			Box::new(recorder),
			None,
			Duration::from_secs(60),
			Some(MAX_BATCH_TICKS),
			&producer_done,
			&cancelled,
		);

		// -- Check
		let error = result.err().ok_or("expected write failure")?;
		assert_eq!(error.to_string(), "Custom(\"test write failure\")");
		let calls = calls.lock().map_err(|_| "test recorder lock poisoned")?;
		assert_eq!(*calls, vec![1]);
		Ok(())
	}

	#[test]
	fn test_rec_consume_finish_failure() -> Result<()> {
		// -- Setup & Fixtures
		let (mut producer, consumer) = new_spsc_bounded("rec-finish-failure", 1)?;
		queue_ticks(
			&mut producer,
			vec![Tick {
				epoch_micro: 0,
				v: [1.0; CHANNEL_COUNT],
			}],
		)?;
		let calls = Arc::new(Mutex::new(Vec::new()));
		let recorder = TestRecorder {
			calls: Arc::clone(&calls),
			fail_write: false,
			fail_finish: true,
		};
		let producer_done = AtomicBool::new(true);
		let cancelled = AtomicBool::new(false);

		// -- Exec
		let result = consume(
			consumer,
			Box::new(recorder),
			None,
			Duration::from_secs(60),
			Some(MAX_BATCH_TICKS),
			&producer_done,
			&cancelled,
		);

		// -- Check
		let error = result.err().ok_or("expected finish failure")?;
		assert_eq!(error.to_string(), "Custom(\"test finish failure\")");
		let calls = calls.lock().map_err(|_| "test recorder lock poisoned")?;
		assert_eq!(*calls, vec![1, 0]);
		Ok(())
	}

	#[test]
	fn test_rec_consume_sum_feed_full_ring() -> Result<()> {
		// -- Setup & Fixtures
		let (mut producer, consumer) = new_spsc_bounded("rec-sum-feed", 2)?;
		let (sum_producer, mut sum_consumer) = new_spsc_bounded("rec-sum-feed-ring", 1)?;
		let ticks: Vec<Tick> = (0..400_i64)
			.map(|second| Tick {
				epoch_micro: second * 1_000_000,
				v: [1.0; CHANNEL_COUNT],
			})
			.collect();
		for chunk in ticks.chunks(200) {
			queue_ticks(&mut producer, chunk.to_vec())?;
		}
		let producer_done = AtomicBool::new(true);
		let cancelled = AtomicBool::new(false);

		// -- Exec
		let stats = consume(
			consumer,
			Box::new(NoopRecorder),
			Some(SumFeed::new(sum_producer)),
			Duration::from_secs(60),
			Some(MAX_BATCH_TICKS),
			&producer_done,
			&cancelled,
		)?;
		let mut sent = 0_u64;
		loop {
			match sum_consumer.try_recv() {
				Ok(Some(rows)) => sent += rows.len() as u64,
				Ok(None) => break,
				Err(error) if error.is_disconnected() => break,
				Err(error) => return Err(error.into()),
			}
		}

		// -- Check
		let sum = stats.sum.ok_or("expected sum feed counters")?;
		assert_eq!(stats.ticks_total, 400);
		assert_eq!(sum.rows_emitted, 400);
		assert!(sum.rows_dropped > 0);
		assert_eq!(sent + sum.rows_dropped, sum.rows_emitted);
		Ok(())
	}

	// -- Test Support

	fn queue_ticks(producer: &mut SyncSpscTx<Vec<Tick>>, ticks: Vec<Tick>) -> Result<()> {
		if producer.try_send(ticks)?.is_some() {
			return Err("fixture ring unexpectedly full".into());
		}
		Ok(())
	}

	struct TestRecorder {
		calls: Arc<Mutex<Vec<usize>>>,
		fail_write: bool,
		fail_finish: bool,
	}

	impl Recorder for TestRecorder {
		fn write_batch(&mut self, ticks: &[Tick]) -> crate::Result<()> {
			self.calls
				.lock()
				.map_err(|_| crate::Error::custom("test recorder lock poisoned"))?
				.push(ticks.len());
			if self.fail_write {
				return Err(crate::Error::custom("test write failure"));
			}
			Ok(())
		}

		fn finish(self: Box<Self>) -> crate::Result<u64> {
			self.calls
				.lock()
				.map_err(|_| crate::Error::custom("test recorder lock poisoned"))?
				.push(0);
			if self.fail_finish {
				return Err(crate::Error::custom("test finish failure"));
			}
			Ok(0)
		}
	}
}

// endregion: --- Tests
