use crate::error::Result;
use rusqlite::Connection;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Creates a unique fixture directory under `tests-data/.tmp/<test_name>/<nanos>`.
pub fn fixture_dir(test_name: &str) -> Result<PathBuf> {
	let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
	let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("tests-data/.tmp")
		.join(test_name)
		.join(nanos.to_string());
	std::fs::create_dir_all(&dir)?;
	Ok(dir)
}

/// Returns the column names of a table in declaration order.
pub fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
	let sql = format!("SELECT name FROM pragma_table_info('{table}') ORDER BY cid");
	let mut stmt = conn.prepare(&sql)?;
	let names = stmt
		.query_map([], |row| row.get::<_, String>(0))?
		.collect::<rusqlite::Result<Vec<String>>>()?;
	Ok(names)
}
