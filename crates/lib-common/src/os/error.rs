use derive_more::{Display, From};

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Display, From)]
#[display("{self:?}")]
pub enum Error {
	#[from(String, &String, &str)]
	Custom(String),

	#[display("cpu sampling is not supported on this platform")]
	Unsupported,

	#[display("{call} failed: {message}")]
	ProcInfo { call: &'static str, message: String },
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

impl std::error::Error for Error {}

// endregion: --- Error Boilerplate
