use crate::mock_in::CHANNELS;
use crate::recorder::CHANNEL_COUNT;
use crate::sqlite_common::{create_new_database_file, database_bytes, open_wal_database};
use crate::sum_acc::SumRow;
use crate::{Error, Result};
use rusqlite::{params_from_iter, Connection, ToSql};
use simple_fs::SPath;
use std::sync::LazyLock;

pub const SUM_DB_FILE_NAME: &str = "capture-sum.sqlite";

const STAT_COUNT: usize = 3;
const STAT_SUFFIXES: [&str; STAT_COUNT] = ["min", "max", "avg"];

/// `ts_epoch_micro`, `tick_count`, then `min`, `max` and `avg` per channel.
const PARAM_COUNT: usize = 2 + CHANNEL_COUNT * STAT_COUNT;

static CREATE_TABLE_SQL: LazyLock<String> = LazyLock::new(create_table_sql);
static INSERT_SQL: LazyLock<String> = LazyLock::new(insert_sql);

/// Sink for closed per-second rows, socketed so tests can inject failures and slowness.
pub trait SumWriter: Send {
	fn write_rows(&mut self, rows: &[SumRow]) -> Result<()>;
	fn finish(self: Box<Self>) -> Result<u64>;
}

/// Writer for `capture-sum.sqlite`, one row per epoch second.
pub struct SumSqlite {
	connection: Option<Connection>,
	database_path: SPath,
}

impl SumSqlite {
	pub fn new(directory: &SPath) -> Result<Self> {
		let database_path = directory.join(SUM_DB_FILE_NAME);
		create_new_database_file(&database_path)?;
		let connection = open_wal_database(&database_path)?;
		connection.execute_batch(CREATE_TABLE_SQL.as_str())?;

		Ok(Self {
			connection: Some(connection),
			database_path,
		})
	}
}

impl SumWriter for SumSqlite {
	fn write_rows(&mut self, rows: &[SumRow]) -> Result<()> {
		if rows.is_empty() {
			return Ok(());
		}

		let connection = self
			.connection
			.as_mut()
			.ok_or_else(|| Error::custom("summary SQLite writer is already finalized"))?;
		let transaction = connection.transaction()?;
		{
			let mut statement = transaction.prepare_cached(INSERT_SQL.as_str())?;
			for row in rows {
				let ts_epoch_micro = row.ts_epoch_micro;
				let tick_count = i64::from(row.tick_count);

				let mut values: Vec<&dyn ToSql> = Vec::with_capacity(PARAM_COUNT);
				values.push(&ts_epoch_micro);
				values.push(&tick_count);
				for channel in 0..CHANNEL_COUNT {
					values.push(&row.min[channel]);
					values.push(&row.max[channel]);
					values.push(&row.avg[channel]);
				}

				statement.execute(params_from_iter(values))?;
			}
		}
		transaction.commit()?;

		Ok(())
	}

	fn finish(mut self: Box<Self>) -> Result<u64> {
		let connection = self
			.connection
			.take()
			.ok_or_else(|| Error::custom("summary SQLite writer is already finalized"))?;
		connection.close().map_err(|(_, error)| Error::Sqlite(error))?;

		database_bytes(&self.database_path)
	}
}

// region:    --- Support

/// `sec` table: primary key, tick count, then `min`, `max` and `avg` per channel in channel order.
fn create_table_sql() -> String {
	let mut sql = String::new();
	sql.push_str("CREATE TABLE sec (\n");
	sql.push_str("    ts_epoch_micro INTEGER PRIMARY KEY,\n");
	sql.push_str("    tick_count INTEGER NOT NULL");
	for channel in CHANNELS {
		for suffix in STAT_SUFFIXES {
			sql.push_str(&format!(",\n    {channel}_{suffix} REAL"));
		}
	}
	sql.push_str("\n);");
	sql
}

fn insert_sql() -> String {
	let mut sql = String::from("INSERT INTO sec (ts_epoch_micro, tick_count");
	for channel in CHANNELS {
		for suffix in STAT_SUFFIXES {
			sql.push_str(&format!(", {channel}_{suffix}"));
		}
	}
	sql.push_str(") VALUES (?1, ?2");
	for index in 3..=PARAM_COUNT {
		sql.push_str(&format!(", ?{index}"));
	}
	sql.push(')');
	sql
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

	use super::*;
	use std::fs;

	#[test]
	fn test_sum_sqlite_new_table_info_columns() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_directory("test_sum_sqlite_new_table_info_columns")?;

		// -- Exec
		Box::new(SumSqlite::new(&directory)?).finish()?;
		let connection = Connection::open(directory.join(SUM_DB_FILE_NAME))?;
		let mut statement = connection.prepare("PRAGMA table_info(sec)")?;
		let columns: Vec<String> = statement
			.query_map([], |row| row.get(1))?
			.collect::<rusqlite::Result<Vec<String>>>()?;

		// -- Check
		let mut expected = vec!["ts_epoch_micro".to_string(), "tick_count".to_string()];
		for channel in CHANNELS {
			for suffix in STAT_SUFFIXES {
				expected.push(format!("{channel}_{suffix}"));
			}
		}
		assert_eq!(columns, expected);
		assert_eq!(columns.len(), PARAM_COUNT);
		Ok(())
	}

	#[test]
	fn test_sum_sqlite_write_rows_round_trip() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_directory("test_sum_sqlite_write_rows_round_trip")?;
		let rows = [
			sum_row(1_700_000_000_000_000, 200, 1.0),
			sum_row(1_700_000_001_000_000, 150, 2.0),
		];
		let mut writer = SumSqlite::new(&directory)?;

		// -- Exec
		writer.write_rows(&rows)?;
		let output_bytes = Box::new(writer).finish()?;

		// -- Check
		assert!(output_bytes > 0);
		assert_eq!(read_rows(&directory)?, rows);
		Ok(())
	}

	#[test]
	fn test_sum_sqlite_write_rows_duplicate_rolls_back() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_directory("test_sum_sqlite_write_rows_duplicate_rolls_back")?;
		let ts_epoch_micro = 1_700_000_000_000_000;
		let rows = [sum_row(ts_epoch_micro, 200, 1.0), sum_row(ts_epoch_micro, 10, 5.0)];
		let mut writer = SumSqlite::new(&directory)?;

		// -- Exec
		let result = writer.write_rows(&rows);
		let output_bytes = Box::new(writer).finish()?;

		// -- Check
		assert!(result.is_err());
		assert!(output_bytes > 0);
		assert!(read_rows(&directory)?.is_empty());
		Ok(())
	}

	#[test]
	fn test_sum_sqlite_new_preserves_existing_database() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_directory("test_sum_sqlite_new_preserves_existing_database")?;
		let database_path = directory.join(SUM_DB_FILE_NAME);
		let original = b"existing summary database contents";
		fs::write(&database_path, original)?;

		// -- Exec
		let result = SumSqlite::new(&directory);

		// -- Check
		assert!(result.is_err());
		assert_eq!(fs::read(database_path)?, original);
		Ok(())
	}

	// -- Test Support

	fn read_rows(directory: &SPath) -> Result<Vec<SumRow>> {
		let connection = Connection::open(directory.join(SUM_DB_FILE_NAME))?;
		let mut statement = connection.prepare(&select_sql())?;
		let rows = statement
			.query_map([], |row| {
				let ts_epoch_micro: i64 = row.get(0)?;
				let tick_count: i64 = row.get(1)?;

				let mut min = [0.0_f64; CHANNEL_COUNT];
				let mut max = [0.0_f64; CHANNEL_COUNT];
				let mut avg = [0.0_f64; CHANNEL_COUNT];
				for channel in 0..CHANNEL_COUNT {
					let base = 2 + channel * STAT_COUNT;
					min[channel] = row.get(base)?;
					max[channel] = row.get(base + 1)?;
					avg[channel] = row.get(base + 2)?;
				}

				Ok(SumRow {
					ts_epoch_micro,
					tick_count: tick_count as u32,
					min,
					max,
					avg,
				})
			})?
			.collect::<rusqlite::Result<Vec<SumRow>>>()?;

		Ok(rows)
	}

	fn select_sql() -> String {
		let mut sql = String::from("SELECT ts_epoch_micro, tick_count");
		for channel in CHANNELS {
			for suffix in STAT_SUFFIXES {
				sql.push_str(&format!(", {channel}_{suffix}"));
			}
		}
		sql.push_str(" FROM sec ORDER BY ts_epoch_micro");
		sql
	}

	fn sum_row(ts_epoch_micro: i64, tick_count: u32, base: f64) -> SumRow {
		SumRow {
			ts_epoch_micro,
			tick_count,
			min: core::array::from_fn(|channel| base + channel as f64),
			max: core::array::from_fn(|channel| base + channel as f64 + 1.0),
			avg: core::array::from_fn(|channel| base + channel as f64 + 0.5),
		}
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
