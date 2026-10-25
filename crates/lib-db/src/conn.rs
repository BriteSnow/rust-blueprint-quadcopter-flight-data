use crate::error::{Error, Result};
use rusqlite::{Connection, OpenFlags};
use std::fs::OpenOptions;
use std::path::Path;
use std::time::Duration;

// region:    --- DbRw

/// Read-write handle, owned by one thread.
pub struct DbRw {
	conn: Connection,
	path: std::path::PathBuf,
}

impl DbRw {
	/// Creates a new database file. Fails if the file already exists, so outputs are never overwritten.
	pub fn create_new(path: impl AsRef<Path>) -> Result<Self> {
		let path = path.as_ref();
		OpenOptions::new().write(true).create_new(true).open(path)?;
		let conn = Connection::open_with_flags(
			path,
			OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
		)?;

		let journal_mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
		if !journal_mode.eq_ignore_ascii_case("wal") {
			return Err(Error::custom(format!("expected journal_mode wal, got {journal_mode}")));
		}

		conn.pragma_update(None, "synchronous", "NORMAL")?;
		let synchronous: i64 = conn.query_row("PRAGMA synchronous", [], |row| row.get(0))?;
		if synchronous != 1 {
			return Err(Error::custom(format!(
				"expected synchronous NORMAL (1), got {synchronous}"
			)));
		}

		Ok(Self {
			conn,
			path: path.to_path_buf(),
		})
	}

	pub fn close(self) -> Result<()> {
		self.conn.close().map_err(|(_, err)| Error::Sqlite(err))
	}

	pub fn path(&self) -> &Path {
		self.path.as_path()
	}

	pub(crate) fn conn(&self) -> &Connection {
		&self.conn
	}

	pub(crate) fn conn_mut(&mut self) -> &mut Connection {
		&mut self.conn
	}
}

// endregion: --- DbRw

// region:    --- DbRo

/// Read-only handle, owned by one thread.
pub struct DbRo {
	conn: Connection,
}

impl DbRo {
	/// Opens an existing database read-only.
	pub fn open(path: impl AsRef<Path>) -> Result<Self> {
		let conn =
			Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
		conn.pragma_update(None, "query_only", "ON")?;
		conn.busy_timeout(Duration::from_millis(1000))?;
		Ok(Self { conn })
	}

	// Used by the summary reader in a later step.
	#[allow(dead_code)]
	pub(crate) fn conn(&self) -> &Connection {
		&self.conn
	}
}

// endregion: --- DbRo

// region:    --- Tests

#[cfg(test)]
mod tests {
	use super::{DbRo, DbRw};
	use crate::_test_support::fixture_dir;

	#[test]
	fn test_conn_db_rw_create_new_existing_file() -> crate::Result<()> {
		let dir = fixture_dir("test_conn_db_rw_create_new_existing_file")?;
		let path = dir.join("capture-main.sqlite");
		std::fs::write(&path, "keep")?;

		assert!(DbRw::create_new(&path).is_err());
		assert_eq!(std::fs::read_to_string(&path)?, "keep");
		Ok(())
	}

	#[test]
	fn test_conn_db_rw_create_new_pragmas() -> crate::Result<()> {
		let dir = fixture_dir("test_conn_db_rw_create_new_pragmas")?;
		let db = DbRw::create_new(dir.join("capture-main.sqlite"))?;

		let journal_mode: String = db.conn().query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
		let synchronous: i64 = db.conn().query_row("PRAGMA synchronous", [], |row| row.get(0))?;

		assert_eq!(journal_mode, "wal");
		assert_eq!(synchronous, 1);
		db.close()?;
		Ok(())
	}

	#[test]
	fn test_conn_db_ro_open_query_only() -> crate::Result<()> {
		let dir = fixture_dir("test_conn_db_ro_open_query_only")?;
		let path = dir.join("capture-main.sqlite");
		let db = DbRw::create_new(&path)?;
		db.conn().execute_batch("CREATE TABLE t (a INTEGER);")?;
		db.close()?;

		let ro = DbRo::open(&path)?;
		let query_only: i64 = ro.conn().query_row("PRAGMA query_only", [], |row| row.get(0))?;
		assert_eq!(query_only, 1);
		assert!(ro.conn().execute("INSERT INTO t (a) VALUES (1)", []).is_err());
		Ok(())
	}
}

// endregion: --- Tests
