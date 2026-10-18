use crate::recorder::{Recorder, Tick, CHANNEL_COUNT};
use crate::{Error, Result};
use simple_fs::SPath;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};

const RECORD_BYTES: usize = 8 + CHANNEL_COUNT * 4;
const TICKS_PER_SEGMENT: u64 = 3_600 * 200;
const BUFFER_CAPACITY: usize = 1_048_576;

pub struct RawRecorder {
	directory: SPath,
	writer: Option<BufWriter<File>>,
	segment_index: Option<u64>,
	segment_paths: Vec<SPath>,
	ticks_written: u64,
}

impl RawRecorder {
	pub fn new(directory: &SPath) -> Self {
		Self {
			directory: directory.clone(),
			writer: None,
			segment_index: None,
			segment_paths: Vec::new(),
			ticks_written: 0,
		}
	}
}

impl Recorder for RawRecorder {
	fn write_batch(&mut self, ticks: &[Tick]) -> Result<()> {
		for tick in ticks {
			let segment_index = self.ticks_written / TICKS_PER_SEGMENT;
			if self.segment_index != Some(segment_index) {
				self.open_segment(segment_index)?;
			}

			let next_ticks_written = self
				.ticks_written
				.checked_add(1)
				.ok_or_else(|| Error::custom("raw recording tick count overflow"))?;
			let mut record = [0_u8; RECORD_BYTES];
			record[..8].copy_from_slice(&tick.epoch_micro.to_le_bytes());
			for (index, value) in tick.v.iter().enumerate() {
				let start = 8 + index * 4;
				record[start..start + 4].copy_from_slice(&value.to_le_bytes());
			}

			let writer = self
				.writer
				.as_mut()
				.ok_or_else(|| Error::custom("raw recording segment is not open"))?;
			writer.write_all(&record)?;
			self.ticks_written = next_ticks_written;
		}

		Ok(())
	}

	fn finish(mut self: Box<Self>) -> Result<u64> {
		self.flush_current()?;

		let mut output_bytes = 0_u64;
		for path in &self.segment_paths {
			output_bytes = output_bytes
				.checked_add(std::fs::metadata(path)?.len())
				.ok_or_else(|| Error::custom("raw recording size overflow"))?;
		}

		Ok(output_bytes)
	}
}

// region:    --- Support

impl RawRecorder {
	fn open_segment(&mut self, segment_index: u64) -> Result<()> {
		self.flush_current()?;

		let path = self.directory.join(format!("seg-{segment_index:06}.bin"));
		let file = OpenOptions::new().write(true).create_new(true).open(&path)?;
		self.writer = Some(BufWriter::with_capacity(BUFFER_CAPACITY, file));
		self.segment_index = Some(segment_index);
		self.segment_paths.push(path);

		Ok(())
	}

	fn flush_current(&mut self) -> Result<()> {
		if let Some(writer) = self.writer.as_mut() {
			writer.flush()?;
		}

		Ok(())
	}
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

	use super::*;

	#[test]
	fn test_rec_raw_write_little_endian_record_simple() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_directory("test_rec_raw_write_little_endian_record_simple")?;
		let tick = Tick {
			epoch_micro: -5,
			v: [1.0, -2.0, 3.5, 4.0, 9.81, -6.0, 0.5, -1.25, 2.75, -0.125],
		};
		let mut expected = tick.epoch_micro.to_le_bytes().to_vec();
		for value in tick.v {
			expected.extend_from_slice(&value.to_le_bytes());
		}
		let mut recorder = RawRecorder::new(&directory);

		// -- Exec
		recorder.write_batch(&[tick])?;
		let output_bytes = Box::new(recorder).finish()?;
		let actual = std::fs::read(directory.join("seg-000000.bin"))?;

		// -- Check
		assert_eq!(actual, expected);
		assert_eq!(output_bytes, RECORD_BYTES as u64);
		Ok(())
	}

	#[test]
	fn test_rec_raw_rolls_segment_inside_batch() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_directory("test_rec_raw_rolls_segment_inside_batch")?;
		let tick = Tick {
			epoch_micro: 0,
			v: [1.0; CHANNEL_COUNT],
		};
		let mut recorder = RawRecorder::new(&directory);
		let batch = vec![tick; 8192];
		let mut ticks_remaining = TICKS_PER_SEGMENT - 1;

		// -- Exec
		while ticks_remaining > 0 {
			let count = ticks_remaining.min(batch.len() as u64) as usize;
			recorder.write_batch(&batch[..count])?;
			ticks_remaining -= count as u64;
		}
		recorder.write_batch(&[tick, tick])?;

		// -- Check
		let first_segment = directory.join("seg-000000.bin");
		assert_eq!(
			std::fs::metadata(&first_segment)?.len(),
			TICKS_PER_SEGMENT * RECORD_BYTES as u64
		);
		let output_bytes = Box::new(recorder).finish()?;
		let second_segment = directory.join("seg-000001.bin");
		assert_eq!(std::fs::metadata(second_segment)?.len(), RECORD_BYTES as u64);
		assert_eq!(output_bytes, (TICKS_PER_SEGMENT + 1) * RECORD_BYTES as u64);
		assert_eq!(std::fs::read_dir(directory)?.count(), 2);
		Ok(())
	}

	// -- Test Support

	fn fixture_directory(test_name: &str) -> Result<SPath> {
		let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
		let parent = SPath::new("tests-data/.tmp").join(test_name);
		std::fs::create_dir_all(&parent)?;
		let directory = parent.join(format!("{}-{unique}", std::process::id()));
		std::fs::create_dir(&directory)?;
		Ok(directory)
	}
}

// endregion: --- Tests
