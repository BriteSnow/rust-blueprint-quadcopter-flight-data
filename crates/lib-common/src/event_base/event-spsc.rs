use super::{EventBaseError, EventBaseResult};
use crossfire::{spsc, Rx, TryRecvError, TrySendError, Tx};

/// Unique synchronous producer. Cloning is deliberately not supported.
pub struct SyncSpscTx<T: Send + 'static> {
	inner: Tx<spsc::Array<T>>,
	name: &'static str,
}

/// Unique synchronous consumer. Cloning is deliberately not supported.
pub struct SyncSpscRx<T: Send + 'static> {
	inner: Rx<spsc::Array<T>>,
	name: &'static str,
}

impl<T: Send + 'static> std::fmt::Debug for SyncSpscTx<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("SyncSpscTx").field("name", &self.name).finish()
	}
}

impl<T: Send + 'static> std::fmt::Debug for SyncSpscRx<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("SyncSpscRx").field("name", &self.name).finish()
	}
}

/// Creates a named ring with positive message capacity.
///
/// Uses Crossfire's synchronous SPSC channel without exposing backend handles
/// or errors to application code.
pub fn new_spsc_bounded<T>(name: &'static str, capacity: usize) -> EventBaseResult<(SyncSpscTx<T>, SyncSpscRx<T>)>
where
	T: Send + 'static,
{
	if capacity == 0 {
		return Err(EventBaseError::InvalidCapacity { name, capacity });
	}
	let (tx, rx) = spsc::bounded_async::<T>(capacity);
	Ok((
		SyncSpscTx {
			inner: tx.into_blocking(),
			name,
		},
		SyncSpscRx {
			inner: rx.into_blocking(),
			name,
		},
	))
}

impl<T: Send + 'static> SyncSpscTx<T> {
	pub fn name(&self) -> &'static str {
		self.name
	}

	pub fn is_disconnected(&self) -> bool {
		self.inner.is_disconnected()
	}

	/// Attempts to enqueue without blocking.
	///
	/// Returns `None` on success or the owned message when full. A disconnected
	/// receiver returns an error without recovering the message. Success means
	/// enqueued, not acknowledged; peer shutdown can race with this operation.
	pub fn try_send(&mut self, message: T) -> EventBaseResult<Option<T>> {
		match self.inner.try_send(message) {
			Ok(()) => Ok(None),
			Err(TrySendError::Full(message)) => Ok(Some(message)),
			Err(TrySendError::Disconnected(_)) => Err(EventBaseError::TxDisconnected { name: self.name }),
		}
	}
}

impl<T: Send + 'static> SyncSpscRx<T> {
	pub fn name(&self) -> &'static str {
		self.name
	}

	pub fn is_disconnected(&self) -> bool {
		self.inner.is_disconnected()
	}

	/// Returns a currently available message or `None` while empty and connected.
	///
	/// Queued messages are drained before reporting sender disconnection.
	pub fn try_recv(&mut self) -> EventBaseResult<Option<T>> {
		match self.inner.try_recv() {
			Ok(message) => Ok(Some(message)),
			Err(TryRecvError::Empty) => Ok(None),
			Err(TryRecvError::Disconnected) => {
				// Recheck after observing shutdown so a final publication is not lost.
				match self.inner.try_recv() {
					Ok(message) => Ok(Some(message)),
					Err(TryRecvError::Empty) => Ok(None),
					Err(TryRecvError::Disconnected) => Err(EventBaseError::RxDisconnected { name: self.name }),
				}
			}
		}
	}
}

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

	use super::*;

	#[test]
	fn test_event_base_spsc_try_operations_simple() -> Result<()> {
		// -- Setup & Fixtures
		let (mut tx, mut rx) = new_spsc_bounded("spsc-test", 1)?;

		// -- Exec
		let sent = tx.try_send(String::from("first"))?;
		let retained = tx.try_send(String::from("second"))?;
		let received = rx.try_recv()?;
		let empty = rx.try_recv()?;

		// -- Check
		assert!(sent.is_none());
		assert_eq!(retained.as_deref(), Some("second"));
		assert_eq!(received.as_deref(), Some("first"));
		assert!(empty.is_none());
		assert_eq!(tx.name(), "spsc-test");
		assert_eq!(rx.name(), "spsc-test");
		Ok(())
	}

	#[test]
	fn test_event_base_spsc_try_recv_final_drain() -> Result<()> {
		// -- Setup & Fixtures
		let (mut tx, mut rx) = new_spsc_bounded("spsc-drain", 1)?;
		assert!(tx.try_send(42)?.is_none());
		drop(tx);

		// -- Exec
		let received = rx.try_recv()?;
		let end = rx.try_recv();

		// -- Check
		assert_eq!(received, Some(42));
		assert!(rx.is_disconnected());
		assert!(matches!(
			end,
			Err(EventBaseError::RxDisconnected { name: "spsc-drain" })
		));
		Ok(())
	}

	#[test]
	fn test_event_base_spsc_try_send_disconnected() -> Result<()> {
		// -- Setup & Fixtures
		let (mut tx, rx) = new_spsc_bounded::<u32>("spsc-closed", 1)?;
		drop(rx);

		// -- Exec
		let result = tx.try_send(42);

		// -- Check
		assert!(tx.is_disconnected());
		assert!(matches!(
			result,
			Err(EventBaseError::TxDisconnected { name: "spsc-closed" })
		));
		Ok(())
	}

	#[test]
	fn test_event_base_spsc_new_invalid_capacity() -> Result<()> {
		// -- Setup & Fixtures
		let capacity = 0;

		// -- Exec
		let result = new_spsc_bounded::<u32>("spsc-invalid", capacity);

		// -- Check
		assert!(matches!(
			result,
			Err(EventBaseError::InvalidCapacity {
				name: "spsc-invalid",
				capacity: 0
			})
		));
		Ok(())
	}
}

// endregion: --- Tests
