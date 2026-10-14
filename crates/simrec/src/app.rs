use crate::cmd::{dataset_candidates, duration_name, duration_tick_count, CliCmd};
use crate::metrics::Metrics;
use crate::mock_in::MockInput;
use crate::rec_raw::RawRecorder;
use crate::rec_sqlite::SqliteRecorder;
use crate::recorder::{NoopRecorder, Recorder};
use crate::sampler::{run_sampler, OsCpuSource, SamplerSetup, CPU_SAMPLES_FILE_NAME};
use crate::sim::{self, ReplaySpeed};
use crate::support::StopGuard;
use crate::{rec, Error, Result};
use clap::Parser;
use lib_common::event_base::new_spsc_bounded;
use simple_fs::SPath;
use std::io::BufWriter;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{fs, thread};

const DEFAULT_FLUSH_MS: u64 = 100;

pub(super) fn run() -> Result<()> {
	let args = CliCmd::parse();
	let speed = match args.speed.as_str() {
		"real" => ReplaySpeed::Realtime,
		"max" => ReplaySpeed::Max,
		_ => return Err(Error::custom("unsupported replay speed")),
	};
	let flush_ms = args.flush_ms.unwrap_or(DEFAULT_FLUSH_MS);
	let flush_interval = Duration::from_millis(flush_ms);
	let max_batch = args.flush_ms.is_none().then_some(rec::MAX_BATCH_TICKS);
	if Instant::now().checked_add(flush_interval).is_none() {
		return Err(Error::custom("--flush-ms exceeds the monotonic clock range"));
	}

	let requested = duration_name(&args.duration)?;
	let requested_ticks = duration_tick_count(&args.duration)?;
	let (dataset_name, dataset) = resolve_dataset(&args.out.join("mock-data"), &args.duration)?;
	let input = MockInput::open(&dataset)?.with_tick_limit(requested_ticks)?;
	let expected_ticks = input.tick_count();
	if dataset_name != requested {
		eprintln!("duration {requested} replays the first {expected_ticks} ticks of the {dataset_name} mock dataset");
	}
	if matches!(speed, ReplaySpeed::Realtime) {
		input.validate_clock_range()?;
	}
	let backend = if args.raw {
		"raw"
	} else if args.sql && args.with_index {
		"sqlite-all-index"
	} else if args.sql {
		"sqlite-pk-index"
	} else {
		"noop"
	};
	let out = args
		.out
		.join("captured")
		.join(format!("{requested}-{}", speed_suffix(&speed)))
		.join(backend);
	prepare_output(&out)?;

	let recorder: Box<dyn Recorder> = if args.raw {
		Box::new(RawRecorder::new(&out))
	} else if args.sql {
		Box::new(SqliteRecorder::new(&out, args.with_index)?)
	} else {
		Box::new(NoopRecorder)
	};
	let samples_file = fs::OpenOptions::new()
		.write(true)
		.create_new(true)
		.open(out.join(CPU_SAMPLES_FILE_NAME))?;
	let sampler = SamplerSetup {
		source: Box::new(OsCpuSource::current()),
		writer: Box::new(BufWriter::new(samples_file)),
		interval: Duration::from_millis(args.sample_ms),
	};
	let started = Instant::now();
	let (produced, recorded) = replay_workers(
		input,
		recorder,
		speed,
		flush_interval,
		max_batch,
		args.ring_cap,
		Some(sampler),
	)?;

	if produced.ticks_total != expected_ticks || recorded.ticks_total != expected_ticks {
		return Err(Error::custom(format!(
			"tick conservation failed: expected {expected_ticks}, produced {}, recorded {}",
			produced.ticks_total, recorded.ticks_total
		)));
	}
	let metrics = Metrics::from_run(
		backend,
		args.speed,
		flush_ms,
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

fn resolve_dataset(mock_root: &SPath, duration: &str) -> Result<(String, SPath)> {
	let candidates = dataset_candidates(duration)?;
	for &name in candidates {
		let directory = mock_root.join(name);
		if fs::metadata(&directory).is_ok_and(|metadata| metadata.is_dir()) {
			return Ok((name.to_string(), directory));
		}
	}
	Err(Error::custom(format!(
		"no mock dataset for {duration:?} under {mock_root}; expected one of {candidates:?}"
	)))
}

fn speed_suffix(speed: &ReplaySpeed) -> &'static str {
	match speed {
		ReplaySpeed::Realtime => "real",
		ReplaySpeed::Max => "max",
	}
}

fn replay_workers(
	input: MockInput,
	recorder: Box<dyn Recorder>,
	speed: ReplaySpeed,
	flush_interval: Duration,
	max_batch: Option<usize>,
	ring_cap: usize,
	sampler: Option<SamplerSetup>,
) -> Result<(sim::ProducerStats, rec::ConsumerStats)> {
	let (producer, consumer) = new_spsc_bounded("sensor-capture", ring_cap)?;
	let cancelled = Arc::new(AtomicBool::new(false));
	let producer_done = Arc::new(AtomicBool::new(false));
	let sampler_stop = AtomicBool::new(false);

	thread::scope(|scope| -> Result<_> {
		let producer_cancelled = Arc::clone(&cancelled);
		let producer_finished = Arc::clone(&producer_done);
		let producer_handle = thread::Builder::new().name("simrec-sim".into()).spawn_scoped(scope, move || {
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
		let consumer_handle = match thread::Builder::new().name("simrec-rec".into()).spawn_scoped(scope, move || {
			let mut guard = StopGuard::new(Arc::clone(&consumer_cancelled));
			let result = rec::consume(
				consumer,
				recorder,
				flush_interval,
				max_batch,
				&consumer_finished,
				&consumer_cancelled,
			);
			if result.is_ok() {
				guard.disarm();
			}
			result
		}) {
			Ok(handle) => handle,
			Err(error) => {
				cancelled.store(true, Ordering::Release);
				let _ = producer_handle.join();
				return Err(error.into());
			}
		};

		let sampler_handle = match sampler {
			Some(setup) => {
				let stop = &sampler_stop;
				match thread::Builder::new()
					.name("simrec-sampler".into())
					.spawn_scoped(scope, move || run_sampler(setup, stop))
				{
					Ok(handle) => Some(handle),
					Err(error) => {
						eprintln!("simrec: cpu sampler not started: {error}");
						None
					}
				}
			}
			None => None,
		};

		let produced = producer_handle
			.join()
			.map_err(|_| Error::custom("producer thread panicked"))
			.and_then(|result| result);
		let recorded = consumer_handle
			.join()
			.map_err(|_| Error::custom("consumer thread panicked"))
			.and_then(|result| result);

		sampler_stop.store(true, Ordering::Release);
		if let Some(handle) = sampler_handle {
			if handle.join().is_err() {
				eprintln!("simrec: cpu sampler panicked");
			}
		}

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
		(Err(Error::EventBase(peer_error)), Err(error)) if peer_error.is_disconnected() => Err(error),
		(Err(error), _) | (_, Err(error)) => Err(error),
	}
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

// endregion: --- Tests
