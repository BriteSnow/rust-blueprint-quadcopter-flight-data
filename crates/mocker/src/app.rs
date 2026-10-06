use clap::Parser;
use simple_fs::SPath;

use crate::{
    error::{Error, Result},
    writer, BYTES_PER_TICK, RATE_HZ, TICK_NS,
};

#[derive(Parser)]
#[command(about = "Generate a deterministic, streaming quadcopter sensor dataset")]
struct Args {
    #[arg(long, default_value = "1d")]
    duration: String,
    #[arg(long, default_value_t = 42)]
    seed: u64,
    #[arg(long, default_value = "data/mock", value_parser = sensor_path)]
    out: SPath,
    #[arg(long, default_value_t = RATE_HZ)]
    rate_hz: u32,
}

pub(super) fn run() -> Result<()> {
    let args = Args::parse();
    if args.rate_hz != RATE_HZ {
        return Err(Error::Invalid(format!(
            "--rate-hz must be {RATE_HZ} for this proof of concept"
        )));
    }
    let tick_count = duration_ticks(&args.duration)?;
    writer::generate(&args.out, args.seed, tick_count)?;
    eprintln!(
        "Generated {tick_count} ticks in {} (seed {}).",
        args.out,
        args.seed
    );
    Ok(())
}

// region:    --- Support

fn sensor_path(value: &str) -> core::result::Result<SPath, String> {
    if value.is_empty() {
        return Err("path must not be empty".to_string());
    }
    Ok(SPath::new(value))
}

fn duration_ticks(value: &str) -> Result<u64> {
    let invalid = || {
        Error::Invalid(format!(
            "invalid duration {value:?}; expected a positive integer followed by d, h, or m"
        ))
    };
    let unit = value.as_bytes().last().copied().ok_or_else(invalid)?;
    let multiplier = match unit {
        b'd' => 86_400_u64,
        b'h' => 3_600_u64,
        b'm' => 60_u64,
        _ => return Err(invalid()),
    };
    let digits = &value[..value.len() - 1];
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    let amount = digits.parse::<u64>().map_err(|_| invalid())?;
    if amount == 0 {
        return Err(invalid());
    }
    let ticks = amount
        .checked_mul(multiplier)
        .and_then(|seconds| seconds.checked_mul(u64::from(RATE_HZ)))
        .ok_or_else(|| Error::Invalid("duration exceeds the tick-count range".into()))?;
    ticks
        .checked_mul(BYTES_PER_TICK as u64)
        .ok_or_else(|| Error::Invalid("duration exceeds the output-size range".into()))?;
    let last_ns = (ticks - 1)
        .checked_mul(TICK_NS)
        .ok_or_else(|| Error::Invalid("duration exceeds the timestamp range".into()))?;
    if last_ns > i64::MAX as u64 {
        return Err(Error::Invalid(
            "duration exceeds the signed nanosecond timestamp range".into(),
        ));
    }
    Ok(ticks)
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    use super::*;

    #[test]
    fn test_main_args_output_paths() -> Result<()> {
        // -- Setup & Fixtures
        let custom_arguments = ["mocker", "--out", "tests-data/generated"];

        // -- Exec
        let defaults = Args::try_parse_from(["mocker"])?;
        let custom = Args::try_parse_from(custom_arguments)?;
        let empty = Args::try_parse_from(["mocker", "--out", ""]);

        // -- Check
        assert_eq!(defaults.out.to_string(), "data/mock");
        assert_eq!(custom.out.to_string(), "tests-data/generated");
        assert!(empty.is_err());
        Ok(())
    }

    #[test]
    fn test_main_duration_ticks_simple() -> Result<()> {
        // -- Setup & Fixtures
        let cases = [
            ("1m", 12_000_u64),
            ("1h", 720_000),
            ("1d", 17_280_000),
            ("10d", 172_800_000),
        ];

        // -- Exec & Check
        for (duration, expected) in cases {
            assert_eq!(duration_ticks(duration)?, expected);
        }
        Ok(())
    }

    #[test]
    fn test_main_duration_ticks_invalid() -> Result<()> {
        // -- Setup & Fixtures
        let cases = [
            "",
            "0m",
            "1",
            "1s",
            "-1m",
            "1.5h",
            " 1d",
            "1d ",
            "18446744073709551616m",
            "18446744073709551615d",
            "106752d",
        ];

        // -- Exec & Check
        for duration in cases {
            assert!(
                duration_ticks(duration).is_err(),
                "duration must be rejected: {duration:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn test_main_duration_ticks_timestamp_boundary() -> Result<()> {
        // -- Setup & Fixtures
        let duration = "106751d";

        // -- Exec
        let ticks = duration_ticks(duration)?;
        let last_ns = (ticks - 1)
            .checked_mul(TICK_NS)
            .ok_or("last timestamp overflow")?;

        // -- Check
        assert!(last_ns <= i64::MAX as u64);
        assert!(duration_ticks("106752d").is_err());
        Ok(())
    }
}

// endregion: --- Tests
