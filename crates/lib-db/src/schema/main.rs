use super::columns::samples_insert_sql;
use crate::conn::DbRw;
use crate::error::Result;
use lib_common::channels::CHANNELS;
use lib_common::Tick;
use rusqlite::params_from_iter;
use rusqlite::types::Value;
use std::sync::LazyLock;

static INSERT_SQL: LazyLock<String> = LazyLock::new(samples_insert_sql);

/// Creates the `samples` table, plus one index per channel when `all_index` is set.
pub fn create_main(db: &DbRw, all_index: bool) -> Result<()> {
	let mut ddl = String::from("CREATE TABLE samples (ts_epoch_micro INTEGER PRIMARY KEY");
	for channel in CHANNELS {
		ddl.push_str(&format!(", {channel} REAL"));
	}
	ddl.push_str(");");
	if all_index {
		for channel in CHANNELS {
			ddl.push_str(&format!("CREATE INDEX idx_samples_{channel} ON samples ({channel});"));
		}
	}
	db.conn().execute_batch(&ddl)?;
	Ok(())
}

pub struct SamplesWriter {
	db: DbRw,
}

impl SamplesWriter {
	pub fn new(db: DbRw) -> Self {
		Self { db }
	}

	/// Writes one transaction per nonempty batch. A failed batch rolls back as a whole.
	pub fn write_batch(&mut self, ticks: &[Tick]) -> Result<()> {
		if ticks.is_empty() {
			return Ok(());
		}
		let tx = self.db.conn_mut().transaction()?;
		{
			let mut stmt = tx.prepare_cached(INSERT_SQL.as_str())?;
			let mut values: Vec<Value> = Vec::with_capacity(CHANNELS.len() + 1);
			for tick in ticks {
				values.push(Value::Integer(tick.epoch_micro));
				values.extend(tick.v.iter().map(|v| Value::Real(f64::from(*v))));
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

// region:    --- Tests

#[cfg(test)]
mod tests {
	use super::{create_main, SamplesWriter};
	use crate::_test_support::{fixture_dir, table_columns};
	use crate::conn::DbRw;
	use lib_common::channels::{CHANNELS, CHANNEL_COUNT};
	use lib_common::Tick;

	fn count_rows(writer: &SamplesWriter) -> crate::Result<i64> {
		Ok(writer
			.db
			.conn()
			.query_row("SELECT count(*) FROM samples", [], |row| row.get(0))?)
	}

	#[test]
	fn test_schema_main_create_main_table_info_columns() -> crate::Result<()> {
		let dir = fixture_dir("test_schema_main_create_main_table_info_columns")?;
		let db = DbRw::create_new(dir.join("capture-main.sqlite"))?;
		create_main(&db, false)?;

		let names = table_columns(db.conn(), "samples")?;
		let mut expected = vec!["ts_epoch_micro".to_string()];
		expected.extend(CHANNELS.iter().map(|c| c.to_string()));
		assert_eq!(names, expected);
		db.close()?;
		Ok(())
	}

	#[test]
	fn test_schema_main_create_main_index_opt_in() -> crate::Result<()> {
		let dir = fixture_dir("test_schema_main_create_main_index_opt_in")?;
		let plain = DbRw::create_new(dir.join("plain.sqlite"))?;
		create_main(&plain, false)?;
		let plain_count: i64 = plain.conn().query_row(
			"SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name LIKE 'idx_samples_%'",
			[],
			|row| row.get(0),
		)?;
		plain.close()?;

		let indexed = DbRw::create_new(dir.join("indexed.sqlite"))?;
		create_main(&indexed, true)?;
		let indexed_count: i64 = indexed.conn().query_row(
			"SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name LIKE 'idx_samples_%'",
			[],
			|row| row.get(0),
		)?;
		indexed.close()?;

		assert_eq!(plain_count, 0);
		assert_eq!(indexed_count, CHANNEL_COUNT as i64);
		Ok(())
	}

	#[test]
	fn test_schema_main_samples_writer_round_trip() -> crate::Result<()> {
		let dir = fixture_dir("test_schema_main_samples_writer_round_trip")?;
		let db = DbRw::create_new(dir.join("capture-main.sqlite"))?;
		create_main(&db, false)?;
		let mut writer = SamplesWriter::new(db);

		writer.write_batch(&[
			Tick {
				epoch_micro: 1,
				v: [0.5_f32; CHANNEL_COUNT],
			},
			Tick {
				epoch_micro: 2,
				v: [1.5_f32; CHANNEL_COUNT],
			},
		])?;

		assert_eq!(count_rows(&writer)?, 2);
		let sql = format!("SELECT {} FROM samples WHERE ts_epoch_micro = 2", CHANNELS[0]);
		let value: f64 = writer.db.conn().query_row(&sql, [], |row| row.get(0))?;
		assert_eq!(value, 1.5);
		writer.close()?;
		Ok(())
	}

	#[test]
	fn test_schema_main_samples_writer_duplicate_rolls_back() -> crate::Result<()> {
		let dir = fixture_dir("test_schema_main_samples_writer_duplicate_rolls_back")?;
		let db = DbRw::create_new(dir.join("capture-main.sqlite"))?;
		create_main(&db, false)?;
		let mut writer = SamplesWriter::new(db);

		let tick = Tick {
			epoch_micro: 1,
			v: [0.0_f32; CHANNEL_COUNT],
		};
		assert!(writer.write_batch(&[tick, tick]).is_err());
		assert_eq!(count_rows(&writer)?, 0);
		Ok(())
	}
}

// endregion: --- Tests
