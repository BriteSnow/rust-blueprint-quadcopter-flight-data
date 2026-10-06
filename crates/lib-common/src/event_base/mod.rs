//! Named, bounded synchronous SPSC endpoints.
//!
//! Backend handles and errors remain private. Each endpoint has one owner,
//! and a full queue returns the unsent message without blocking.

// region:    --- Modules

#[path = "event-base-error.rs"]
mod event_base_error;
#[path = "event-spsc.rs"]
mod event_spsc;

pub use event_base_error::{EventBaseError, EventBaseResult};
pub use event_spsc::{new_spsc_bounded, SyncSpscRx, SyncSpscTx};

// endregion: --- Modules
