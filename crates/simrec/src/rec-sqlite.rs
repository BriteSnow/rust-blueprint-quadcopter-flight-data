use crate::mock_in::CHANNELS;
use crate::recorder::{Recorder, Tick};
use crate::sqlite_common::{create_new_database_file, database_bytes, open_wal_database};
use crate::{Error, Result};
use rusqlite::{params, Connection};
use simple_fs::SPath;

const DATABASE_FILENAME: &str = "capture-main.sqlite";
const INSERT_SQL: &str = "INSERT INTO samples (ts_epoch_micro, rotor_fl_rpm, rotor_fr_rpm, rotor_rl_rpm, rotor_rr_rpm, accel_x_mps2, accel_y_mps2, accel_z_mps2, gyro_x_rads, gyro_y_rads, gyro_z_rads) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)";

pub struct SqliteRecorder {
	connection: Option<Connection>,
	database_path: SPath,
}

impl SqliteRecorder {
	pub fn new(directory: &SPath, with_index: bool) -> Result<Self> {
		let database_path = directory.join(DATABASE_FILENAME);
		create_new_database_file(&database_path)?;
		let connection = open_wal_database(&database_path)?;
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

		connection.execute_batch(
			"CREATE TABLE samples (
                ts_epoch_micro INTEGER PRIMARY KEY,
                rotor_fl_rpm REAL,
                rotor_fr_rpm REAL,
                rotor_rl_rpm REAL,
                rotor_rr_rpm REAL,
                accel_x_mps2 REAL,
                accel_y_mps2 REAL,
                accel_z_mps2 REAL,
                gyro_x_rads REAL,
                gyro_y_rads REAL,
                gyro_z_rads REAL
            );",
		)?;

		if with_index {
			for column in CHANNELS {
				connection.execute_batch(&format!("CREATE INDEX idx_samples_{column} ON samples ({column});"))?;
			}
		}

		Ok(Self {
			connection: Some(connection),
			database_path,
		})
	}

	fn recording_bytes(&self) -> Result<u64> {
		database_bytes(&self.database_path)
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
					tick.epoch_micro,
					f64::from(tick.v[0]),
					f64::from(tick.v[1]),
					f64::from(tick.v[2]),
					f64::from(tick.v[3]),
					f64::from(tick.v[4]),
					f64::from(tick.v[5]),
					f64::from(tick.v[6]),
					f64::from(tick.v[7]),
					f64::from(tick.v[8]),
					f64::from(tick.v[9]),
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
		connection.close().map_err(|(_, error)| Error::Sqlite(error))?;

		self.recording_bytes()
	}
}

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

	use super::*;
	use crate::recorder::CHANNEL_COUNT;
	use std::fs;

	#[test]
	fn test_rec_sqlite_write_batch_and_finish() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_directory("test_rec_sqlite_write_batch_and_finish")?;
		let ticks = [
			Tick {
				epoch_micro: 0,
				v: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0],
			},
			Tick {
				epoch_micro: 5_000,
				v: [-1.0, -2.0, -3.0, -4.0, -5.0, -6.0, -7.0, -8.0, -9.0, -10.0],
			},
		];
		let mut recorder = SqliteRecorder::new(&directory, false)?;
		let connection = recorder.connection.as_ref().ok_or("SQLite connection should be open")?;
		let journal_mode: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
		let synchronous: i64 = connection.query_row("PRAGMA synchronous", [], |row| row.get(0))?;

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
			let (stored_ts, stored): (i64, [f64; CHANNEL_COUNT]) = connection.query_row(
                "SELECT ts_epoch_micro, rotor_fl_rpm, rotor_fr_rpm, rotor_rl_rpm, rotor_rr_rpm, accel_x_mps2, accel_y_mps2, accel_z_mps2, gyro_x_rads, gyro_y_rads, gyro_z_rads FROM samples WHERE ts_epoch_micro = ?1",
                [tick.epoch_micro],
                |row| {
                    let ts_epoch_micro = row.get(0)?;
                    let mut values = [0.0_f64; CHANNEL_COUNT];
                    for (index, value) in values.iter_mut().enumerate() {
                        *value = row.get(index + 1)?;
                    }
                    Ok((ts_epoch_micro, values))
                },
            )?;
			assert_eq!(stored_ts, tick.epoch_micro);
			let stored: [f32; CHANNEL_COUNT] = std::array::from_fn(|index| stored[index] as f32);
			assert_eq!(stored, tick.v);
		}

		Ok(())
	}

	#[test]
	fn test_rec_sqlite_batch_failure_rolls_back_transaction() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_directory("test_rec_sqlite_batch_failure_rolls_back_transaction")?;
		let duplicate_ticks = [
			Tick {
				epoch_micro: 0,
				v: [1.0; CHANNEL_COUNT],
			},
			Tick {
				epoch_micro: 0,
				v: [2.0; CHANNEL_COUNT],
			},
		];
		let mut recorder = SqliteRecorder::new(&directory, false)?;

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
		let directory = fixture_directory("test_rec_sqlite_new_preserves_existing_database")?;
		let database_path = directory.join(DATABASE_FILENAME);
		let original = b"existing database contents";
		fs::write(&database_path, original)?;

		// -- Exec
		let result = SqliteRecorder::new(&directory, false);

		// -- Check
		assert!(result.is_err());
		assert_eq!(fs::read(database_path)?, original);
		Ok(())
	}

	#[test]
	fn test_rec_sqlite_index_is_opt_in() -> Result<()> {
		// -- Setup & Fixtures
		let without_index = fixture_directory("test_rec_sqlite_index_is_opt_in_without")?;
		let with_index = fixture_directory("test_rec_sqlite_index_is_opt_in_with")?;

		// -- Exec
		Box::new(SqliteRecorder::new(&without_index, false)?).finish()?;
		Box::new(SqliteRecorder::new(&with_index, true)?).finish()?;

		// -- Check
		assert_eq!(count_sample_indexes(&without_index)?, 0);
		assert_eq!(count_sample_indexes(&with_index)?, CHANNEL_COUNT as i64);
		Ok(())
	}

	#[test]
	fn test_rec_sqlite_new_table_info_columns() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_directory("test_rec_sqlite_new_table_info_columns")?;

		// -- Exec
		Box::new(SqliteRecorder::new(&directory, false)?).finish()?;
		let connection = Connection::open(directory.join(DATABASE_FILENAME))?;
		let mut statement = connection.prepare("PRAGMA table_info(samples)")?;
		let columns: Vec<String> = statement
			.query_map([], |row| row.get(1))?
			.collect::<rusqlite::Result<Vec<String>>>()?;

		// -- Check
		let mut expected = vec!["ts_epoch_micro".to_string()];
		expected.extend(CHANNELS.map(str::to_string));
		assert_eq!(columns, expected);
		Ok(())
	}

	// -- Test Support

	fn count_sample_indexes(directory: &SPath) -> Result<i64> {
		let connection = Connection::open(directory.join(DATABASE_FILENAME))?;
		let count: i64 = connection.query_row(
			"SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND tbl_name = 'samples'",
			[],
			|row| row.get(0),
		)?;
		Ok(count)
	}

	fn fixture_directory(test_name: &str) -> Result<SPath> {
		let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
		let parent = SPath::new("tests-data/.tmp").join(test_name);
		fs::create_dir_all(&parent)?;
		let directory = parent.join(format!("{}-{unique}", std::process::id()));
		fs::create_dir(&directory)?;
		Ok(directory)
	}
}

// endregion: --- Tests
