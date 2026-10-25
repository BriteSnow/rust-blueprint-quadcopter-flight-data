use crate::cli::{dataset_candidates, duration_name, duration_tick_count, CliCmd};
use crate::dispatcher::DispatchStats;
use crate::metrics::{Metrics, SumMetrics};
use crate::ostel::{OsCpuSource, SamplerSetup, MAIN_OSTEL_FILE_NAME, SUM_OSTEL_FILE_NAME};
use crate::rec::rec_main::{self, NoopRecorder, RawRecorder, Recorder, SqliteRecorder};
use crate::rec::rec_sum::{create_sum_writer, SumStats};
use crate::run::{run_replay, ReplayOptions, ReplayOutcome, SumSetup, REC_SUM_THREAD_NAME};
use crate::sim::{MockInput, ReplaySpeed};
use crate::{Error, Result};
use clap::Parser;
use simple_fs::SPath;
use std::fs;
use std::io::BufWriter;
use std::time::{Duration, Instant};

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
	let max_batch = config.flush_ms.is_none().then_some(rec_main::MAX_BATCH_TICKS);
	if Instant::now().checked_add(flush_interval).is_none() {
		return Err(Error::custom("--flush-ms exceeds the monotonic clock range"));
	}

	let requested = duration_name(&config.duration)?;
	let requested_ticks = duration_tick_count(&config.duration)?;
	let (dataset_name, dataset) = resolve_dataset(&config.out.join("mock-data"), &config.duration)?;
	let input = MockInput::open(&dataset)?.with_tick_limit(requested_ticks)?;
	let expected_ticks = input.tick_count();
	let start_epoch_us = input.start_epoch_us();
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
		sum_thread_name: REC_SUM_THREAD_NAME,
		interval: Duration::from_millis(config.sample_ms),
	};
	let sum_setup = match config.sum {
		Some(options) => Some(SumSetup {
			writer: create_sum_writer(&out, start_epoch_us)?,
			flush_interval: Duration::from_secs(options.flush_s),
			ring_cap: options.ring_cap,
		}),
		None => None,
	};

	let started = Instant::now();
	let ReplayOutcome {
		produced,
		dispatched,
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

	if produced.ticks_total != expected_ticks
		|| dispatched.ticks_total != expected_ticks
		|| recorded.ticks_total != expected_ticks
	{
		return Err(Error::custom(format!(
			"tick conservation failed: expected {expected_ticks}, produced {}, dispatched {}, recorded {}",
			produced.ticks_total, dispatched.ticks_total, recorded.ticks_total
		)));
	}
	let metrics = Metrics::from_run(
		backend,
		speed_name.to_string(),
		flush_ms,
		config.ring_cap,
		started.elapsed(),
		produced,
		dispatched,
		recorded,
	);
	metrics.publish(&out)?;

	match (config.sum, summary) {
		(Some(options), Some(Ok(stats))) => match check_sum_conservation(&stats, &dispatched, expected_ticks) {
			Ok(()) => {
				let sum_metrics = SumMetrics::from_run(
					speed_name.to_string(),
					options.flush_s,
					options.ring_cap,
					&dispatched,
					stats,
				);
				if let Err(error) = sum_metrics.publish(&out) {
					eprintln!("simrec: summary metrics not published: {error}");
				}
			}
			Err(error) => {
				eprintln!("simrec: summary conservation failed, capture-sum-metrics.json not published: {error}")
			}
		},
		(_, Some(Err(error))) => {
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

pub(crate) fn check_sum_conservation(stats: &SumStats, dispatched: &DispatchStats, ticks_total: u64) -> Result<()> {
	if stats.ticks_received + dispatched.sum_ticks_dropped != ticks_total {
		return Err(Error::custom(format!(
			"summary tick conservation failed: expected {ticks_total}, received {}, dropped {}",
			stats.ticks_received, dispatched.sum_ticks_dropped
		)));
	}
	if stats.row_ticks_written != stats.ticks_received {
		return Err(Error::custom(format!(
			"summary row conservation failed: received {}, written {}",
			stats.ticks_received, stats.row_ticks_written
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

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

// endregion: --- Tests
