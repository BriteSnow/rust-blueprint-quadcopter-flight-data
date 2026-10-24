use super::recorder::Recorder;
use crate::Result;
use lib_common::Tick;
use lib_db::{create_main, db_file_bytes, DbRw, SamplesWriter};
use simple_fs::SPath;

const DATABASE_FILENAME: &str = "capture-main.sqlite";
const INSERT_SQL: &str = "INSERT INTO samples (ts_epoch_micro, rotor_fl_rpm, rotor_fr_rpm, rotor_rl_rpm, rotor_rr_rpm, accel_x_mps2, accel_y_mps2, accel_z_mps2, gyro_x_rads, gyro_y_rads, gyro_z_rads) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)";
pub struct SqliteRecorder {
	writer: SamplesWriter,
	database_path: SPath,
}

impl SqliteRecorder {
	pub fn new(directory: &SPath, with_index: bool) -> Result<Self> {
		let database_path = directory.join(DATABASE_FILENAME);
		let db = DbRw::create_new(&database_path)?;
		create_main(&db, with_index)?;

		Ok(Self {
			writer: SamplesWriter::new(db),
			database_path,
		})
	}
}

impl Recorder for SqliteRecorder {
	fn write_batch(&mut self, ticks: &[Tick]) -> Result<()> {
		self.writer.write_batch(ticks)?;
		Ok(())
	}

	fn finish(self: Box<Self>) -> Result<u64> {
		let Self { writer, database_path } = *self;
		writer.close()?;

		Ok(db_file_bytes(&database_path)?)
	}
}
// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

	use super::*;
	use lib_common::channels::{CHANNEL_COUNT, CHANNELS};
	use rusqlite::Connection;
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

		// -- Exec
		recorder.write_batch(&ticks)?;
		let output_bytes = Box::new(recorder).finish()?;

		// -- Check
		assert!(output_bytes > 0);

		let connection = Connection::open(directory.join(DATABASE_FILENAME))?;
		let journal_mode: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
		assert!(journal_mode.eq_ignore_ascii_case("wal"));
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
