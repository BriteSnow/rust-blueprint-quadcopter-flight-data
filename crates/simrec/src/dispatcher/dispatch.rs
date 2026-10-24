use crate::support::{check_cancelled, idle_wait};
use crate::Result;
use lib_common::event_base::{SyncSpscRx, SyncSpscTx};
use lib_common::Tick;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Default, Clone, Copy)]
pub struct DispatchStats {
	pub ticks_total: u64,
	/// Batches that found the main ring full at least once.
	pub main_stall_count: u64,
	/// Batches the summary ring refused or that arrived after it disconnected.
	pub sum_batches_dropped: u64,
	/// Ticks carried by those dropped summary batches.
	pub sum_ticks_dropped: u64,
}

/// Forwards every sim batch to the main recorder ring, in order.
///
/// Stops only after `sim_done` is set and the sim ring is drained. A plain
/// sender disconnect is not success, because sim also drops its sender on
/// failure. Sets `dispatch_done` before returning so the recorder can finish.
pub fn dispatch(
	mut input: SyncSpscRx<Vec<Tick>>,
	mut main: SyncSpscTx<Arc<Vec<Tick>>>,
	sum: Option<SyncSpscTx<Arc<Vec<Tick>>>>,
	sim_done: &AtomicBool,
	dispatch_done: &AtomicBool,
	cancelled: &AtomicBool,
) -> Result<DispatchStats> {
	let mut stats = DispatchStats::default();
	let mut sum = sum;

	loop {
		check_cancelled(cancelled)?;
		let message = match input.try_recv() {
			Ok(message) => message,
			Err(error) if error.is_disconnected() => None,
			Err(error) => return Err(error.into()),
		};
		match message {
			Some(batch) => forward(&mut main, &mut sum, batch, &mut stats, cancelled)?,
			None => {
				if sim_done.load(Ordering::Acquire) {
					// Recheck after acquiring completion so the final push cannot be missed.
					match input.try_recv() {
						Ok(Some(batch)) => {
							forward(&mut main, &mut sum, batch, &mut stats, cancelled)?;
							continue;
						}
						Ok(None) => break,
						Err(error) if error.is_disconnected() => break,
						Err(error) => return Err(error.into()),
					}
				}
				idle_wait(cancelled)?;
			}
		}
	}

	dispatch_done.store(true, Ordering::Release);
	Ok(stats)
}

// region:    --- Support

/// Sends one batch to main, then offers the same allocation to sum.
fn forward(
	main: &mut SyncSpscTx<Arc<Vec<Tick>>>,
	sum: &mut Option<SyncSpscTx<Arc<Vec<Tick>>>>,
	batch: Vec<Tick>,
	stats: &mut DispatchStats,
	cancelled: &AtomicBool,
) -> Result<()> {
	stats.ticks_total += batch.len() as u64;
	let mut message = Arc::new(batch);
	let mut stalled = false;
	loop {
		match main.try_send(Arc::clone(&message))? {
			None => break,
			Some(returned) => {
				if !stalled {
					stats.main_stall_count += 1;
					stalled = true;
				}
				message = returned;
				idle_wait(cancelled)?;
			}
		}
	}

	forward_sum(sum, message, stats);
	Ok(())
}

/// Best effort: a full ring, or a disconnected ring, drops the whole batch.
fn forward_sum(
	sum: &mut Option<SyncSpscTx<Arc<Vec<Tick>>>>,
	message: Arc<Vec<Tick>>,
	stats: &mut DispatchStats,
) {
	let Some(sender) = sum.as_mut() else {
		return;
	};

	let batch_ticks = message.len() as u64;
	match sender.try_send(Arc::clone(&message)) {
		Ok(None) => {}
		Ok(Some(_)) => {
			stats.sum_batches_dropped += 1;
			stats.sum_ticks_dropped += batch_ticks;
		}
		Err(_) => {
			// Disconnection drops the message. Stop sending, and count every later batch.
			stats.sum_batches_dropped += 1;
			stats.sum_ticks_dropped += batch_ticks;
			*sum = None;
		}
	}
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

	use super::*;
	use crate::Error;
	use lib_common::channels::CHANNEL_COUNT;
	use lib_common::event_base::new_spsc_bounded;
	use std::thread;
	use std::time::Duration;

	#[test]
	fn test_dispatch_dispatch_forwards_in_order() -> Result<()> {
		// -- Setup & Fixtures
		let (mut sim_tx, sim_rx) = new_spsc_bounded("sim-dispatch-test", 4)?;
		let (main_tx, mut main_rx) = new_spsc_bounded("dispatch-rec-main-test", 4)?;
		for batch_index in 0..3_i64 {
			let ticks = vec![tick(batch_index * 10), tick(batch_index * 10 + 1)];
			if sim_tx.try_send(ticks)?.is_some() {
				return Err("fixture ring unexpectedly full".into());
			}
		}
		drop(sim_tx);
		let sim_done = AtomicBool::new(true);
		let dispatch_done = AtomicBool::new(false);
		let cancelled = AtomicBool::new(false);

		// -- Exec
		let stats = dispatch(sim_rx, main_tx, None, &sim_done, &dispatch_done, &cancelled)?;

		// -- Check
		let mut epochs = Vec::new();
		while let Ok(Some(batch)) = main_rx.try_recv() {
			epochs.extend(batch.iter().map(|tick| tick.epoch_micro));
		}
		assert_eq!(epochs, vec![0, 1, 10, 11, 20, 21]);
		assert_eq!(stats.ticks_total, 6);
		assert!(dispatch_done.load(Ordering::Acquire));
		Ok(())
	}

	#[test]
	fn test_dispatch_dispatch_sim_failure_not_success() -> Result<()> {
		// -- Setup & Fixtures
		let (sim_tx, sim_rx) = new_spsc_bounded::<Vec<Tick>>("sim-dispatch-fail", 4)?;
		let (main_tx, _main_rx) = new_spsc_bounded::<Arc<Vec<Tick>>>("dispatch-rec-main-fail", 4)?;
		drop(sim_tx);
		let sim_done = AtomicBool::new(false);
		let dispatch_done = AtomicBool::new(false);
		let cancelled = AtomicBool::new(false);

		// -- Exec
		let result = thread::scope(|scope| {
			scope.spawn(|| {
				thread::sleep(Duration::from_millis(20));
				cancelled.store(true, Ordering::Release);
			});
			dispatch(sim_rx, main_tx, None, &sim_done, &dispatch_done, &cancelled)
		});

		// -- Check
		assert!(matches!(result, Err(Error::Cancelled)));
		assert!(!dispatch_done.load(Ordering::Acquire));
		Ok(())
	}

	#[test]
	fn test_dispatch_dispatch_main_stall_counted() -> Result<()> {
		// -- Setup & Fixtures
		let (mut sim_tx, sim_rx) = new_spsc_bounded("sim-dispatch-stall", 4)?;
		let (main_tx, mut main_rx) = new_spsc_bounded("dispatch-rec-main-stall", 1)?;
		for batch_index in 0..2_i64 {
			if sim_tx.try_send(vec![tick(batch_index)])?.is_some() {
				return Err("fixture ring unexpectedly full".into());
			}
		}
		drop(sim_tx);
		let sim_done = AtomicBool::new(true);
		let dispatch_done = AtomicBool::new(false);
		let cancelled = AtomicBool::new(false);

		// -- Exec
		let (stats, received) = thread::scope(|scope| -> Result<(DispatchStats, usize)> {
			let drain = scope.spawn(move || {
				let mut received = 0_usize;
				loop {
					thread::sleep(Duration::from_millis(5));
					match main_rx.try_recv() {
						Ok(Some(_)) => received += 1,
						Ok(None) => {}
						Err(_) => break,
					}
				}
				received
			});
			let stats = dispatch(sim_rx, main_tx, None, &sim_done, &dispatch_done, &cancelled)?;
			let received = drain.join().map_err(|_| "drain thread panicked")?;
			Ok((stats, received))
		})?;

		// -- Check
		assert_eq!(received, 2);
		assert_eq!(stats.ticks_total, 2);
		assert!(stats.main_stall_count >= 1);
		assert!(stats.main_stall_count <= 2);
		Ok(())
	}

	#[test]
	fn test_dispatch_dispatch_full_sum_ring_drops() -> Result<()> {
		// -- Setup & Fixtures
		let (mut sim_tx, sim_rx) = new_spsc_bounded("sim-dispatch-sum-full", 8)?;
		let (main_tx, mut main_rx) = new_spsc_bounded::<Arc<Vec<Tick>>>("dispatch-rec-main-sum-full", 8)?;
		let (sum_tx, mut sum_rx) = new_spsc_bounded("dispatch-rec-sum-full", 1)?;
		for batch_index in 0..4_i64 {
			if sim_tx.try_send(vec![tick(batch_index)])?.is_some() {
				return Err("fixture ring unexpectedly full".into());
			}
		}
		drop(sim_tx);
		let sim_done = AtomicBool::new(true);
		let dispatch_done = AtomicBool::new(false);
		let cancelled = AtomicBool::new(false);

		// -- Exec
		let (stats, main_ticks, sum_ticks) = thread::scope(|scope| -> Result<(DispatchStats, usize, u64)> {
			let drain = scope.spawn(move || {
				let mut received = 0_usize;
				loop {
					thread::sleep(Duration::from_millis(2));
					match main_rx.try_recv() {
						Ok(Some(batch)) => received += batch.len(),
						Ok(None) => {}
						Err(_) => break,
					}
				}
				received
			});
			let stats = dispatch(sim_rx, main_tx, Some(sum_tx), &sim_done, &dispatch_done, &cancelled)?;
			let main_ticks = drain.join().map_err(|_| "drain thread panicked")?;
			let mut sum_ticks = 0_u64;
			loop {
				match sum_rx.try_recv() {
					Ok(Some(batch)) => sum_ticks += batch.len() as u64,
					Ok(None) => break,
					Err(error) if error.is_disconnected() => break,
					Err(error) => return Err(error.into()),
				}
			}
			Ok((stats, main_ticks, sum_ticks))
		})?;

		// -- Check
		assert_eq!(main_ticks, 4);
		assert!(stats.sum_batches_dropped > 0);
		assert_eq!(sum_ticks + stats.sum_ticks_dropped, stats.ticks_total);
		Ok(())
	}

	// -- Test Support

	fn tick(epoch_micro: i64) -> Tick {
		Tick {
			epoch_micro,
			v: [1.0; CHANNEL_COUNT],
		}
	}
}

// endregion: --- Tests
