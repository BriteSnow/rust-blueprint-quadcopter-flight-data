use crate::{Error, Result};
use rusqlite::Connection;
use simple_fs::SPath;
use std::fs::{self, OpenOptions};

/// Creates `path` as a new empty file, failing when the file already exists.
pub fn create_new_database_file(path: &SPath) -> Result<()> {
	let file = OpenOptions::new().write(true).create_new(true).open(path)?;
	drop(file);

	Ok(())
}

/// Opens `path` with the WAL journal mode and `synchronous=NORMAL`, reading both back.
pub fn open_wal_database(path: &SPath) -> Result<Connection> {
	let connection = Connection::open(path)?;

	connection.pragma_update(None, "journal_mode", "WAL")?;
	let journal_mode: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
	if !journal_mode.eq_ignore_ascii_case("wal") {
		return Err(Error::custom(format!(
			"SQLite journal mode is {journal_mode}, expected WAL"
		)));
	}

	connection.pragma_update(None, "synchronous", "NORMAL")?;
	let synchronous: i64 = connection.query_row("PRAGMA synchronous", [], |row| row.get(0))?;
	if synchronous != 1 {
		return Err(Error::custom(format!(
			"SQLite synchronous mode is {synchronous}, expected NORMAL"
		)));
	}

	Ok(connection)
}

/// Total bytes of the database file plus any remaining `-wal` and `-shm` files.
pub fn database_bytes(path: &SPath) -> Result<u64> {
	let database_metadata = fs::metadata(path)?;
	if !database_metadata.is_file() {
		return Err(Error::custom(format!(
			"SQLite database {path} is not a regular file"
		)));
	}

	let mut output_bytes = database_metadata.len();
	for suffix in ["-wal", "-shm"] {
		let sidecar_path = SPath::new(format!("{path}{suffix}"));
		match fs::metadata(&sidecar_path) {
			Ok(metadata) if metadata.is_file() => {
				output_bytes = output_bytes
					.checked_add(metadata.len())
					.ok_or_else(|| Error::custom("SQLite recording size overflow"))?;
			}
			Ok(_) => {
				return Err(Error::custom(format!(
					"SQLite sidecar {sidecar_path} is not a regular file"
				)));
			}
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
			Err(error) => return Err(error.into()),
		}
	}

	Ok(output_bytes)
}
