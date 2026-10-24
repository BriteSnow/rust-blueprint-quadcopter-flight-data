use derive_more::{Display, From};

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Display, From)]
pub enum Error {
	#[from(String, &String, &str)]
	Custom(String),

	#[from]
	Io(std::io::Error),

	#[from]
	Sqlite(rusqlite::Error),
}

// region:    --- Custom

impl Error {
	pub fn custom(val: impl std::fmt::Display) -> Self {
		Self::Custom(val.to_string())
	}
}

// endregion: --- Custom

impl std::error::Error for Error {}
