use simple_fs::SPath;

use clap::{ArgGroup, Parser};

use crate::{Error, Result};

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

    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..))]
    pub flush_ms: u64,

    #[arg(long, default_value = "realtime", value_parser = ["realtime", "max"])]
    pub speed: String,

    #[arg(long, default_value_t = 4096, value_parser = ring_capacity)]
    pub ring_cap: usize,
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
        assert_eq!(command.flush_ms, 10);
        assert_eq!(command.speed, "realtime");
        assert_eq!(command.ring_cap, 4096);
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
}

// endregion: --- Tests
