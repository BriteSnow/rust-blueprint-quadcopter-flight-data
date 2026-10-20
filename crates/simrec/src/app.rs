use crate::cmd::{dataset_candidates, duration_name, duration_tick_count, CliCmd};
use crate::metrics::{Metrics, SumMetrics};
use crate::mock_in::MockInput;
use crate::rec_raw::RawRecorder;
use crate::rec_sqlite::SqliteRecorder;
use crate::recorder::{NoopRecorder, Recorder};
use crate::sampler::{run_sampler, OsCpuSource, SamplerSetup, MAIN_OSTEL_FILE_NAME, SUM_OSTEL_FILE_NAME, SUM_THREAD_NAME};
use crate::sim::{self, ReplaySpeed};
use crate::sum::{self, SumStats};
use crate::sum_acc::{SumFeed, SumFeedStats};
use crate::sum_sqlite::{SumSqlite, SumWriter};
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

/// Output backend of the main recording, which also names its output folder.
#[derive(Clone, Copy)]
enum Backend {
	Noop,
	Raw,
	Sqlite { with_index: bool },
}

impl Backend {
	fn name(self) -> &'static str {
		match self {
			Self::Noop => "noop",
			Self::Raw => "raw",
			Self::Sqlite { with_index: true } => "sqlite-all-index",
			Self::Sqlite { with_index: false } => "sqlite-pk-index",
		}
	}

	fn recorder(self, out: &SPath) -> Result<Box<dyn Recorder>> {
		let recorder: Box<dyn Recorder> = match self {
			Self::Noop => Box::new(NoopRecorder),
			Self::Raw => Box::new(RawRecorder::new(out)),
			Self::Sqlite { with_index } => Box::new(SqliteRecorder::new(out, with_index)?),
		};
		Ok(recorder)
	}
}

/// Summary options, present unless `--no-sum` disables the summary.
#[derive(Clone, Copy)]
struct SumOptions {
	flush_s: u64,
	ring_cap: usize,
}

/// Resolved capture settings, independent of the CLI parser.
struct CaptureConfig {
	duration: String,
	out: SPath,
	speed: ReplaySpeed,
	flush_ms: Option<u64>,
	ring_cap: usize,
	sample_ms: u64,
	backend: Backend,
	sum: Option<SumOptions>,
}

impl CaptureConfig {
	fn from_cli(args: CliCmd) -> Result<Self> {
		let speed = match args.speed.as_str() {
			"real" => ReplaySpeed::Realtime,
			"max" => ReplaySpeed::Max,
			_ => return Err(Error::custom("unsupported replay speed")),
		};
		let backend = if args.raw {
			Backend::Raw
		} else if args.sql {
			Backend::Sqlite {
				with_index: args.with_index,
			}
		} else {
			Backend::Noop
		};
		let sum = (!args.no_sum).then_some(SumOptions {
			flush_s: args.sum_flush_s,
			ring_cap: args.sum_ring_cap,
		});
		Ok(Self {
			duration: args.duration,
			out: args.out,
			speed,
			flush_ms: args.flush_ms,
			ring_cap: args.ring_cap,
			sample_ms: args.sample_ms,
			backend,
			sum,
		})
	}
}

/// Settings for the summary thread and the sum ring that feeds it.
struct SumSetup {
	writer: Box<dyn SumWriter>,
	flush_interval: Duration,
	ring_cap: usize,
}

/// Result of the replay workers, with the summary outcome kept apart from the main capture.
struct ReplayOutcome {
	produced: sim::ProducerStats,
	recorded: rec::ConsumerStats,
	summary: Option<Result<SumStats>>,
}

/// Timing and sizing of the replay workers.
struct ReplayOptions {
	speed: ReplaySpeed,
	flush_interval: Duration,
	max_batch: Option<usize>,
	ring_cap: usize,
}

/// What a finished capture wrote and where.
struct CaptureReport {
	backend: &'static str,
	ticks: u64,
	out: SPath,
}

pub(super) fn run() -> Result<()> {
	let config = CaptureConfig::from_cli(CliCmd::parse())?;
	let CaptureReport { backend, ticks, out } = run_capture(config)?;
	eprintln!("{backend} replay completed: {ticks} ticks; metrics in {out}.");
	Ok(())
}

fn run_capture(config: CaptureConfig) -> Result<CaptureReport> {
	let flush_ms = config.flush_ms.unwrap_or(DEFAULT_FLUSH_MS);
	let flush_interval = Duration::from_millis(flush_ms);
	let max_batch = config.flush_ms.is_none().then_some(rec::MAX_BATCH_TICKS);
	if Instant::now().checked_add(flush_interval).is_none() {
		return Err(Error::custom("--flush-ms exceeds the monotonic clock range"));
	}

	let requested = duration_name(&config.duration)?;
	let requested_ticks = duration_tick_count(&config.duration)?;
	let (dataset_name, dataset) = resolve_dataset(&config.out.join("mock-data"), &config.duration)?;
	let input = MockInput::open(&dataset)?.with_tick_limit(requested_ticks)?;
	let expected_ticks = input.tick_count();
	if dataset_name != requested {
		eprintln!("duration {requested} replays the first {expected_ticks} ticks of the {dataset_name} mock dataset");
	}
	if matches!(config.speed, ReplaySpeed::Realtime) {
		input.validate_clock_range()?;
	}
	let backend = config.backend.name();
	let speed_name = speed_suffix(&config.speed);
	let out = config
		.out
		.join("captured")
		.join(format!("{requested}-{speed_name}"))
		.join(backend);
	prepare_output(&out)?;

	let recorder = config.backend.recorder(&out)?;
	let sampler = SamplerSetup {
		source: Box::new(OsCpuSource::current()),
		main_writer: Box::new(create_new_file(&out.join(MAIN_OSTEL_FILE_NAME))?),
		sum_writer: match config.sum {
			Some(_) => Some(Box::new(create_new_file(&out.join(SUM_OSTEL_FILE_NAME))?)),
			None => None,
		},
		interval: Duration::from_millis(config.sample_ms),
	};
	let sum_setup = match config.sum {
		Some(options) => Some(SumSetup {
			writer: Box::new(SumSqlite::new(&out)?),
			flush_interval: Duration::from_secs(options.flush_s),
			ring_cap: options.ring_cap,
		}),
		None => None,
	};

	let started = Instant::now();
	let ReplayOutcome {
		produced,
		recorded,
		summary,
	} = run_replay(
		input,
		recorder,
		ReplayOptions {
			speed: config.speed,
			flush_interval,
			max_batch,
			ring_cap: config.ring_cap,
		},
		Some(sampler),
		sum_setup,
	)?;

	if produced.ticks_total != expected_ticks || recorded.ticks_total != expected_ticks {
		return Err(Error::custom(format!(
			"tick conservation failed: expected {expected_ticks}, produced {}, recorded {}",
			produced.ticks_total, recorded.ticks_total
		)));
	}
	let feed = recorded.sum;
	let metrics = Metrics::from_run(
		backend,
		speed_name.to_string(),
		flush_ms,
		config.ring_cap,
		started.elapsed(),
		produced,
		recorded,
	);
	metrics.publish(&out)?;

	match (config.sum, summary, feed) {
		(Some(options), Some(Ok(stats)), Some(feed)) => match check_sum_conservation(&stats, &feed, expected_ticks) {
			Ok(()) => {
				let sum_metrics = SumMetrics::from_run(
					speed_name.to_string(),
					options.flush_s,
					options.ring_cap,
					feed,
					stats,
				);
				if let Err(error) = sum_metrics.publish(&out) {
					eprintln!("simrec: summary metrics not published: {error}");
				}
			}
			Err(error) => eprintln!("simrec: summary conservation failed, capture-sum-metrics.json not published: {error}"),
		},
		(_, Some(Err(error)), _) => {
			eprintln!("simrec: summary failed, capture-sum-metrics.json not published: {error}")
		}
		_ => {}
	}

	Ok(CaptureReport {
		backend,
		ticks: expected_ticks,
		out,
	})
}

// region:    --- Support

fn create_new_file(path: &SPath) -> Result<BufWriter<fs::File>> {
	let file = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
	Ok(BufWriter::new(file))
}

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

fn check_sum_conservation(stats: &SumStats, feed: &SumFeedStats, ticks_total: u64) -> Result<()> {
	if stats.rows_written + feed.rows_dropped != feed.rows_emitted {
		return Err(Error::custom(format!(
			"summary row conservation failed: written {}, dropped {}, emitted {}",
			stats.rows_written, feed.rows_dropped, feed.rows_emitted
		)));
	}
	if feed.rows_dropped == 0 && stats.ticks_summarized != ticks_total {
		return Err(Error::custom(format!(
			"summary tick conservation failed: expected {ticks_total}, summarized {}",
			stats.ticks_summarized
		)));
	}
	Ok(())
}

fn speed_suffix(speed: &ReplaySpeed) -> &'static str {
	match speed {
		ReplaySpeed::Realtime => "real",
		ReplaySpeed::Max => "max",
	}
}

fn run_replay(
	input: MockInput,
	recorder: Box<dyn Recorder>,
	options: ReplayOptions,
	sampler: Option<SamplerSetup>,
	sum_setup: Option<SumSetup>,
) -> Result<ReplayOutcome> {
	let ReplayOptions {
		speed,
		flush_interval,
		max_batch,
		ring_cap,
	} = options;
	let (producer, consumer) = new_spsc_bounded("sensor-capture", ring_cap)?;
	let (sum_feed, sum_channel) = match sum_setup {
		Some(setup) => {
			let (sum_producer, sum_consumer) = new_spsc_bounded("sum-capture", setup.ring_cap)?;
			(
				Some(SumFeed::new(sum_producer)),
				Some((sum_consumer, setup.writer, setup.flush_interval)),
			)
		}
		None => (None, None),
	};
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
				sum_feed,
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

		let summary_handle = sum_channel.map(|(sum_consumer, writer, sum_flush)| {
			thread::Builder::new()
				.name(SUM_THREAD_NAME.into())
				.spawn_scoped(scope, move || sum::consume(sum_consumer, writer, sum_flush))
				.map_err(Error::from)
		});

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

		let summary = match summary_handle {
			Some(Ok(handle)) => Some(
				handle
					.join()
					.map_err(|_| Error::custom("summary thread panicked"))
					.and_then(|result| result),
			),
			Some(Err(error)) => Some(Err(error)),
			None => None,
		};

		sampler_stop.store(true, Ordering::Release);
		if let Some(handle) = sampler_handle {
			if handle.join().is_err() {
				eprintln!("simrec: cpu sampler panicked");
			}
		}

		// Preserve the originating failure rather than its peer's cancellation.
		let (produced, recorded) = merge_worker_results(produced, recorded)?;
		Ok(ReplayOutcome {
			produced,
			recorded,
			summary,
		})
	})
}

#[cfg(test)]
fn replay_workers(
	input: MockInput,
	recorder: Box<dyn Recorder>,
	speed: ReplaySpeed,
	flush_interval: Duration,
	max_batch: Option<usize>,
	ring_cap: usize,
	sampler: Option<SamplerSetup>,
) -> Result<(sim::ProducerStats, rec::ConsumerStats)> {
	let options = ReplayOptions {
		speed,
		flush_interval,
		max_batch,
		ring_cap,
	};
	let outcome = run_replay(input, recorder, options, sampler, None)?;
	Ok((outcome.produced, outcome.recorded))
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
