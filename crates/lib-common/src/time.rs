use std::time::{SystemTime, UNIX_EPOCH};

/// Returns the current wall-clock time in microseconds since the Unix epoch.
///
/// Times before the epoch are negative. Values outside the `i64` range saturate.
pub fn now() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_micros()).unwrap_or(i64::MAX),
        Err(error) => i64::try_from(error.duration().as_micros())
            .map(|micros| -micros)
            .unwrap_or(i64::MIN),
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    use super::*;

    #[test]
    fn test_time_now_epoch_micros() -> Result<()> {
        // -- Setup & Fixtures
        let reference = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros())?;

        // -- Exec
        let actual = now();

        // -- Check
        assert!((actual - reference).abs() < 5_000_000);
        Ok(())
    }
}

// endregion: --- Tests
