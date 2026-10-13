use std::{
    fs::File,
    io::{BufReader, Read},
    time::{Duration, Instant},
};

use serde::Deserialize;
use simple_fs::SPath;

use crate::{recorder::Tick, Error, Result};

pub struct MockInput {
    manifest: Manifest,
    reader: BufReader<File>,
    next_tick: u64,
    tick_limit: u64,
}

pub const TICK_NS: u64 = 5_000_000;
pub const BYTES_PER_TICK: u64 = 20;
const CHANNELS: [&str; 5] = [
    "rotor_fl",
    "rotor_fr",
    "rotor_rl",
    "rotor_rr",
    "accel_z",
];

impl MockInput {
    pub fn open(directory: &SPath) -> Result<Self> {
        let manifest_path = directory.join("manifest.json");
        let manifest_file = File::open(&manifest_path).map_err(|error| {
            Error::custom(format!("cannot open {manifest_path}: {error}"))
        })?;
        if !manifest_file.metadata()?.is_file() {
            return Err(Error::custom("manifest.json must be a regular file"));
        }
        let manifest: Manifest = serde_json::from_reader(BufReader::new(manifest_file))?;
        let ticks_path = directory.join("ticks.bin");
        let ticks = File::open(&ticks_path).map_err(|error| {
            Error::custom(format!("cannot open {ticks_path}: {error}"))
        })?;
        let metadata = ticks.metadata()?;
        if !metadata.is_file() {
            return Err(Error::custom("ticks.bin must be a regular file"));
        }
        validate_manifest(&manifest, metadata.len())?;
        let story_path = directory.join("story.jsonl");
        if !File::open(&story_path)?.metadata()?.is_file() {
            return Err(Error::custom("story.jsonl must be a regular file"));
        }
        let tick_limit = manifest.tick_count;
        Ok(Self {
            manifest,
            reader: BufReader::with_capacity(1_048_576, ticks),
            next_tick: 0,
            tick_limit,
        })
    }

    pub fn tick_count(&self) -> u64 {
        self.tick_limit
    }

    pub fn with_tick_limit(mut self, ticks: u64) -> Result<Self> {
        if ticks == 0 || ticks > self.manifest.tick_count {
            return Err(Error::custom(format!(
                "requested {ticks} ticks is outside the dataset range of {} ticks",
                self.manifest.tick_count
            )));
        }
        self.tick_limit = ticks;
        Ok(self)
    }

    pub fn validate_clock_range(&self) -> Result<()> {
        let offset = (self.tick_limit - 1)
            .checked_mul(TICK_NS)
            .ok_or("realtime duration exceeds the clock range")?;
        if Instant::now()
            .checked_add(Duration::from_nanos(offset))
            .is_none()
        {
            return Err(Error::custom("realtime duration exceeds the clock range"));
        }
        Ok(())
    }

    pub fn next_tick(&mut self) -> Result<Option<Tick>> {
        if self.next_tick == self.tick_limit {
            // A truncated replay stops before the file end, so the trailing-byte check is skipped.
            if self.tick_limit < self.manifest.tick_count {
                return Ok(None);
            }
            let mut extra = [0_u8; 1];
            if self.reader.read(&mut extra)? != 0 {
                return Err(Error::custom("ticks.bin grew after startup validation"));
            }
            return Ok(None);
        }
        let mut bytes = [0_u8; BYTES_PER_TICK as usize];
        self.reader.read_exact(&mut bytes)?;
        let tick = decode_tick(&bytes, timestamp(&self.manifest, self.next_tick)?)?;
        self.next_tick += 1;
        Ok(Some(tick))
    }
}

// region:    --- Support

#[derive(Deserialize)]
struct Manifest {
    version: u32,
    seed: u64,
    rate_hz: u32,
    channels: Vec<String>,
    tick_count: u64,
    start_ns: i64,
    bytes_per_tick: u64,
}

fn validate_manifest(manifest: &Manifest, actual_bytes: u64) -> Result<()> {
    if manifest.version != 1 {
        return Err(Error::custom("only manifest version 1 is supported"));
    }
    if manifest.rate_hz != 200 || manifest.bytes_per_tick != BYTES_PER_TICK {
        return Err(Error::custom("input must contain 200 Hz, 20-byte ticks"));
    }
    if manifest.channels.len() != CHANNELS.len()
        || !manifest
            .channels
            .iter()
            .zip(CHANNELS)
            .all(|(actual, expected)| actual == expected)
    {
        return Err(Error::custom("manifest channel order does not match the sensor format"));
    }
    if manifest.tick_count == 0 {
        return Err(Error::custom("manifest tick_count must be positive"));
    }
    let expected_bytes = manifest
        .tick_count
        .checked_mul(BYTES_PER_TICK)
        .ok_or("manifest payload size exceeds the unsigned 64-bit range")?;
    if actual_bytes != expected_bytes {
        return Err(Error::custom(format!(
            "ticks.bin size mismatch: expected {expected_bytes}, found {actual_bytes}"
        )));
    }
    timestamp(manifest, manifest.tick_count - 1)?;
    let _ = manifest.seed;
    Ok(())
}

fn timestamp(manifest: &Manifest, index: u64) -> Result<i64> {
    let timestamp = i128::from(manifest.start_ns) + i128::from(index) * i128::from(TICK_NS);
    i64::try_from(timestamp)
        .map_err(|_| Error::custom("manifest timestamps exceed the signed 64-bit range"))
}

fn decode_tick(bytes: &[u8; 20], ts_ns: i64) -> Result<Tick> {
    let mut values = [0.0_f32; 5];
    for (index, value) in values.iter_mut().enumerate() {
        let start = index * 4;
        *value = f32::from_le_bytes([
            bytes[start],
            bytes[start + 1],
            bytes[start + 2],
            bytes[start + 3],
        ]);
        if !value.is_finite() {
            return Err(Error::custom(format!(
                "non-finite sensor value at timestamp {ts_ns}, channel {index}"
            )));
        }
    }
    Ok(Tick { ts_ns, v: values })
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    use super::*;

    #[test]
    fn test_mock_in_validate_manifest_simple() -> Result<()> {
        // -- Setup & Fixtures
        let manifest = fixture_manifest();

        // -- Exec
        validate_manifest(&manifest, 40)?;
        let last = timestamp(&manifest, 1)?;

        // -- Check
        assert_eq!(last, 5_000_000);
        Ok(())
    }

    #[test]
    fn test_mock_in_validate_manifest_invalid() -> Result<()> {
        // -- Setup & Fixtures
        let mut manifest = fixture_manifest();

        // -- Exec & Check
        assert!(validate_manifest(&manifest, 39).is_err());
        manifest.channels.swap(0, 1);
        assert!(validate_manifest(&manifest, 40).is_err());
        manifest.channels.swap(0, 1);
        manifest.start_ns = i64::MAX;
        assert!(validate_manifest(&manifest, 40).is_err());
        manifest.start_ns = 0;
        manifest.tick_count = 0;
        assert!(validate_manifest(&manifest, 0).is_err());
        Ok(())
    }

    #[test]
    fn test_mock_in_decode_tick_little_endian() -> Result<()> {
        // -- Setup & Fixtures
        let values = [1.0_f32, -2.0, 3.5, 4.0, 9.81];
        let mut bytes = [0_u8; 20];
        for (index, value) in values.into_iter().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }

        // -- Exec
        let tick = decode_tick(&bytes, 123)?;

        // -- Check
        assert_eq!(tick.ts_ns, 123);
        assert_eq!(tick.v, values);
        Ok(())
    }

    #[test]
    fn test_mock_in_decode_tick_non_finite() -> Result<()> {
        // -- Setup & Fixtures
        let invalid_values = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];

        // -- Exec & Check
        for value in invalid_values {
            for channel in 0..5 {
                let mut bytes = [0_u8; 20];
                bytes[channel * 4..channel * 4 + 4].copy_from_slice(&value.to_le_bytes());
                let error = decode_tick(&bytes, 123).err().ok_or("expected decoding failure")?;
                let message = error.to_string();
                assert!(message.contains("non-finite sensor value"));
                assert!(message.contains(&format!("channel {channel}")));
            }
        }
        Ok(())
    }

    #[test]
    fn test_mock_in_next_tick_file_replay() -> Result<()> {
        // -- Setup & Fixtures
        let directory = fixture_directory("test_mock_in_next_tick_file_replay")?;
        let mut input = MockInput::open(&directory)?;

        // -- Exec
        let first = input.next_tick()?.ok_or("expected first tick")?;
        let second = input.next_tick()?.ok_or("expected second tick")?;
        let end = input.next_tick()?;

        // -- Check
        assert_eq!(input.tick_count(), 2);
        assert_eq!(first.ts_ns, 0);
        assert_eq!(first.v, [1.0_f32, 2.0, 3.0, 4.0, 9.81]);
        assert_eq!(second.ts_ns, 5_000_000);
        assert_eq!(second.v, [5.0_f32; 5]);
        assert!(end.is_none());
        Ok(())
    }

    #[test]
    fn test_mock_in_with_tick_limit_stops_early() -> Result<()> {
        // -- Setup & Fixtures
        let directory = fixture_directory("test_mock_in_with_tick_limit_stops_early")?;

        // -- Exec
        let mut input = MockInput::open(&directory)?.with_tick_limit(1)?;
        let first = input.next_tick()?;
        let end = input.next_tick()?;

        // -- Check
        assert_eq!(input.tick_count(), 1);
        assert!(first.is_some());
        assert!(end.is_none());
        assert!(MockInput::open(&directory)?.with_tick_limit(3).is_err());
        Ok(())
    }

    #[test]
    fn test_mock_in_open_missing_required_files() -> Result<()> {
        // -- Setup & Fixtures
        let filenames = ["manifest.json", "ticks.bin", "story.jsonl"];

        // -- Exec & Check
        for filename in filenames {
            let directory =
                fixture_directory("test_mock_in_open_missing_required_files")?;
            std::fs::rename(
                directory.join(filename),
                directory.join(format!("{filename}.saved")),
            )?;
            assert!(
                MockInput::open(&directory).is_err(),
                "missing {filename} must reject input"
            );
        }
        Ok(())
    }

    #[test]
    fn test_mock_in_next_tick_truncated_after_open() -> Result<()> {
        // -- Setup & Fixtures
        let directory =
            fixture_directory("test_mock_in_next_tick_truncated_after_open")?;
        let mut input = MockInput::open(&directory)?;
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(directory.join("ticks.bin"))?;
        file.set_len(19)?;

        // -- Exec
        let result = input.next_tick();

        // -- Check
        assert!(matches!(
            result,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof
        ));
        Ok(())
    }

    #[test]
    fn test_mock_in_next_tick_grown_after_open() -> Result<()> {
        // -- Setup & Fixtures
        let directory = fixture_directory("test_mock_in_next_tick_grown_after_open")?;
        let mut input = MockInput::open(&directory)?;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(directory.join("ticks.bin"))?;
        std::io::Write::write_all(&mut file, &[0])?;

        // -- Exec
        let first = input.next_tick()?;
        let second = input.next_tick()?;
        let result = input.next_tick();

        // -- Check
        assert!(first.is_some());
        assert!(second.is_some());
        let error = result.err().ok_or("expected trailing-byte rejection")?;
        assert!(error.to_string().contains("grew after startup validation"));
        Ok(())
    }

    // -- Test Support
    fn fixture_directory(test_name: &str) -> Result<SPath> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let parent = SPath::new("tests-data/.tmp").join(test_name);
        std::fs::create_dir_all(&parent)?;
        let directory = parent.join(format!("{}-{unique}", std::process::id()));
        std::fs::create_dir(&directory)?;
        let manifest = serde_json::json!({
            "version": 1,
            "seed": 42,
            "rate_hz": 200,
            "channels": CHANNELS,
            "tick_count": 2,
            "start_ns": 0,
            "bytes_per_tick": 20
        });
        std::fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&manifest)?,
        )?;
        let mut bytes = Vec::with_capacity(40);
        for values in [[1.0_f32, 2.0, 3.0, 4.0, 9.81], [5.0_f32; 5]] {
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        std::fs::write(directory.join("ticks.bin"), bytes)?;
        std::fs::write(directory.join("story.jsonl"), b"")?;
        Ok(directory)
    }

    fn fixture_manifest() -> Manifest {
        Manifest {
            version: 1,
            seed: 42,
            rate_hz: 200,
            channels: CHANNELS.map(str::to_string).into(),
            tick_count: 2,
            start_ns: 0,
            bytes_per_tick: 20,
        }
    }
}

// endregion: --- Tests
