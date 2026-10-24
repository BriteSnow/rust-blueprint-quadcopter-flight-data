use super::publish::publish_json;
use crate::dispatcher::DispatchStats;
use crate::rec::rec_sum::{SumStats, SUM_BUCKET_US};
use crate::Result;
use serde::Serialize;
use simple_fs::SPath;

pub const SUM_METRICS_FILE_NAME: &str = "capture-sum-metrics.json";

#[derive(Serialize)]
pub struct SumMetrics {
	backend: &'static str,
	speed: String,
	bucket_us: i64,
	sum_flush_s: u64,
	sum_ring_cap: usize,
	ticks_received: u64,
	ticks_dropped: u64,
	batches_dropped: u64,
	rows_written: u64,
	partial_rows: u64,
	wall_time_s: f64,
	rows_per_s: f64,
	flush_count: u64,
	flush_ms_p50: f64,
	flush_ms_p99: f64,
	flush_ms_max: f64,
	percentile_method: &'static str,
	finish_ms: f64,
	output_bytes: u64,
}

impl SumMetrics {
	pub fn from_run(
		speed: String,
		sum_flush_s: u64,
		sum_ring_cap: usize,
		dispatched: &DispatchStats,
		stats: SumStats,
	) -> Self {
		Self {
			backend: "sqlite-sum",
			speed,
			bucket_us: SUM_BUCKET_US,
			sum_flush_s,
			sum_ring_cap,
			ticks_received: stats.ticks_received,
			ticks_dropped: dispatched.sum_ticks_dropped,
			batches_dropped: dispatched.sum_batches_dropped,
			rows_written: stats.rows_written,
			partial_rows: stats.partial_rows,
			wall_time_s: stats.wall_time_s,
			rows_per_s: if stats.wall_time_s > 0.0 {
				stats.rows_written as f64 / stats.wall_time_s
			} else {
				0.0
			},
			flush_count: stats.flushes.count(),
			flush_ms_p50: stats.flushes.percentile_ms(50),
			flush_ms_p99: stats.flushes.percentile_ms(99),
			flush_ms_max: stats.flushes.max_ns as f64 / 1_000_000.0,
			percentile_method: "nearest-rank power-of-two nanosecond bucket upper bound",
			finish_ms: stats.finish_ms,
			output_bytes: stats.output_bytes,
		}
	}

	pub fn publish(&self, directory: &SPath) -> Result<()> {
		publish_json(directory, SUM_METRICS_FILE_NAME, self)
	}
}
