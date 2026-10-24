type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

use crate::sim::{MockInput, BYTES_PER_TICK, CHANNELS, TICK_PERIOD_US};
use lib_common::channels::CHANNEL_COUNT;
use simple_fs::SPath;
use std::fs;

pub const FIXTURE_START_US: i64 = 1_792_022_400_000_000;

pub fn fixture_input(test_name: &str, tick_count: u64) -> Result<MockInput> {
	let directory = fixture_dataset(test_name, tick_count)?;
	Ok(MockInput::open(&directory)?)
}

pub fn fixture_dataset(test_name: &str, tick_count: u64) -> Result<SPath> {
	let directory = fixture_directory(test_name)?;
	let manifest = serde_json::json!({
		"version": 3,
		"seed": 42,
		"rate_hz": 200,
		"channels": CHANNELS,
		"tick_count": tick_count,
		"start_epoch_us": FIXTURE_START_US,
		"tick_period_us": TICK_PERIOD_US,
		"bytes_per_tick": BYTES_PER_TICK
	});
	fs::write(directory.join("manifest.json"), serde_json::to_vec(&manifest)?)?;
	let mut output = std::io::BufWriter::new(fs::File::create(directory.join("ticks.bin"))?);
	for index in 0..tick_count {
		for channel in 0..CHANNEL_COUNT {
			let value = (index * CHANNEL_COUNT as u64 + channel as u64) as f32;
			std::io::Write::write_all(&mut output, &value.to_le_bytes())?;
		}
	}
	std::io::Write::flush(&mut output)?;
	drop(output);
	fs::write(directory.join("story.jsonl"), b"")?;
	Ok(directory)
}

pub fn fixture_directory(test_name: &str) -> Result<SPath> {
	let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
	let parent = SPath::new("tests-data/.tmp").join(test_name);
	fs::create_dir_all(&parent)?;
	let directory = parent.join(format!("{}-{unique}", std::process::id()));
	fs::create_dir(&directory)?;
	Ok(directory)
}

pub fn fixture_directory_at(test_name: &str) -> Rc<...>
