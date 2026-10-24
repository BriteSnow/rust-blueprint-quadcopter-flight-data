// region:    --- Modules

mod conn;
mod error;
mod files;
mod schema;

#[cfg(test)]
mod _test_support;

pub use conn::{DbRo, DbRw};
pub use error::{Error, Result};
pub use files::db_file_bytes;
pub use schema::{create_main, create_sum, SamplesWriter, SumReader, SumRow, SumWriter};

// endregion: --- Modules
