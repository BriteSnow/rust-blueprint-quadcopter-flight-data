use clap::Parser;
use simple_fs::SPath;

use crate::{
    error::{Error, Result},
    writer, BYTES_PER_TICK, RATE_HZ, TICK_NS,
};

#[derive(Parser)]
#[command(about = "Generate a deterministic, streaming quadcopter sensor dataset")]
struct Args {
    #[arg(long, default_value = "d1")]
    duration: String,
    #[arg(long, default_value_t = 42)]
    seed: u64,
    #[arg(short, long, default_value = "sandbox", value_parser = sensor_path)]
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
    let out = args.out.join("mock-data").join(duration_name(&args.duration)?);
    writer::generate(&out, args.seed, tick_count)?;
    eprintln!(
        "Generated {tick_count} ticks in {} (seed {}).",
        out,
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

fn duration_seconds(value: &str) -> Result<u64> {
    let invalid = || {
        Error::Invalid(format!(
            "invalid duration {value:?}; expected d, h, or m followed by a positive integer"
        ))
    };
    let unit = value.as_bytes().first().copied().ok_or_else(invalid)?;
    let unit_seconds = match unit {
        b'd' => 86_400_u64,
        b'h' => 3_600_u64,
        b'm' => 60_u64,
        _ => return Err(invalid()),
    };
    let digits = &value[1..];
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    let amount = digits.parse::<u64>().map_err(|_| invalid())?;
    if amount == 0 {
        return Err(invalid());
    }
    amount
        .checked_mul(unit_seconds)
        .ok_or_else(|| Error::Invalid("duration exceeds the time range".into()))
}

fn duration_name(value: &str) -> Result<String> {
    let seconds = duration_seconds(value)?;
    if seconds.is_multiple_of(86_400) {
        Ok(format!("d{:03}", seconds / 86_400))
    } else {
        Ok(format!("m{:03}", seconds / 60))
    }
}

fn duration_ticks(value: &str) -> Result<u64> {
    let seconds = duration_seconds(value)?;
    let ticks = seconds
        .checked_mul(u64::from(RATE_HZ))
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
        let short = Args::try_parse_from(["mocker", "-o", "tests-data/generated"])?;
        let empty = Args::try_parse_from(["mocker", "--out", ""]);

        // -- Check
        assert_eq!(defaults.out.to_string(), "sandbox");
        assert_eq!(custom.out.to_string(), "tests-data/generated");
        assert_eq!(short.out.to_string(), "tests-data/generated");
        assert!(empty.is_err());
        Ok(())
    }

    #[test]
    fn test_main_duration_ticks_simple() -> Result<()> {
        // -- Setup & Fixtures
        let cases = [
            ("m1", 12_000_u64),
            ("h1", 720_000),
            ("d1", 17_280_000),
            ("d10", 172_800_000),
        ];

        // -- Exec & Check
        for (duration, expected) in cases {
            assert_eq!(duration_ticks(duration)?, expected);
        }
        Ok(())
    }

    #[test]
    fn test_main_duration_name_simple() -> Result<()> {
        // -- Setup & Fixtures
        let cases = [
            ("d1", "d001"),
            ("d10", "d010"),
            ("m30", "m030"),
            ("h1", "m060"),
            ("h24", "d001"),
            ("m1440", "d001"),
        ];

        // -- Exec & Check
        for (duration, expected) in cases {
            assert_eq!(duration_name(duration)?, expected);
        }
        Ok(())
    }

    #[test]
    fn test_main_duration_ticks_invalid() -> Result<()> {
        // -- Setup & Fixtures
        let cases = [
            "",
            "d0",
            "m0",
            "1",
            "1d",
            "1s",
            "-m1",
            "h1.5",
            " d1",
            "d1 ",
            "m18446744073709551616",
            "d18446744073709551615",
            "d106752",
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
        let duration = "d106751";

        // -- Exec
        let ticks = duration_ticks(duration)?;
        let last_ns = (ticks - 1)
            .checked_mul(TICK_NS)
            .ok_or("last timestamp overflow")?;

        // -- Check
        assert!(last_ns <= i64::MAX as u64);
        assert!(duration_ticks("d106752").is_err());
        Ok(())
    }
}

// endregion: --- Tests
