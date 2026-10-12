use std::fs::{self, OpenOptions};

use rusqlite::{params, Connection};
use simple_fs::SPath;

use crate::{
    recorder::{Recorder, Tick},
    Error, Result,
};

const DATABASE_FILENAME: &str = "capture.sqlite";
const INSERT_SQL: &str = "INSERT INTO samples (ts_ns, rotor_fl, rotor_fr, rotor_rl, rotor_rr, accel_z) VALUES (?1, ?2, ?3, ?4, ?5, ?6)";

pub struct SqliteRecorder {
    connection: Option<Connection>,
    database_path: SPath,
}

impl SqliteRecorder {
    pub fn new(directory: &SPath) -> Result<Self> {
        let database_path = directory.join(DATABASE_FILENAME);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&database_path)?;
        drop(file);

        let connection = Connection::open(&database_path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        let journal_mode: String =
            connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(Error::custom(format!(
                "SQLite journal mode is {journal_mode}, expected WAL"
            )));
        }

        connection.pragma_update(None, "synchronous", "NORMAL")?;
        let synchronous: i64 =
            connection.query_row("PRAGMA synchronous", [], |row| row.get(0))?;
        if synchronous != 1 {
            return Err(Error::custom(format!(
                "SQLite synchronous mode is {synchronous}, expected NORMAL"
            )));
        }

        connection.execute_batch(
            "CREATE TABLE samples (
                ts_ns INTEGER PRIMARY KEY,
                rotor_fl REAL,
                rotor_fr REAL,
                rotor_rl REAL,
                rotor_rr REAL,
                accel_z REAL
            );",
        )?;

        Ok(Self {
            connection: Some(connection),
            database_path,
        })
    }

    fn recording_bytes(&self) -> Result<u64> {
        let database_metadata = fs::metadata(&self.database_path)?;
        if !database_metadata.is_file() {
            return Err(Error::custom(format!(
                "SQLite database {} is not a regular file",
                self.database_path
            )));
        }

        let mut output_bytes = database_metadata.len();
        for suffix in ["-wal", "-shm"] {
            let sidecar_path = SPath::new(format!("{}{suffix}", self.database_path));
            match fs::metadata(&sidecar_path) {
                Ok(metadata) if metadata.is_file() => {
                    output_bytes = output_bytes
                        .checked_add(metadata.len())
                        .ok_or_else(|| Error::custom("SQLite recording size overflow"))?;
                }
                Ok(_) => {
                    return Err(Error::custom(format!(
                        "SQLite sidecar {} is not a regular file",
                        sidecar_path
                    )));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }

        Ok(output_bytes)
    }
}

impl Recorder for SqliteRecorder {
    fn write_batch(&mut self, ticks: &[Tick]) -> Result<()> {
        if ticks.is_empty() {
            return Ok(());
        }

        let connection = self
            .connection
            .as_mut()
            .ok_or_else(|| Error::custom("SQLite recorder is already finalized"))?;
        let transaction = connection.transaction()?;
        {
            let mut statement = transaction.prepare_cached(INSERT_SQL)?;
            for tick in ticks {
                statement.execute(params![
                    tick.ts_ns,
                    f64::from(tick.v[0]),
                    f64::from(tick.v[1]),
                    f64::from(tick.v[2]),
                    f64::from(tick.v[3]),
                    f64::from(tick.v[4]),
                ])?;
            }
        }
        transaction.commit()?;

        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<u64> {
        let connection = self
            .connection
            .take()
            .ok_or_else(|| Error::custom("SQLite recorder is already finalized"))?;
        connection
            .close()
            .map_err(|(_, error)| Error::Sqlite(error))?;

        self.recording_bytes()
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    use super::*;

    #[test]
    fn test_rec_sqlite_write_batch_and_finish() -> Result<()> {
        // -- Setup & Fixtures
        let directory = fixture_directory("test_rec_sqlite_write_batch_and_finish")?;
        let ticks = [
            Tick {
                ts_ns: 0,
                v: [1.0, 2.0, 3.0, 4.0, 9.81],
            },
            Tick {
                ts_ns: 5_000_000,
                v: [-1.0, -2.0, -3.0, -4.0, 8.5],
            },
        ];
        let mut recorder = SqliteRecorder::new(&directory)?;
        let connection = recorder
            .connection
            .as_ref()
            .ok_or("SQLite connection should be open")?;
        let journal_mode: String =
            connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        let synchronous: i64 =
            connection.query_row("PRAGMA synchronous", [], |row| row.get(0))?;

        // -- Exec
        recorder.write_batch(&ticks)?;
        let output_bytes = Box::new(recorder).finish()?;

        // -- Check
        assert!(journal_mode.eq_ignore_ascii_case("wal"));
        assert_eq!(synchronous, 1);
        assert!(output_bytes > 0);

        let connection = Connection::open(directory.join(DATABASE_FILENAME))?;
        let count: i64 = connection.query_row("SELECT COUNT(*) FROM samples", [], |row| row.get(0))?;
        assert_eq!(count, ticks.len() as i64);

        for tick in ticks {
            let stored: (i64, f64, f64, f64, f64, f64) = connection.query_row(
                "SELECT ts_ns, rotor_fl, rotor_fr, rotor_rl, rotor_rr, accel_z FROM samples WHERE ts_ns = ?1",
                [tick.ts_ns],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )?;
            assert_eq!(stored.0, tick.ts_ns);
            assert_eq!(
                [
                    stored.1 as f32,
                    stored.2 as f32,
                    stored.3 as f32,
                    stored.4 as f32,
                    stored.5 as f32,
                ],
                tick.v
            );
        }

        Ok(())
    }

    #[test]
    fn test_rec_sqlite_batch_failure_rolls_back_transaction() -> Result<()> {
        // -- Setup & Fixtures
        let directory =
            fixture_directory("test_rec_sqlite_batch_failure_rolls_back_transaction")?;
        let duplicate_ticks = [
            Tick {
                ts_ns: 0,
                v: [1.0; 5],
            },
            Tick {
                ts_ns: 0,
                v: [2.0; 5],
            },
        ];
        let mut recorder = SqliteRecorder::new(&directory)?;

        // -- Exec
        let result = recorder.write_batch(&duplicate_ticks);
        let output_bytes = Box::new(recorder).finish()?;

        // -- Check
        assert!(result.is_err());
        assert!(output_bytes > 0);

        let connection = Connection::open(directory.join(DATABASE_FILENAME))?;
        let count: i64 = connection.query_row("SELECT COUNT(*) FROM samples", [], |row| row.get(0))?;
        assert_eq!(count, 0);
        Ok(())
    }

    #[test]
    fn test_rec_sqlite_new_preserves_existing_database() -> Result<()> {
        // -- Setup & Fixtures
        let directory =
            fixture_directory("test_rec_sqlite_new_preserves_existing_database")?;
        let database_path = directory.join(DATABASE_FILENAME);
        let original = b"existing database contents";
        fs::write(&database_path, original)?;

        // -- Exec
        let result = SqliteRecorder::new(&directory);

        // -- Check
        assert!(result.is_err());
        assert_eq!(fs::read(database_path)?, original);
        Ok(())
    }

    // -- Test Support

    fn fixture_directory(test_name: &str) -> Result<SPath> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let parent = SPath::new("tests-data/.tmp").join(test_name);
        fs::create_dir_all(&parent)?;
        let directory = parent.join(format!("{}-{unique}", std::process::id()));
        fs::create_dir(&directory)?;
        Ok(directory)
    }
}

// endregion: --- Tests
