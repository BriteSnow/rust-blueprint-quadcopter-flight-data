use std::{
    fs::{self, OpenOptions},
    io::Write,
    time::Duration,
};

use serde::Serialize;
use simple_fs::SPath;

use crate::{
    rec::{ConsumerStats, MAX_BATCH_TICKS},
    sim::{ProducerStats, MAX_PENDING_TICKS},
    Result,
};

#[derive(Serialize)]
pub struct Metrics {
    backend: &'static str,
    speed: String,
    flush_ms: u64,
    ring_cap: usize,
    max_pending_ticks: usize,
    max_batch_ticks: usize,
    ticks_total: u64,
    wall_time_s: f64,
    ticks_per_s: f64,
    stall_count: u64,
    max_pending_len: usize,
    flush_count: u64,
    flush_ms_p50: f64,
    flush_ms_p99: f64,
    flush_ms_max: f64,
    percentile_method: &'static str,
    finish_ms: f64,
    output_bytes: u64,
}

pub struct FlushHistogram {
    bins: [u64; 65],
    count: u64,
    max_ns: u64,
}

impl Metrics {
    pub fn from_run(
        backend: &'static str,
        speed: String,
        flush_ms: u64,
        ring_cap: usize,
        elapsed: Duration,
        produced: ProducerStats,
        recorded: ConsumerStats,
    ) -> Self {
        let wall_time_s = elapsed.as_secs_f64();
        Self {
            backend,
            speed,
            flush_ms,
            ring_cap,
            max_pending_ticks: MAX_PENDING_TICKS,
            max_batch_ticks: MAX_BATCH_TICKS,
            ticks_total: recorded.ticks_total,
            wall_time_s,
            ticks_per_s: if wall_time_s > 0.0 {
                recorded.ticks_total as f64 / wall_time_s
            } else {
                0.0
            },
            stall_count: produced.stall_count,
            max_pending_len: produced.max_pending_len,
            flush_count: recorded.flushes.count(),
            flush_ms_p50: recorded.flushes.percentile_ms(50),
            flush_ms_p99: recorded.flushes.percentile_ms(99),
            flush_ms_max: recorded.flushes.max_ns as f64 / 1_000_000.0,
            percentile_method: "nearest-rank power-of-two nanosecond bucket upper bound",
            finish_ms: recorded.finish_ms,
            output_bytes: recorded.output_bytes,
        }
    }

    pub fn publish(&self, directory: &SPath) -> Result<()> {
        self.publish_as(directory, "metrics.json")
    }

    pub fn publish_as(&self, directory: &SPath, file_name: &str) -> Result<()> {
        let temporary_path = directory.join(format!(".{file_name}.tmp"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)?;
        serde_json::to_writer_pretty(&mut file, self)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::hard_link(&temporary_path, directory.join(file_name))?;
        if let Err(error) = fs::remove_file(&temporary_path) {
            eprintln!("simrec: metrics published, but temporary cleanup failed: {error}");
        }
        Ok(())
    }
}

impl FlushHistogram {
    pub fn record(&mut self, elapsed: Duration) {
        let nanos = elapsed.as_nanos().min(u128::from(u64::MAX)) as u64;
        let bucket = if nanos <= 1 {
            0
        } else {
            (u64::BITS - (nanos - 1).leading_zeros()) as usize
        };
        self.bins[bucket] += 1;
        self.count += 1;
        self.max_ns = self.max_ns.max(nanos);
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn percentile_ms(&self, percentile: u64) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let rank = (u128::from(self.count) * u128::from(percentile.clamp(1, 100))).div_ceil(100);
        let mut cumulative = 0_u128;
        for (index, count) in self.bins.iter().enumerate() {
            cumulative += u128::from(*count);
            if cumulative >= rank {
                let upper = if index == 64 {
                    u64::MAX
                } else {
                    1_u64 << index
                };
                return upper.min(self.max_ns) as f64 / 1_000_000.0;
            }
        }
        self.max_ns as f64 / 1_000_000.0
    }
}

impl Default for FlushHistogram {
    fn default() -> Self {
        Self {
            bins: [0; 65],
            count: 0,
            max_ns: 0,
        }
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    use super::*;

    #[test]
    fn test_metrics_from_run_diagnostic_values() -> Result<()> {
        // -- Setup & Fixtures
        let produced = ProducerStats {
            ticks_total: 400,
            stall_count: 7,
            max_pending_len: 12,
        };
        let mut flushes = FlushHistogram::default();
        flushes.record(Duration::from_millis(1));
        flushes.record(Duration::from_millis(3));
        let recorded = ConsumerStats {
            ticks_total: 400,
            flushes,
            finish_ms: 2.5,
            output_bytes: 0,
        };

        // -- Exec
        let metrics = Metrics::from_run(
            "noop",
            "max".to_string(),
            10,
            16,
            Duration::from_secs(2),
            produced,
            recorded,
        );
        let serialized = serde_json::to_value(&metrics)?;

        // -- Check
        assert_eq!(serialized["backend"], "noop");
        assert_eq!(serialized["speed"], "max");
        assert_eq!(serialized["flush_ms"], 10);
        assert_eq!(serialized["ring_cap"], 16);
        assert_eq!(serialized["max_pending_ticks"], MAX_PENDING_TICKS);
        assert_eq!(serialized["max_batch_ticks"], MAX_BATCH_TICKS);
        assert_eq!(serialized["ticks_total"], 400);
        assert_eq!(serialized["wall_time_s"], 2.0);
        assert_eq!(serialized["ticks_per_s"], 200.0);
        assert_eq!(serialized["stall_count"], 7);
        assert_eq!(serialized["max_pending_len"], 12);
        assert_eq!(serialized["flush_count"], 2);
        assert_eq!(serialized["flush_ms_p50"], 1.048576);
        assert_eq!(serialized["flush_ms_p99"], 3.0);
        assert_eq!(serialized["flush_ms_max"], 3.0);
        assert_eq!(serialized["finish_ms"], 2.5);
        assert_eq!(serialized["output_bytes"], 0);
        assert_eq!(
            serialized["percentile_method"],
            "nearest-rank power-of-two nanosecond bucket upper bound"
        );
        Ok(())
    }

    #[test]
    fn test_metrics_publish_complete_document() -> Result<()> {
        // -- Setup & Fixtures
        let directory = fixture_directory("test_metrics_publish_complete_document")?;
        let metrics = fixture_metrics();
        let expected = serde_json::to_value(&metrics)?;

        // -- Exec
        metrics.publish(&directory)?;
        let bytes = std::fs::read(directory.join("metrics.json"))?;
        let actual: serde_json::Value = serde_json::from_slice(&bytes)?;

        // -- Check
        assert_eq!(actual, expected);
        assert_eq!(bytes.last(), Some(&b'\n'));
        assert!(!std::fs::exists(directory.join(".metrics.json.tmp"))?);
        assert_eq!(std::fs::read_dir(&directory)?.count(), 1);
        Ok(())
    }

    #[test]
    fn test_metrics_publish_existing_destination() -> Result<()> {
        // -- Setup & Fixtures
        let directory = fixture_directory("test_metrics_publish_existing_destination")?;
        let metrics = fixture_metrics();
        metrics.publish(&directory)?;
        let original = std::fs::read(directory.join("metrics.json"))?;
        let mut replacement = fixture_metrics();
        replacement.ticks_total = 999;

        // -- Exec
        let result = replacement.publish(&directory);
        let retained = std::fs::read(directory.join("metrics.json"))?;
        let temporary: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.join(".metrics.json.tmp"))?,
        )?;

        // -- Check
        assert!(result.is_err());
        assert_eq!(retained, original);
        assert_eq!(temporary["ticks_total"], 999);
        Ok(())
    }

    #[test]
    fn test_metrics_publish_existing_temporary_file() -> Result<()> {
        // -- Setup & Fixtures
        let directory =
            fixture_directory("test_metrics_publish_existing_temporary_file")?;
        let temporary_path = directory.join(".metrics.json.tmp");
        let original = b"retained partial publication";
        std::fs::write(&temporary_path, original)?;
        let metrics = fixture_metrics();

        // -- Exec
        let result = metrics.publish(&directory);

        // -- Check
        assert!(matches!(
            result,
            Err(crate::Error::Io(error))
                if error.kind() == std::io::ErrorKind::AlreadyExists
        ));
        assert_eq!(std::fs::read(&temporary_path)?, original);
        assert!(!std::fs::exists(directory.join("metrics.json"))?);
        Ok(())
    }

    #[test]
    fn test_metrics_flush_histogram_percentiles() -> Result<()> {
        // -- Setup & Fixtures
        let mut histogram = FlushHistogram::default();

        // -- Exec
        for nanos in [1_u64, 2, 3, 4, 5] {
            histogram.record(Duration::from_nanos(nanos));
        }

        // -- Check
        assert_eq!(histogram.count(), 5);
        assert_eq!(histogram.percentile_ms(50), 4.0 / 1_000_000.0);
        assert_eq!(histogram.percentile_ms(99), 5.0 / 1_000_000.0);
        assert_eq!(histogram.max_ns, 5);
        Ok(())
    }

    #[test]
    fn test_metrics_flush_histogram_empty_and_saturated() -> Result<()> {
        // -- Setup & Fixtures
        let mut histogram = FlushHistogram::default();

        // -- Exec & Check
        assert_eq!(histogram.percentile_ms(99), 0.0);
        histogram.record(Duration::from_secs(u64::MAX));
        assert_eq!(histogram.max_ns, u64::MAX);
        assert_eq!(
            histogram.percentile_ms(99),
            u64::MAX as f64 / 1_000_000.0
        );
        Ok(())
    }

    // -- Test Support

    fn fixture_metrics() -> Metrics {
        Metrics::from_run(
            "noop",
            "max".to_string(),
            10,
            1,
            Duration::from_secs(1),
            ProducerStats::default(),
            ConsumerStats::default(),
        )
    }

    fn fixture_directory(test_name: &str) -> Result<SPath> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let parent = SPath::new("tests-data/.tmp").join(test_name);
        std::fs::create_dir_all(&parent)?;
        let directory = parent.join(format!("{}-{unique}", std::process::id()));
        std::fs::create_dir(&directory)?;
        Ok(directory)
    }
}

// endregion: --- Tests
