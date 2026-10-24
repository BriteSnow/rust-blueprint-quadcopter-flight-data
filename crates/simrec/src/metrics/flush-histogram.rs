use std::time::Duration;

pub struct FlushHistogram {
	bins: [u64; 65],
	count: u64,
	pub(crate) max_ns: u64,
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
				let upper = if index == 64 { u64::MAX } else { 1_u64 << index };
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
		assert_eq!(histogram.percentile_ms(99), u64::MAX as f64 / 1_000_000.0);
		Ok(())
	}
}

// endregion: --- Tests
