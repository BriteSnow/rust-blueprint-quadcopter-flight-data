use lib_common::channels::CHANNELS;

/// Insert statement for the `samples` table, in tick order.
pub(crate) fn samples_insert_sql() -> String {
	let mut columns = vec!["ts_epoch_micro".to_string()];
	columns.extend(CHANNELS.iter().map(|c| c.to_string()));
	insert_sql("samples", &columns)
}

/// Summary column names: bucket key, tick count, then `<channel>_min`, `_max`, `_avg` per channel.
pub(crate) fn summary_column_names() -> Vec<String> {
	let mut columns = vec!["ts_epoch_micro".to_string(), "tick_count".to_string()];
	for channel in CHANNELS {
		for suffix in ["min", "max", "avg"] {
			columns.push(format!("{channel}_{suffix}"));
		}
	}
	columns
}

pub(crate) fn insert_sql(table: &str, columns: &[String]) -> String {
	format!(
		"INSERT INTO {table} ({}) VALUES ({})",
		columns.join(", "),
		placeholders(columns.len())
	)
}

fn placeholders(count: usize) -> String {
	(1..=count)
		.map(|i| format!("?{i}"))
		.collect::<Vec<_>>()
		.join(", ")
}
