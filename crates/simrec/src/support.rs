use crate::{Error, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub struct StopGuard {
	cancelled: Arc<AtomicBool>,
	armed: bool,
}

impl StopGuard {
	pub fn new(cancelled: Arc<AtomicBool>) -> Self {
		Self { cancelled, armed: true }
	}

	pub fn disarm(&mut self) {
		self.armed = false;
	}
}

impl Drop for StopGuard {
	fn drop(&mut self) {
		if self.armed {
			self.cancelled.store(true, Ordering::Release);
		}
	}
}

pub fn check_cancelled(cancelled: &AtomicBool) -> Result<()> {
	if cancelled.load(Ordering::Acquire) {
		Err(Error::Cancelled)
	} else {
		Ok(())
	}
}

pub fn wait_until(deadline: Instant, cancelled: &AtomicBool) -> Result<()> {
	loop {
		check_cancelled(cancelled)?;
		let remaining = deadline.saturating_duration_since(Instant::now());
		if remaining.is_zero() {
			return Ok(());
		}
		thread::sleep(remaining.min(Duration::from_millis(1)));
	}
}

pub fn idle_wait(cancelled: &AtomicBool) -> Result<()> {
	check_cancelled(cancelled)?;
	thread::sleep(Duration::from_micros(100));
	Ok(())
}

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

	use super::*;

	#[test]
	fn test_support_stop_guard_drop_cancels() -> Result<()> {
		// -- Setup & Fixtures
		let cancelled = Arc::new(AtomicBool::new(false));
		let guard = StopGuard::new(Arc::clone(&cancelled));

		// -- Exec
		drop(guard);

		// -- Check
		assert!(cancelled.load(Ordering::Acquire));
		assert!(matches!(check_cancelled(&cancelled), Err(Error::Cancelled)));
		Ok(())
	}

	#[test]
	fn test_support_stop_guard_disarm_preserves_success() -> Result<()> {
		// -- Setup & Fixtures
		let cancelled = Arc::new(AtomicBool::new(false));
		let mut guard = StopGuard::new(Arc::clone(&cancelled));

		// -- Exec
		guard.disarm();
		drop(guard);

		// -- Check
		assert!(!cancelled.load(Ordering::Acquire));
		check_cancelled(&cancelled)?;
		Ok(())
	}

	#[test]
	fn test_support_stop_guard_panic_cancels() -> Result<()> {
		// -- Setup & Fixtures
		let cancelled = Arc::new(AtomicBool::new(false));
		let worker_cancelled = Arc::clone(&cancelled);

		// -- Exec
		let result = std::panic::catch_unwind(move || {
			let _guard = StopGuard::new(worker_cancelled);
			panic!("injected worker panic");
		});

		// -- Check
		assert!(result.is_err());
		assert!(cancelled.load(Ordering::Acquire));
		Ok(())
	}

	#[test]
	fn test_support_wait_until_cancelled() -> Result<()> {
		// -- Setup & Fixtures
		let cancelled = AtomicBool::new(true);
		let deadline = Instant::now()
			.checked_add(Duration::from_secs(60))
			.ok_or("test deadline exceeds clock range")?;

		// -- Exec
		let result = wait_until(deadline, &cancelled);

		// -- Check
		assert!(matches!(result, Err(Error::Cancelled)));
		Ok(())
	}

	#[test]
	fn test_support_wait_until_elapsed() -> Result<()> {
		// -- Setup & Fixtures
		let cancelled = AtomicBool::new(false);
		let deadline = Instant::now();

		// -- Exec
		let result = wait_until(deadline, &cancelled);

		// -- Check
		result?;
		assert!(!cancelled.load(Ordering::Acquire));
		Ok(())
	}

	#[test]
	fn test_support_idle_wait_cancelled() -> Result<()> {
		// -- Setup & Fixtures
		let cancelled = AtomicBool::new(true);

		// -- Exec
		let result = idle_wait(&cancelled);

		// -- Check
		assert!(matches!(result, Err(Error::Cancelled)));
		Ok(())
	}
}

// endregion: --- Tests
