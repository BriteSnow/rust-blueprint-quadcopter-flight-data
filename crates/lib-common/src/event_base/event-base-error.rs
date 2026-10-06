use derive_more::Display;

pub type EventBaseResult<T> = core::result::Result<T, EventBaseError>;

#[non_exhaustive]
#[derive(Debug, Display)]
pub enum EventBaseError {
    #[display("invalid capacity {capacity} for channel `{name}`")]
    InvalidCapacity { name: &'static str, capacity: usize },

    #[display("channel `{name}` receiver disconnected")]
    TxDisconnected { name: &'static str },

    #[display("channel `{name}` sender disconnected")]
    RxDisconnected { name: &'static str },
}

impl EventBaseError {
    /// Whether the failure represents peer shutdown.
    pub fn is_disconnected(&self) -> bool {
        matches!(self, Self::TxDisconnected { .. } | Self::RxDisconnected { .. })
    }
}

impl std::error::Error for EventBaseError {}
