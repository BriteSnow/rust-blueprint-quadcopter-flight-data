use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use lib_common::event_base::SyncSpscRx;

use crate::{
    metrics::FlushHistogram,
    recorder::{Recorder, Tick},
    support::{check_cancelled, idle_wait},
    Result,
};

#[derive(Default)]
pub struct ConsumerStats {
    pub ticks_total: u64,
    pub flushes: FlushHistogram,
    pub finish_ms: f64,
    pub output_bytes: u64,
}

pub const MAX_BATCH_TICKS: usize = 8192;

pub fn consume(
    mut consumer: SyncSpscRx<Vec<Tick>>,
    mut recorder: Box<dyn Recorder>,
    interval: Duration,
    producer_done: &AtomicBool,
    cancelled: &AtomicBool,
) -> Result<ConsumerStats> {
    let mut stats = ConsumerStats::default();
    let mut batch = Vec::with_capacity(MAX_BATCH_TICKS);
    let mut last_flush = Instant::now();

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
                    &mut batch,
                    &mut stats,
                    interval,
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
                                &mut batch,
                                &mut stats,
                                interval,
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
                    flush(&mut *recorder, &mut batch, &mut stats)?;
                    last_flush = Instant::now();
                }
                idle_wait(cancelled)?;
            }
        }
    }

    flush(&mut *recorder, &mut batch, &mut stats)?;
    let started = Instant::now();
    stats.output_bytes = recorder.finish()?;
    stats.finish_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(stats)
}

// region:    --- Support

fn consume_message(
    message: Vec<Tick>,
    recorder: &mut dyn Recorder,
    batch: &mut Vec<Tick>,
    stats: &mut ConsumerStats,
    interval: Duration,
    last_flush: &mut Instant,
) -> Result<()> {
    for tick in message {
        batch.push(tick);
        if batch.len() == MAX_BATCH_TICKS || last_flush.elapsed() >= interval {
            flush(recorder, batch, stats)?;
            *last_flush = Instant::now();
        }
    }
    Ok(())
}

fn flush(
    recorder: &mut dyn Recorder,
    batch: &mut Vec<Tick>,
    stats: &mut ConsumerStats,
) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let started = Instant::now();
    recorder.write_batch(batch)?;
    stats.flushes.record(started.elapsed());
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
    use crate::recorder::NoopRecorder;
    use lib_common::event_base::{new_spsc_bounded, SyncSpscTx};
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_rec_consume_final_drain() -> Result<()> {
        // -- Setup & Fixtures
        let (mut producer, consumer) = new_spsc_bounded("rec-final-drain", 1)?;
        let ticks = vec![
            Tick { ts_ns: 0, v: [0.0; 5] },
            Tick { ts_ns: 5_000_000, v: [1.0; 5] },
        ];
        queue_ticks(&mut producer, ticks)?;
        let producer_done = AtomicBool::new(true);
        let cancelled = AtomicBool::new(false);

        // -- Exec
        let stats = consume(
            consumer,
            Box::new(NoopRecorder),
            Duration::from_secs(60),
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
            Duration::from_millis(10),
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
        let tick = Tick { ts_ns: 0, v: [1.0; 5] };
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
            Duration::from_secs(86_400),
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
    fn test_rec_consume_write_failure() -> Result<()> {
        // -- Setup & Fixtures
        let (mut producer, consumer) = new_spsc_bounded("rec-write-failure", 1)?;
        queue_ticks(&mut producer, vec![Tick { ts_ns: 0, v: [1.0; 5] }])?;
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
            Duration::from_secs(60),
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
        queue_ticks(&mut producer, vec![Tick { ts_ns: 0, v: [1.0; 5] }])?;
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
            Duration::from_secs(60),
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
