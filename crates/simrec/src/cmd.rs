use simple_fs::SPath;

use clap::{ArgGroup, Parser};

use crate::{mock_in::TICK_NS, Error, Result};

#[derive(Debug, Parser)]
#[command(version, about = "Replay sensor ticks through a dedicated capture pipeline")]
#[command(group(ArgGroup::new("backend").required(true).multiple(false).args(["noop", "sql", "raw"])))]
pub struct CliCmd {
    #[arg(long)]
    pub noop: bool,

    #[arg(long)]
    pub sql: bool,

    #[arg(long)]
    pub raw: bool,

    #[arg(long, conflicts_with_all = ["noop", "raw"])]
    pub with_index: bool,

    #[arg(long, default_value = "d1", value_parser = duration_arg)]
    pub duration: String,

    #[arg(short, long, default_value = "sandbox", value_parser = sensor_path)]
    pub out: SPath,

    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub flush_ms: Option<u64>,

    #[arg(long, default_value = "real", value_parser = ["real", "max"])]
    pub speed: String,

    #[arg(long, default_value_t = 4096, value_parser = ring_capacity)]
    pub ring_cap: usize,

    #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(1..))]
    pub sample_ms: u64,
}

// region:    --- Support

fn sensor_path(value: &str) -> core::result::Result<SPath, String> {
    if value.is_empty() {
        return Err("path must not be empty".to_string());
    }
    Ok(SPath::new(value))
}

fn ring_capacity(value: &str) -> core::result::Result<usize, String> {
    let capacity = value
        .parse::<usize>()
        .map_err(|_| "ring capacity must be an integer".to_string())?;
    if !(1..=65_536).contains(&capacity) {
        return Err("ring capacity must be between 1 and 65536 messages".to_string());
    }
    Ok(capacity)
}

fn duration_arg(value: &str) -> core::result::Result<String, String> {
    duration_seconds(value)
        .map(|_| value.to_string())
        .ok_or_else(|| {
            format!("invalid duration {value:?}; expected d, h, or m followed by a positive integer")
        })
}

pub fn duration_name(value: &str) -> Result<String> {
    let seconds = duration_seconds(value).ok_or_else(|| {
        Error::custom(format!(
            "invalid duration {value:?}; expected d, h, or m followed by a positive integer"
        ))
    })?;
    if seconds.is_multiple_of(86_400) {
        Ok(format!("d{:03}", seconds / 86_400))
    } else {
        Ok(format!("m{:03}", seconds / 60))
    }
}

pub fn duration_tick_count(value: &str) -> Result<u64> {
    let seconds = duration_seconds(value).ok_or_else(|| {
        Error::custom(format!(
            "invalid duration {value:?}; expected d, h, or m followed by a positive integer"
        ))
    })?;
    const NANOS_PER_SECOND: u64 = 1_000_000_000;
    seconds
        .checked_mul(NANOS_PER_SECOND / TICK_NS)
        .ok_or_else(|| Error::custom(format!("duration {value:?} exceeds the tick range")))
}

/// Returns the mock dataset names that can serve a duration, in preference order.
pub fn dataset_candidates(value: &str) -> Result<&'static [&'static str]> {
    let seconds = duration_seconds(value).ok_or_else(|| {
        Error::custom(format!(
            "invalid duration {value:?}; expected d, h, or m followed by a positive integer"
        ))
    })?;
    match seconds {
        0..=86_400 => Ok(&["d001", "d010"][..]),
        86_401..=864_000 => Ok(&["d010"][..]),
        _ => Err(Error::custom(format!(
            "duration {value:?} exceeds the 10-day mock dataset"
        ))),
    }
}

fn duration_seconds(value: &str) -> Option<u64> {
    let unit_seconds = match *value.as_bytes().first()? {
        b'd' => 86_400_u64,
        b'h' => 3_600_u64,
        b'm' => 60_u64,
        _ => return None,
    };
    let digits = &value[1..];
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let amount = digits.parse::<u64>().ok()?;
    if amount == 0 {
        return None;
    }
    amount.checked_mul(unit_seconds)
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    use super::*;

    #[test]
    fn test_cmd_cli_cmd_defaults() -> Result<()> {
        // -- Setup & Fixtures
        let arguments = ["simrec", "--noop"];

        // -- Exec
        let command = CliCmd::try_parse_from(arguments)?;

        // -- Check
        assert!(command.noop);
        assert!(!command.sql);
        assert!(!command.raw);
        assert!(!command.with_index);
        assert_eq!(command.duration, "d1");
        assert_eq!(command.out.to_string(), "sandbox");
        assert_eq!(command.flush_ms, None);
        assert_eq!(command.speed, "real");
        assert_eq!(command.ring_cap, 4096);
        assert_eq!(command.sample_ms, 1000);
        Ok(())
    }

    #[test]
    fn test_cmd_cli_cmd_custom_paths() -> Result<()> {
        // -- Setup & Fixtures
        let arguments = [
            "simrec",
            "--noop",
            "--duration",
            "m30",
            "--out",
            "tests-data/output",
        ];

        // -- Exec
        let command = CliCmd::try_parse_from(arguments)?;

        // -- Check
        assert_eq!(command.duration, "m30");
        assert_eq!(command.out.to_string(), "tests-data/output");
        Ok(())
    }

    #[test]
    fn test_cmd_cli_cmd_invalid_arguments() -> Result<()> {
        // -- Setup & Fixtures
        let cases: &[&[&str]] = &[
            &["simrec"],
            &["simrec", "--noop", "--raw"],
            &["simrec", "--sql", "--raw"],
            &["simrec", "--noop", "--with-index"],
            &["simrec", "--noop", "--flush-ms", "0"],
            &["simrec", "--noop", "--sample-ms", "0"],
            &["simrec", "--noop", "--ring-cap", "0"],
            &["simrec", "--noop", "--ring-cap", "65537"],
            &["simrec", "--noop", "--ring-cap", "invalid"],
            &["simrec", "--noop", "--speed", "invalid"],
            &["simrec", "--noop", "--duration", ""],
            &["simrec", "--noop", "--duration", "d0"],
            &["simrec", "--noop", "--duration", "1d"],
            &["simrec", "--noop", "--out", ""],
        ];

        // -- Exec & Check
        for arguments in cases {
            assert!(
                CliCmd::try_parse_from(arguments.iter().copied()).is_err(),
                "arguments must be rejected: {arguments:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn test_cmd_duration_name_simple() -> Result<()> {
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
    fn test_cmd_duration_tick_count_simple() -> Result<()> {
        // -- Setup & Fixtures
        let cases = [("m1", 12_000), ("h1", 720_000), ("d1", 17_280_000)];

        // -- Exec & Check
        for (duration, expected) in cases {
            assert_eq!(duration_tick_count(duration)?, expected);
        }
        Ok(())
    }

    #[test]
    fn test_cmd_dataset_candidates_simple() -> Result<()> {
        // -- Setup & Fixtures
        let cases = [
            ("m1", &["d001", "d010"][..]),
            ("d1", &["d001", "d010"][..]),
            ("h24", &["d001", "d010"][..]),
            ("d2", &["d010"][..]),
            ("d10", &["d010"][..]),
        ];

        // -- Exec & Check
        for (duration, expected) in cases {
            assert_eq!(dataset_candidates(duration)?, expected);
        }
        assert!(dataset_candidates("d11").is_err());
        Ok(())
    }
}

// endregion: --- Tests
