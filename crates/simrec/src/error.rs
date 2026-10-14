use derive_more::{Display, From};
use lib_common::event_base::EventBaseError;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Display, From)]
#[display("{self:?}")]
pub enum Error {
	#[from(String, &String, &str)]
	Custom(String),

	#[display("capture pipeline cancelled after a worker failure")]
	Cancelled,

	#[from]
	EventBase(EventBaseError),

	// -- Externals
	#[from]
	Io(std::io::Error),

	#[from]
	Sqlite(rusqlite::Error),

	#[from]
	Json(serde_json::Error),
}

// region:    --- Custom

impl Error {
	pub fn custom(value: impl Into<String>) -> Self {
		Self::Custom(value.into())
	}

	pub fn custom_from_err(error: impl std::error::Error) -> Self {
		Self::Custom(error.to_string())
	}
}

// endregion: --- Custom

// region:    --- Error Boilerplate

impl std::error::Error for Error {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::EventBase(error) => Some(error),
			Self::Io(error) => Some(error),
			Self::Sqlite(error) => Some(error),
			Self::Json(error) => Some(error),
			Self::Custom(_) | Self::Cancelled => None,
		}
	}
}

// endregion: --- Error Boilerplate
