use crate::error::{Error, Result};
use crate::story::Story;
use crate::synth::Synth;
use crate::{BYTES_PER_TICK, CHANNELS, CHANNEL_COUNT, RATE_HZ};
use serde::Serialize;
use simple_fs::SPath;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};

const CHUNK_TICKS: usize = RATE_HZ as usize;
const HOUR_TICKS: u64 = 3_600 * RATE_HZ as u64;
const MANIFEST_VERSION: u32 = 3;
const TICK_PERIOD_US: i64 = 5_000;

#[derive(Serialize)]
struct Manifest {
	version: u32,
	seed: u64,
	rate_hz: u32,
	channels: [&'static str; CHANNEL_COUNT],
	tick_count: u64,
	start_epoch_us: i64,
	tick_period_us: i64,
	bytes_per_tick: usize,
}

fn create_new(path: &SPath) -> Result<File> {
	Ok(OpenOptions::new().write(true).create_new(true).open(path)?)
}

pub fn generate(out: &SPath, seed: u64, tick_count: u64, start_epoch_us: i64) -> Result<()> {
	check_timestamp_range(start_epoch_us, tick_count)?;
	fs::create_dir_all(out)?;
	if fs::read_dir(out)?.next().transpose()?.is_some() {
		return Err(Error::Invalid(format!(
			"output directory {} is not empty; use a new or empty directory",
			out
		)));
	}

	let mut ticks = BufWriter::with_capacity(1_048_576, create_new(&out.join("ticks.bin"))?);
	let mut story_output = BufWriter::with_capacity(65_536, create_new(&out.join("story.jsonl"))?);
	let mut story = Story::new(seed, tick_count);
	let mut synth = Synth::new(seed);
	let mut chunk = [0_u8; CHUNK_TICKS * BYTES_PER_TICK];
	let mut written = 0_u64;

	while let Some(segment) = story.next_segment() {
		serde_json::to_writer(&mut story_output, &segment)?;
		story_output.write_all(b"\n")?;
		synth.begin_segment(&segment);
		let mut tick = segment.start_tick;
		while tick < segment.end_tick {
			let until_hour = HOUR_TICKS - written % HOUR_TICKS;
			let count = (segment.end_tick - tick).min(CHUNK_TICKS as u64).min(until_hour) as usize;
			for offset in 0..count {
				let values = synth.sample(tick + offset as u64, &segment);
				for (channel, value) in values.into_iter().enumerate() {
					let start = offset * BYTES_PER_TICK + channel * 4;
					chunk[start..start + 4].copy_from_slice(&value.to_le_bytes());
				}
			}
			ticks.write_all(&chunk[..count * BYTES_PER_TICK])?;
			tick += count as u64;
			written += count as u64;
			if written.is_multiple_of(HOUR_TICKS) {
				eprintln!(
					"Generated {} simulated hours ({written}/{tick_count} ticks).",
					written / HOUR_TICKS
				);
			}
		}
	}

	ticks.flush()?;
	story_output.flush()?;
	ticks.get_ref().sync_all()?;
	story_output.get_ref().sync_all()?;
	drop(ticks);
	drop(story_output);

	let manifest = Manifest {
		version: MANIFEST_VERSION,
		seed,
		rate_hz: RATE_HZ,
		channels: CHANNELS,
		tick_count,
		start_epoch_us,
		tick_period_us: TICK_PERIOD_US,
		bytes_per_tick: BYTES_PER_TICK,
	};
	let temporary_path = out.join(".manifest.json.tmp");
	let mut temporary = create_new(&temporary_path)?;
	serde_json::to_writer_pretty(&mut temporary, &manifest)?;
	temporary.write_all(b"\n")?;
	temporary.sync_all()?;
	drop(temporary);

	// Publish the completed manifest without replacing an existing pathname.
	// This requires hard-link support; it is not a crash-durability guarantee.
	fs::hard_link(&temporary_path, out.join("manifest.json"))?;
	if let Err(error) = fs::remove_file(&temporary_path) {
		eprintln!("mocker: dataset completed, but temporary manifest cleanup failed: {error}");
	}
	Ok(())
}

// region:    --- Support

fn check_timestamp_range(start_epoch_us: i64, tick_count: u64) -> Result<()> {
	if start_epoch_us <= 0 {
		return Err(Error::Invalid(format!(
			"start epoch {start_epoch_us} us must be positive"
		)));
	}
	i64::try_from(tick_count)
		.ok()
		.and_then(|ticks| ticks.checked_mul(TICK_PERIOD_US))
		.and_then(|span| start_epoch_us.checked_add(span))
		.ok_or_else(|| Error::Invalid("dataset timestamps exceed the signed 64-bit microsecond range".into()))?;
	Ok(())
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

	use super::*;

	#[test]
	fn test_writer_generate_manifest_start_us() -> Result<()> {
		// -- Setup & Fixtures
		let out = fixture_directory("test_writer_generate_manifest_start_us")?.join("dataset");
		let tick_count = 2_000_u64;

		// -- Exec
		generate(&out, 42, tick_count, 1_000)?;
		let manifest: serde_json::Value = serde_json::from_slice(&fs::read(out.join("manifest.json"))?)?;

		// -- Check
		let channels = manifest["channels"].as_array().ok_or("channels should be an array")?;
		let actual_channels: Vec<&str> = channels.iter().filter_map(|value| value.as_str()).collect();
		assert_eq!(manifest["version"], 3);
		assert_eq!(manifest["bytes_per_tick"], BYTES_PER_TICK);
		assert_eq!(actual_channels, CHANNELS);
		assert_eq!(manifest["start_epoch_us"], 1_000);
		assert_eq!(manifest["tick_period_us"], 5_000);
		assert_eq!(manifest["tick_count"], tick_count);
		assert!(manifest.get("start_ns").is_none());
		assert_eq!(
			fs::metadata(out.join("ticks.bin"))?.len(),
			tick_count * BYTES_PER_TICK as u64
		);
		Ok(())
	}

	#[test]
	fn test_writer_generate_invalid_start_us() -> Result<()> {
		// -- Setup & Fixtures
		let parent = fixture_directory("test_writer_generate_invalid_start_us")?;
		let cases = [0_i64, -1, i64::MAX - TICK_PERIOD_US];

		// -- Exec & Check
		for (index, start_epoch_us) in cases.into_iter().enumerate() {
			let out = parent.join(format!("dataset-{index}"));
			assert!(
				generate(&out, 42, 2, start_epoch_us).is_err(),
				"start epoch must be rejected: {start_epoch_us}"
			);
			assert!(fs::metadata(&out).is_err(), "rejected dataset must not create {out}");
		}
		Ok(())
	}

	#[test]
	fn test_writer_generate_story_matches_fixture() -> Result<()> {
		// -- Setup & Fixtures
		// Skips until the fixture is captured from the pre-change build (see plan.md).
		let Some(expected) = crate::_test_support::opt_fixture_bytes(STORY_FIXTURE)? else {
			return Ok(());
		};
		let out = fixture_directory("test_writer_generate_story_matches_fixture")?.join("dataset");

		// -- Exec
		generate(&out, 42, 120_000, 1_792_022_400_000_000)?;
		let actual = fs::read(out.join("story.jsonl"))?;

		// -- Check
		assert_eq!(actual.len(), expected.len());
		assert_eq!(actual, expected);
		Ok(())
	}

	// -- Test Support

	const STORY_FIXTURE: &str = "tests-data/fixtures/story-seed-42-m010.jsonl";

	fn fixture_directory(test_name: &str) -> Result<SPath> {
		let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
		let parent = SPath::new("tests-data/.tmp").join(test_name);
		fs::create_dir_all(&parent)?;
		let directory = parent.join(format!("{}-{unique}", std::process::id()));
		fs::create_dir(&directory)?;
		Ok(directory)
	}
}

// endregion: --- Tests
