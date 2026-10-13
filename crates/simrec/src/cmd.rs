use simple_fs::SPath;

use clap::{ArgGroup, Parser};

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

    #[arg(long, requires = "sql")]
    pub with_index: bool,

    #[arg(long, default_value = "data/mock", value_parser = sensor_path)]
    pub mock_dir: SPath,

    #[arg(long, default_value = ".out", value_parser = sensor_path)]
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
        assert_eq!(command.mock_dir.to_string(), "data/mock");
        assert_eq!(command.out.to_string(), ".out");
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
            "--mock-dir",
            "tests-data/input",
            "--out",
            "tests-data/output",
        ];

        // -- Exec
        let command = CliCmd::try_parse_from(arguments)?;

        // -- Check
        assert_eq!(command.mock_dir.to_string(), "tests-data/input");
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
            &["simrec", "--noop", "--mock-dir", ""],
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
}

// endregion: --- Tests
