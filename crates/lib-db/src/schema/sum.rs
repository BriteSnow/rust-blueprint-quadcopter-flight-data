use super::columns::{insert_sql, summary_column_names};
use crate::conn::{DbRo, DbRw};
use crate::error::{Error, Result};
use lib_common::channels::{CHANNELS, CHANNEL_COUNT};
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Row};
use std::path::Path;
use std::sync::LazyLock;

/// Summary table name.
const SUM_TABLE: &str = "summary";

/// Metadata table name, holds the schema version, bucket width, channels, and start.
const META_TABLE: &str = "meta";

/// Version of the summary schema, written once to `meta.schema_version`.
const SCHEMA_VERSION: i64 = 1;

static INSERT_SQL: LazyLock<String> =
	LazyLock::new(|| insert_sql(SUM_TABLE, &summary_column_names()));

static SELECT_COLUMNS: LazyLock<String> = LazyLock::new(|| summary_column_names().join(", "));

/// One summary bucket. Min, max, and avg are per channel, in tick order.
#[derive(Debug, Clone, PartialEq)]
pub struct SumRow {
	/// Bucket start, in epoch microseconds.
	pub ts_epoch_micro: i64,
	pub tick_count: u32,
	pub min: [f64; CHANNEL_COUNT],
	pub max: [f64; CHANNEL_COUNT],
	pub avg: [f64; CHANNEL_COUNT],
}

/// Creates the summary and metadata tables, and writes the metadata rows.
pub fn create_sum(db: &mut DbRw, bucket_us: i64, start_epoch_us: i64) -> Result<()> {
	let mut ddl = format!(
		"CREATE TABLE {SUM_TABLE} (ts_epoch_micro INTEGER PRIMARY KEY, tick_count INTEGER NOT NULL"
	);
	for channel in CHANNELS {
		for suffix in ["min", "max", "avg"] {
			ddl.push_str(&format!(", {channel}_{suffix} REAL"));
		}
	}
	ddl.push(')');
	let meta_ddl = format!("CREATE TABLE {META_TABLE} (key TEXT PRIMARY KEY, value TEXT NOT NULL)");

	let tx = db.conn_mut().transaction()?;
	tx.execute_batch(&ddl)?;
	tx.execute_batch(&meta_ddl)?;
	{
		let mut stmt = tx.prepare(&format!("INSERT INTO {META_TABLE} (key, value) VALUES (?1, ?2)"))?;
		for (key, value) in [
			("schema_version", SCHEMA_VERSION.to_string()),
			("bucket_us", bucket_us.to_string()),
			("channels", channels_json()),
			("start_epoch_us", start_epoch_us.to_string()),
		] {
			stmt.execute(params![key, value])?;
		}
	}
	tx.commit()?;
	Ok(())
}

pub struct SumWriter {
	db: DbRw,
}

impl SumWriter {
	pub fn new(db: DbRw) -> Self {
		Self { db }
	}

	pub fn path(&self) -> &std::path::Path {
		self.db.path()
	}

	/// Writes one transaction per nonempty slice. A failed slice rolls back as a whole.
	pub fn write_rows(&mut self, rows: &[SumRow]) -> Result<()> {
		if rows.is_empty() {
			return Ok(());
		}
		let tx = self.db.conn_mut().transaction()?;
		{
			let mut stmt = tx.prepare_cached(INSERT_SQL.as_str())?;
			let mut values: Vec<Value> = Vec::with_capacity(2 + 3 * CHANNEL_COUNT);
			for row in rows {
				values.push(Value::Integer(row.ts_epoch_micro));
				values.push(Value::Integer(i64::from(row.tick_count)));
				for ((min, max), avg) in row.min.iter().zip(&row.max).zip(&row.avg) {
					values.push(Value::Real(*min));
					values.push(Value::Real(*max));
					values.push(Value::Real(*avg));
				}
				stmt.execute(params_from_iter(values.drain(..)))?;
			}
		}
		tx.commit()?;
		Ok(())
	}

	pub fn close(self) -> Result<()> {
		self.db.close()
	}
}

/// Read-only view of one summary database, for the console.
pub struct SumReader {
	db: DbRo,
}

impl SumReader {
	/// Opens an existing summary database read-only.
	pub fn open(path: impl AsRef<Path>) -> Result<Self> {
		Ok(Self { db: DbRo::open(path)? })
	}

	pub fn new(db: DbRo) -> Self {
		Self { db }
	}

	/// Bucket width in microseconds, as recorded when the summary was created.
	pub fn bucket_us(&self) -> Result<i64> {
		let value: String = self.db.conn().query_row(
			&format!("SELECT value FROM {META_TABLE} WHERE key = 'bucket_us'"),
			[],
			|row| row.get(0),
		)?;
		value
			.parse::<i64>()
			.map_err(|error| Error::custom(format!("meta.bucket_us is not an integer: {error}")))
	}

	/// Rows strictly after `ts_epoch_micro`, in ascending time order.
	pub fn rows_after(&self, ts_epoch_micro: i64, limit: usize) -> Result<Vec<SumRow>> {
		let sql = format!(
			"SELECT {} FROM {SUM_TABLE} WHERE ts_epoch_micro > ?1 ORDER BY ts_epoch_micro LIMIT ?2",
			SELECT_COLUMNS.as_str()
		);
		let mut stmt = self.db.conn().prepare_cached(&sql)?;
		let rows = stmt
			.query_map(params![ts_epoch_micro, limit_i64(limit)], sum_row_from_sql)?
			.collect::<core::result::Result<Vec<SumRow>, rusqlite::Error>>()?;
		Ok(rows)
	}

	/// The last `n` rows, returned in ascending time order.
	pub fn latest(&self, n: usize) -> Result<Vec<SumRow>> {
		let sql = format!(
			"SELECT {} FROM {SUM_TABLE} ORDER BY ts_epoch_micro DESC LIMIT ?1",
			SELECT_COLUMNS.as_str()
		);
		let mut stmt = self.db.conn().prepare_cached(&sql)?;
		let mut rows = stmt
			.query_map([limit_i64(n)], sum_row_from_sql)?
			.collect::<core::result::Result<Vec<SumRow>, rusqlite::Error>>()?;
		rows.reverse();
		Ok(rows)
	}
}

// region:    --- Support

fn channels_json() -> String {
	let channels = CHANNELS
		.iter()
		.map(|channel| format!("\"{channel}\""))
		.collect::<Vec<_>>()
		.join(",");
	format!("[{channels}]")
}

fn limit_i64(limit: usize) -> i64 {
	i64::try_from(limit).unwrap_or(i64::MAX)
}

fn sum_row_from_sql(row: &Row) -> rusqlite::Result<SumRow> {
	let mut min = [0.0_f64; CHANNEL_COUNT];
	let mut max = [0.0_f64; CHANNEL_COUNT];
	let mut avg = [0.0_f64; CHANNEL_COUNT];
	for channel in 0..CHANNEL_COUNT {
		let base = 2 + channel * 3;
		min[channel] = row.get(base)?;
		max[channel] = row.get(base + 1)?;
		avg[channel] = row.get(base + 2)?;
	}
	Ok(SumRow {
		ts_epoch_micro: row.get(0)?,
		tick_count: row.get(1)?,
		min,
		max,
		avg,
	})
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
	use super::{create_sum, SumReader, SumRow, SumWriter, SUM_TABLE};
	use crate::_test_support::{fixture_dir, table_columns};
	use crate::conn::DbRw;
	use lib_common::channels::{CHANNELS, CHANNEL_COUNT};
	use super::super::columns::summary_column_names;

	const FIXTURE_BUCKET_US: i64 = 500_000;
	const FIXTURE_START_US: i64 = 1_792_022_400_000_000;

	fn sample_row(ts_epoch_micro: i64) -> SumRow {
		SumRow {
			ts_epoch_micro,
			tick_count: 100,
			min: [1.5; CHANNEL_COUNT],
			max: [2.5; CHANNEL_COUNT],
			avg: [2.0; CHANNEL_COUNT],
		}
	}

	#[test]
	fn test_schema_sum_create_sum_table_info_columns() -> crate::Result<()> {
		let dir = fixture_dir("test_schema_sum_create_sum_table_info_columns")?;
		let mut db = DbRw::create_new(dir.join("capture-sum.sqlite"))?;
		create_sum(&mut db, FIXTURE_BUCKET_US, FIXTURE_START_US)?;

		assert_eq!(table_columns(db.conn(), SUM_TABLE)?, summary_column_names());
		db.close()?;
		Ok(())
	}

	#[test]
	fn test_schema_sum_create_sum_meta_values() -> crate::Result<()> {
		// -- Setup & Fixtures
		let dir = fixture_dir("test_schema_sum_create_sum_meta_values")?;
		let path = dir.join("capture-sum.sqlite");
		let mut db = DbRw::create_new(&path)?;

		// -- Exec
		create_sum(&mut db, FIXTURE_BUCKET_US, FIXTURE_START_US)?;
		let start_epoch_us: String = db.conn().query_row(
			"SELECT value FROM meta WHERE key = 'start_epoch_us'",
			[],
			|row| row.get(0),
		)?;
		let channels_json: String =
			db.conn()
				.query_row("SELECT value FROM meta WHERE key = 'channels'", [], |row| {
					row.get(0)
				})?;
		db.close()?;
		let reader = SumReader::open(&path)?;
		let bucket_us = reader.bucket_us()?;
		let channels: Vec<String> = serde_json::from_str(&channels_json)
			.map_err(|error| crate::Error::custom(format!("meta.channels is not JSON: {error}")))?;

		// -- Check
		assert_eq!(bucket_us, FIXTURE_BUCKET_US);
		assert_eq!(start_epoch_us, FIXTURE_START_US.to_string());
		assert_eq!(channels, CHANNELS.map(str::to_string).to_vec());
		Ok(())
	}

	#[test]
	fn test_schema_sum_sum_writer_round_trip() -> crate::Result<()> {
		let dir = fixture_dir("test_schema_sum_sum_writer_round_trip")?;
		let mut db = DbRw::create_new(dir.join("capture-sum.sqlite"))?;
		create_sum(&mut db, FIXTURE_BUCKET_US, FIXTURE_START_US)?;
		let mut writer = SumWriter::new(db);

		writer.write_rows(&[sample_row(1), sample_row(2)])?;

		let sql = format!(
			"SELECT tick_count, rotor_fl_rpm_min, gyro_z_rads_avg FROM {SUM_TABLE} WHERE ts_epoch_micro = 1"
		);
		let (tick_count, min, avg): (i64, f64, f64) =
			writer.db.conn().query_row(&sql, [], |row| {
				Ok((row.get(0)?, row.get(1)?, row.get(2)?))
			})?;
		assert_eq!((tick_count, min, avg), (100, 1.5, 2.0));
		writer.close()?;
		Ok(())
	}

	#[test]
	fn test_schema_sum_sum_writer_duplicate_rolls_back() -> crate::Result<()> {
		let dir = fixture_dir("test_schema_sum_sum_writer_duplicate_rolls_back")?;
		let mut db = DbRw::create_new(dir.join("capture-sum.sqlite"))?;
		create_sum(&mut db, FIXTURE_BUCKET_US, FIXTURE_START_US)?;
		let mut writer = SumWriter::new(db);

		let row = sample_row(1);
		assert!(writer.write_rows(&[row.clone(), row]).is_err());

		let count: i64 = writer.db.conn().query_row(
			&format!("SELECT count(*) FROM {SUM_TABLE}"),
			[],
			|row| row.get(0),
		)?;
		assert_eq!(count, 0);
		Ok(())
	}

	#[test]
	fn test_schema_sum_sum_reader_rows_after_and_latest() -> crate::Result<()> {
		// -- Setup & Fixtures
		let dir = fixture_dir("test_schema_sum_sum_reader_rows_after_and_latest")?;
		let path = dir.join("capture-sum.sqlite");
		let mut db = DbRw::create_new(&path)?;
		create_sum(&mut db, FIXTURE_BUCKET_US, FIXTURE_START_US)?;
		let mut writer = SumWriter::new(db);
		writer.write_rows(&[
			sample_row(1),
			sample_row(2),
			sample_row(3),
			sample_row(4),
			sample_row(5),
		])?;
		writer.close()?;
		let reader = SumReader::open(&path)?;

		// -- Exec
		let after = reader.rows_after(2, 10)?;
		let latest = reader.latest(2)?;

		// -- Check
		let after_keys: Vec<i64> = after.iter().map(|row| row.ts_epoch_micro).collect();
		let latest_keys: Vec<i64> = latest.iter().map(|row| row.ts_epoch_micro).collect();
		assert_eq!(after_keys, vec![3, 4, 5]);
		assert_eq!(latest_keys, vec![4, 5]);
		Ok(())
	}
}

// endregion: --- Tests
