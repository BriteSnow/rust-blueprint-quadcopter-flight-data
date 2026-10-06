use std::{
    fs,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use clap::Parser;
use lib_common::event_base::new_spsc_bounded;
use simple_fs::SPath;

use crate::{
    cmd::CliCmd,
    metrics::Metrics,
    mock_in::MockInput,
    rec,
    rec_raw::RawRecorder,
    rec_sqlite::SqliteRecorder,
    recorder::{NoopRecorder, Recorder},
    sim::{self, ReplaySpeed},
    support::StopGuard,
    Error, Result,
};

pub(super) fn run() -> Result<()> {
    let args = CliCmd::parse();
    let speed = match args.speed.as_str() {
        "realtime" => ReplaySpeed::Realtime,
        "max" => ReplaySpeed::Max,
        _ => return Err(Error::custom("unsupported replay speed")),
    };
    let flush_interval = Duration::from_millis(args.flush_ms);
    if Instant::now().checked_add(flush_interval).is_none() {
        return Err(Error::custom("--flush-ms exceeds the monotonic clock range"));
    }

    let input = MockInput::open(&args.mock_dir)?;
    let expected_ticks = input.tick_count();
    if matches!(speed, ReplaySpeed::Realtime) {
        input.validate_clock_range()?;
    }
    let backend = if args.raw {
        "raw"
    } else if args.sql {
        "sqlite"
    } else {
        "noop"
    };
    let out = args.out.join(backend);
    prepare_output(&out)?;

    let recorder: Box<dyn Recorder> = if args.raw {
        Box::new(RawRecorder::new(&out))
    } else if args.sql {
        Box::new(SqliteRecorder::new(&out)?)
    } else {
        Box::new(NoopRecorder)
    };
    let started = Instant::now();
    let (produced, recorded) =
        replay_workers(input, recorder, speed, flush_interval, args.ring_cap)?;

    if produced.ticks_total != expected_ticks || recorded.ticks_total != expected_ticks {
        return Err(Error::custom(format!(
            "tick conservation failed: expected {expected_ticks}, produced {}, recorded {}",
            produced.ticks_total, recorded.ticks_total
        )));
    }
    let metrics = Metrics::from_run(
        backend,
        args.speed,
        args.flush_ms,
        args.ring_cap,
        started.elapsed(),
        produced,
        recorded,
    );
    metrics.publish(&out)?;
    eprintln!(
        "{backend} replay completed: {expected_ticks} ticks; metrics in {}.",
        out
    );
    Ok(())
}

// region:    --- Support

fn prepare_output(directory: &SPath) -> Result<()> {
    fs::create_dir_all(directory)?;
    if fs::read_dir(directory)?.next().transpose()?.is_some() {
        return Err(Error::custom(format!(
            "output directory {} is not empty; use a new or empty directory",
            directory
        )));
    }
    Ok(())
}

fn replay_workers(
    input: MockInput,
    recorder: Box<dyn Recorder>,
    speed: ReplaySpeed,
    flush_interval: Duration,
    ring_cap: usize,
) -> Result<(sim::ProducerStats, rec::ConsumerStats)> {
    let (producer, consumer) = new_spsc_bounded("sensor-capture", ring_cap)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let producer_done = Arc::new(AtomicBool::new(false));

    thread::scope(|scope| -> Result<_> {
        let producer_cancelled = Arc::clone(&cancelled);
        let producer_finished = Arc::clone(&producer_done);
        let producer_handle = thread::Builder::new()
            .name("simrec-producer".into())
            .spawn_scoped(scope, move || {
                let mut guard = StopGuard::new(Arc::clone(&producer_cancelled));
                let result = sim::produce(input, producer, speed, &producer_cancelled);
                if result.is_ok() {
                    producer_finished.store(true, Ordering::Release);
                    guard.disarm();
                }
                result
            })?;

        let consumer_cancelled = Arc::clone(&cancelled);
        let consumer_finished = Arc::clone(&producer_done);
        let consumer_handle = match thread::Builder::new()
            .name("simrec-consumer".into())
            .spawn_scoped(scope, move || {
                let mut guard = StopGuard::new(Arc::clone(&consumer_cancelled));
                let result = rec::consume(
                    consumer,
                    recorder,
                    flush_interval,
                    &consumer_finished,
                    &consumer_cancelled,
                );
                if result.is_ok() {
                    guard.disarm();
                }
                result
            })
        {
            Ok(handle) => handle,
            Err(error) => {
                cancelled.store(true, Ordering::Release);
                let _ = producer_handle.join();
                return Err(error.into());
            }
        };

        let produced = producer_handle
            .join()
            .map_err(|_| Error::custom("producer thread panicked"))
            .and_then(|result| result);
        let recorded = consumer_handle
            .join()
            .map_err(|_| Error::custom("consumer thread panicked"))
            .and_then(|result| result);

        // Preserve the originating failure rather than its peer's cancellation.
        merge_worker_results(produced, recorded)
    })
}

fn merge_worker_results(
    produced: Result<sim::ProducerStats>,
    recorded: Result<rec::ConsumerStats>,
) -> Result<(sim::ProducerStats, rec::ConsumerStats)> {
    match (produced, recorded) {
        (Ok(produced), Ok(recorded)) => Ok((produced, recorded)),
        (Err(Error::Cancelled), Err(error)) => Err(error),
        (Err(Error::EventBase(peer_error)), Err(error)) if peer_error.is_disconnected() => {
            Err(error)
        }
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

// endregion: --- Tests
