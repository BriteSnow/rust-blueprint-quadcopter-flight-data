use crate::dispatcher::{dispatch, DispatchStats};
use crate::ostel::{run_sampler, SamplerSetup};
use crate::rec::rec_main::{self, ConsumerStats, Recorder};
use crate::rec::rec_sum::{self, SumStats};
use crate::sim::{self, MockInput, ProducerStats, ReplaySpeed};
use crate::support::StopGuard;
use crate::{Error, Result};
use lib_common::event_base::new_spsc_bounded;
use lib_common::Tick;
use lib_db::SumWriter;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Settings for the summary thread and the sum ring that feeds it.
pub(crate) struct SumSetup {
	pub(crate) writer: SumWriter,
	pub(crate) flush_interval: Duration,
	pub(crate) ring_cap: usize,
}

/// Result of the replay workers, with the summary outcome kept apart from the main capture.
pub(crate) struct ReplayOutcome {
	pub(crate) produced: ProducerStats,
	pub(crate) dispatched: DispatchStats,
	pub(crate) recorded: ConsumerStats,
	pub(crate) summary: Option<Result<SumStats>>,
}

/// Timing and sizing of the replay workers.
pub(crate) struct ReplayOptions {
	pub(crate) speed: ReplaySpeed,
	pub(crate) flush_interval: Duration,
	pub(crate) max_batch: Option<usize>,
	pub(crate) ring_cap: usize,
}

pub(crate) fn run_replay(
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
	let (producer, dispatch_input) = new_spsc_bounded("sim-dispatch", ring_cap)?;
	let (main_producer, consumer) = new_spsc_bounded("dispatch-rec-main", ring_cap)?;
	let (sum_producer, sum_channel) = match sum_setup {
		Some(setup) => {
			let (sum_producer, sum_consumer) =
				new_spsc_bounded::<Arc<Vec<Tick>>>("dispatch-rec-sum", setup.ring_cap)?;
			(Some(sum_producer), Some((sum_consumer, setup.writer, setup.flush_interval)))
		}
		None => (None, None),
	};
	let cancelled = Arc::new(AtomicBool::new(false));
	let sim_done = Arc::new(AtomicBool::new(false));
	let dispatch_done = Arc::new(AtomicBool::new(false));
	let sampler_stop = AtomicBool::new(false);

	thread::scope(|scope| -> Result<_> {
		let producer_cancelled = Arc::clone(&cancelled);
		let producer_finished = Arc::clone(&sim_done);
		let producer_handle = thread::Builder::new().name(SIM_THREAD_NAME.into()).spawn_scoped(scope, move || {
			let mut guard = StopGuard::new(Arc::clone(&producer_cancelled));
			let result = sim::produce(input, producer, speed, &producer_cancelled);
			if result.is_ok() {
				producer_finished.store(true, Ordering::Release);
				guard.disarm();
			}
			result
		})?;

		let dispatch_cancelled = Arc::clone(&cancelled);
		let dispatch_sim_done = Arc::clone(&sim_done);
		let dispatch_finished = Arc::clone(&dispatch_done);
		let dispatch_handle = match thread::Builder::new().name(DISPATCH_THREAD_NAME.into()).spawn_scoped(scope, move || {
			let mut guard = StopGuard::new(Arc::clone(&dispatch_cancelled));
			let result = dispatch(
				dispatch_input,
				main_producer,
				sum_producer,
				&dispatch_sim_done,
				&dispatch_finished,
				&dispatch_cancelled,
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

		let consumer_cancelled = Arc::clone(&cancelled);
		let consumer_finished = Arc::clone(&dispatch_done);
		let consumer_handle = match thread::Builder::new().name(REC_MAIN_THREAD_NAME.into()).spawn_scoped(scope, move || {
			let mut guard = StopGuard::new(Arc::clone(&consumer_cancelled));
			let result = rec_main::consume(
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

		let summary_handle = sum_channel.map(|(sum_consumer, writer, sum_flush)| {
			thread::Builder::new()
				.name(REC_SUM_THREAD_NAME.into())
				.spawn_scoped(scope, move || rec_sum::consume(sum_consumer, writer, sum_flush))
				.map_err(Error::from)
		});

		let sampler_handle = match sampler {
			Some(setup) => {
				let stop = &sampler_stop;
				match thread::Builder::new()
					.name(SAMPLER_THREAD_NAME.into())
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
		let dispatched = dispatch_handle
			.join()
			.map_err(|_| Error::custom("dispatcher thread panicked"))
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
		let (produced, dispatched, recorded) = merge_worker_results(produced, dispatched, recorded)?;
		Ok(ReplayOutcome {
			produced,
			dispatched,
			recorded,
			summary,
		})
	})
}

// region:    --- Support

pub(crate) const REC_SUM_THREAD_NAME: &str = "simrec-rec-sum";
const SIM_THREAD_NAME: &str = "simrec-sim";
const DISPATCH_THREAD_NAME: &str = "simrec-dispatch";
const REC_MAIN_THREAD_NAME: &str = "simrec-rec-main";
const SAMPLER_THREAD_NAME: &str = "simrec-sampler";

#[cfg(test)]
fn replay_workers(
	input: MockInput,
	recorder: Box<dyn Recorder>,
	speed: ReplaySpeed,
	flush_interval: Duration,
	max_batch: Option<usize>,
	ring_cap: usize,
	sampler: Option<SamplerSetup>,
) -> Result<(ProducerStats, ConsumerStats)> {
	let options = ReplayOptions {
		speed,
		flush_interval,
		max_batch,
		ring_cap,
	};
	let outcome = run_replay(input, recorder, options, sampler, None)?;
	Ok((outcome.produced, outcome.recorded))
}

fn is_peer_error(error: &Error) -> bool {
	match error {
		Error::Cancelled => true,
		Error::EventBase(peer_error) => peer_error.is_disconnected(),
		_ => false,
	}
}

fn merge_worker_results(
	produced: Result<ProducerStats>,
	dispatched: Result<DispatchStats>,
	recorded: Result<ConsumerStats>,
) -> Result<(ProducerStats, DispatchStats, ConsumerStats)> {
	match (produced, dispatched, recorded) {
		(Ok(produced), Ok(dispatched), Ok(recorded)) => Ok((produced, dispatched, recorded)),
		(produced, dispatched, recorded) => {
			let mut first_error = None;
			for error in [produced.err(), dispatched.err(), recorded.err()].into_iter().flatten() {
				if !is_peer_error(&error) {
					return Err(error);
				}
				if first_error.is_none() {
					first_error = Some(error);
				}
			}
			Err(first_error.unwrap_or_else(|| Error::custom("worker failed without an error")))
		}
	}
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;

// endregion: --- Tests
