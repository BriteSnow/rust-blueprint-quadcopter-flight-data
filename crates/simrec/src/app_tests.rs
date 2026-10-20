// region:    --- Tests

#[cfg(test)]
mod test_cases {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

	use crate::app::*;
	use crate::recorder::CHANNEL_COUNT;
	use crate::sampler::{CpuSource, SamplerSetup, MAIN_OSTEL_FILE_NAME};
	use crate::sim::ReplaySpeed;
	use crate::{mock_in, recorder, sim};
	use std::sync::Arc;

	#[test]
	fn test_main_prepare_output_new_and_empty() -> Result<()> {
		// -- Setup & Fixtures
		let parent = fixture_directory("test_main_prepare_output_new_and_empty")?;
		let directory = parent.join("noop");

		// -- Exec
		prepare_output(&directory)?;
		prepare_output(&directory)?;

		// -- Check
		assert!(fs::metadata(&directory)?.is_dir());
		assert_eq!(fs::read_dir(&directory)?.count(), 0);
		Ok(())
	}

	#[test]
	fn test_main_prepare_output_nonempty_retains_artifacts() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_directory("test_main_prepare_output_nonempty_retains_artifacts")?;
		let artifact = directory.join("capture.bin");
		let original = b"existing recording";
		fs::write(&artifact, original)?;

		// -- Exec
		let result = prepare_output(&directory);

		// -- Check
		let error = result.err().ok_or("expected output refusal")?;
		assert!(error.to_string().contains("is not empty"));
		assert_eq!(fs::read(&artifact)?, original);
		assert_eq!(fs::read_dir(&directory)?.count(), 1);
		Ok(())
	}

	#[test]
	fn test_main_prepare_output_file_destination() -> Result<()> {
		// -- Setup & Fixtures
		let parent = fixture_directory("test_main_prepare_output_file_destination")?;
		let destination = parent.join("noop");
		let original = b"existing file";
		fs::write(&destination, original)?;

		// -- Exec
		let result = prepare_output(&destination);

		// -- Check
		assert!(result.is_err());
		assert_eq!(fs::read(&destination)?, original);
		Ok(())
	}

	#[test]
	fn test_main_replay_workers_conservation_and_order() -> Result<()> {
		// -- Setup & Fixtures
		let tick_count = 20_000;
		let input = fixture_input("test_main_replay_workers_conservation_and_order", tick_count)?;
		let observed = Arc::new(std::sync::Mutex::new(Observed::default()));
		let recorder = ObservingRecorder {
			observed: Arc::clone(&observed),
			failure: Failure::None,
		};

		// -- Exec
		let (produced, recorded) = replay_workers(
			input,
			Box::new(recorder),
			ReplaySpeed::Max,
			Duration::from_secs(60),
			Some(rec::MAX_BATCH_TICKS),
			1,
			None,
		)?;

		// -- Check
		let observed = observed.lock().map_err(|_| "observation lock poisoned")?;
		assert_eq!(produced.ticks_total, tick_count);
		assert_eq!(recorded.ticks_total, tick_count);
		assert_eq!(observed.ticks, tick_count);
		assert!(observed.finished);
		assert!(recorded.flushes.count() > 0);
		assert!(produced.max_pending_len <= sim::MAX_PENDING_TICKS);
		Ok(())
	}

	#[test]
	fn test_main_replay_workers_write_failure_shutdown() -> Result<()> {
		// -- Setup & Fixtures
		let input = fixture_input("test_main_replay_workers_write_failure_shutdown", 20_000)?;
		let observed = Arc::new(std::sync::Mutex::new(Observed::default()));
		let recorder = ObservingRecorder {
			observed: Arc::clone(&observed),
			failure: Failure::Error,
		};

		// -- Exec
		let result = replay_workers(
			input,
			Box::new(recorder),
			ReplaySpeed::Max,
			Duration::ZERO,
			Some(rec::MAX_BATCH_TICKS),
			1,
			None,
		);

		// -- Check
		let error = result.err().ok_or("expected recorder failure")?;
		assert!(matches!(error, Error::Custom(message) if message == "injected write failure"));
		let observed = observed.lock().map_err(|_| "observation lock poisoned")?;
		assert!(!observed.finished);
		assert_eq!(observed.ticks, 0);
		Ok(())
	}

	#[test]
	fn test_main_replay_workers_finish_failure_shutdown() -> Result<()> {
		// -- Setup & Fixtures
		let tick_count = 20_000;
		let input = fixture_input("test_main_replay_workers_finish_failure_shutdown", tick_count)?;
		let observed = Arc::new(std::sync::Mutex::new(Observed::default()));
		let recorder = ObservingRecorder {
			observed: Arc::clone(&observed),
			failure: Failure::FinishError,
		};

		// -- Exec
		let result = replay_workers(
			input,
			Box::new(recorder),
			ReplaySpeed::Max,
			Duration::from_secs(60),
			Some(rec::MAX_BATCH_TICKS),
			1,
			None,
		);

		// -- Check
		let error = result.err().ok_or("expected finalization failure")?;
		assert!(matches!(
			error,
			Error::Custom(message) if message == "injected finish failure"
		));
		let observed = observed.lock().map_err(|_| "observation lock poisoned")?;
		assert_eq!(observed.ticks, tick_count);
		assert!(!observed.finished);
		Ok(())
	}

	#[test]
	fn test_main_replay_workers_recorder_panic_shutdown() -> Result<()> {
		// -- Setup & Fixtures
		let input = fixture_input("test_main_replay_workers_recorder_panic_shutdown", 20_000)?;
		let observed = Arc::new(std::sync::Mutex::new(Observed::default()));
		let recorder = ObservingRecorder {
			observed: Arc::clone(&observed),
			failure: Failure::Panic,
		};

		// -- Exec
		let result = replay_workers(
			input,
			Box::new(recorder),
			ReplaySpeed::Max,
			Duration::ZERO,
			Some(rec::MAX_BATCH_TICKS),
			1,
			None,
		);

		// -- Check
		let error = result.err().ok_or("expected worker panic failure")?;
		assert!(matches!(error, Error::Custom(message) if message == "consumer thread panicked"));
		let observed = observed.lock().map_err(|_| "observation lock poisoned")?;
		assert!(!observed.finished);
		assert_eq!(observed.ticks, 0);
		Ok(())
	}

	#[test]
	fn test_main_replay_workers_producer_decode_failure_shutdown() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_dataset("test_main_replay_workers_producer_decode_failure_shutdown", 20_000)?;
		let input = MockInput::open(&directory)?;
		let mut file = fs::OpenOptions::new().write(true).open(directory.join("ticks.bin"))?;
		std::io::Write::write_all(&mut file, &f32::NAN.to_le_bytes())?;
		drop(file);
		let observed = Arc::new(std::sync::Mutex::new(Observed::default()));
		let recorder = ObservingRecorder {
			observed: Arc::clone(&observed),
			failure: Failure::None,
		};

		// -- Exec
		let result = replay_workers(
			input,
			Box::new(recorder),
			ReplaySpeed::Max,
			Duration::from_millis(10),
			Some(rec::MAX_BATCH_TICKS),
			1,
			None,
		);

		// -- Check
		let error = result.err().ok_or("expected producer decoding failure")?;
		assert!(matches!(
			error,
			Error::Custom(message) if message.contains("non-finite sensor value")
		));
		let observed = observed.lock().map_err(|_| "observation lock poisoned")?;
		assert_eq!(observed.ticks, 0);
		assert!(!observed.finished);
		Ok(())
	}

	#[test]
	fn test_main_replay_workers_producer_read_failure_shutdown() -> Result<()> {
		// -- Setup & Fixtures
		let directory = fixture_dataset("test_main_replay_workers_producer_read_failure_shutdown", 20_000)?;
		let input = MockInput::open(&directory)?;
		let file = fs::OpenOptions::new().write(true).open(directory.join("ticks.bin"))?;
		file.set_len(0)?;
		drop(file);
		let observed = Arc::new(std::sync::Mutex::new(Observed::default()));
		let recorder = ObservingRecorder {
			observed: Arc::clone(&observed),
			failure: Failure::None,
		};

		// -- Exec
		let result = replay_workers(
			input,
			Box::new(recorder),
			ReplaySpeed::Max,
			Duration::from_millis(10),
			Some(rec::MAX_BATCH_TICKS),
			1,
			None,
		);

		// -- Check
		let error = result.err().ok_or("expected producer read failure")?;
		assert!(matches!(
			error,
			Error::Io(error) if error.kind() == std::io::ErrorKind::UnexpectedEof
		));
		let observed = observed.lock().map_err(|_| "observation lock poisoned")?;
		assert_eq!(observed.ticks, 0);
		assert!(!observed.finished);
		Ok(())
	}

	#[test]
	fn test_main_merge_worker_results_recorder_failure_over_disconnection() -> Result<()> {
		// -- Setup & Fixtures
		let produced = Err(Error::EventBase(
			lib_common::event_base::EventBaseError::TxDisconnected { name: "sensor-capture" },
		));
		let recorded = Err(Error::custom("injected write failure"));

		// -- Exec
		let result = merge_worker_results(produced, recorded);

		// -- Check
		let error = result.err().ok_or("expected originating recorder failure")?;
		assert!(matches!(
			error,
			Error::Custom(message) if message == "injected write failure"
		));
		Ok(())
	}

	#[test]
	fn test_main_merge_worker_results_producer_failure_over_cancellation() -> Result<()> {
		// -- Setup & Fixtures
		let produced = Err(Error::Io(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)));
		let recorded = Err(Error::Cancelled);

		// -- Exec
		let result = merge_worker_results(produced, recorded);

		// -- Check
		assert!(matches!(
			result,
			Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof
		));
		Ok(())
	}

	#[test]
	fn test_main_replay_workers_raw_timestamps() -> Result<()> {
		// -- Setup & Fixtures
		let tick_count = 2_000_u64;
		let dataset = fixture_dataset("test_main_replay_workers_raw_timestamps", tick_count)?;
		let input = MockInput::open(&dataset)?;
		let out = fixture_directory("test_main_replay_workers_raw_timestamps_out")?;

		// -- Exec
		let (_, recorded) = replay_workers(
			input,
			Box::new(RawRecorder::new(&out)),
			ReplaySpeed::Max,
			Duration::from_millis(10),
			Some(rec::MAX_BATCH_TICKS),
			16,
			None,
		)?;
		let bytes = fs::read(out.join("capture-main-seg-000000.bin"))?;
		let ticks = fs::read(dataset.join("ticks.bin"))?;

		// -- Check
		assert_eq!(recorded.ticks_total, tick_count);
		assert_eq!(bytes.len() as u64, tick_count * RAW_RECORD_BYTES as u64);
		let first = bytes.get(..8).ok_or("expected a first record")?;
		assert_eq!(i64::from_le_bytes(first.try_into()?), FIXTURE_START_US);
		let first_payload = bytes.get(8..RAW_RECORD_BYTES).ok_or("expected a first record payload")?;
		let first_tick = ticks.get(..TICK_BYTES).ok_or("expected a first input tick")?;
		assert_eq!(first_payload, first_tick);
		for (index, record) in bytes.as_chunks::<RAW_RECORD_BYTES>().0.iter().enumerate() {
			let ts_us = i64::from_le_bytes(record[..8].try_into()?);
			assert_eq!(ts_us, FIXTURE_START_US + index as i64 * mock_in::TICK_PERIOD_US);
		}
		Ok(())
	}

	#[test]
	fn test_main_replay_workers_sqlite_timestamps() -> Result<()> {
		// -- Setup & Fixtures
		let tick_count = 2_000_u64;
		let dataset = fixture_dataset("test_main_replay_workers_sqlite_timestamps", tick_count)?;
		let input = MockInput::open(&dataset)?;
		let out = fixture_directory("test_main_replay_workers_sqlite_timestamps_out")?;

		// -- Exec
		let (_, recorded) = replay_workers(
			input,
			Box::new(SqliteRecorder::new(&out, false)?),
			ReplaySpeed::Max,
			Duration::from_millis(10),
			Some(rec::MAX_BATCH_TICKS),
			16,
			None,
		)?;
		let connection = rusqlite::Connection::open(out.join("capture-main.sqlite"))?;
		let (min_ts_us, max_ts_us, count): (i64, i64, i64) = connection.query_row(
			"SELECT MIN(ts_epoch_micro), MAX(ts_epoch_micro), COUNT(*) FROM samples",
			[],
			|row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
		)?;
		let first_values: [f64; CHANNEL_COUNT] = connection.query_row(
			"SELECT rotor_fl_rpm, rotor_fr_rpm, rotor_rl_rpm, rotor_rr_rpm, accel_x_mps2, accel_y_mps2, accel_z_mps2, gyro_x_rads, gyro_y_rads, gyro_z_rads FROM samples WHERE ts_epoch_micro = ?1",
			[FIXTURE_START_US],
			|row| {
				let mut values = [0.0_f64; CHANNEL_COUNT];
				for (index, value) in values.iter_mut().enumerate() {
					*value = row.get(index)?;
				}
				Ok(values)
			},
		)?;
		let ticks = fs::read(dataset.join("ticks.bin"))?;
		let first_tick = ticks.get(..TICK_BYTES).ok_or("expected a first input tick")?;
		let mut expected = [0.0_f32; CHANNEL_COUNT];
		for (channel, value) in expected.iter_mut().enumerate() {
			let start = channel * 4;
			*value = f32::from_le_bytes(first_tick[start..start + 4].try_into()?);
		}

		// -- Check
		assert_eq!(recorded.ticks_total, tick_count);
		assert_eq!(count, tick_count as i64);
		assert_eq!(min_ts_us, FIXTURE_START_US);
		assert_eq!(
			max_ts_us,
			FIXTURE_START_US + (tick_count as i64 - 1) * mock_in::TICK_PERIOD_US
		);
		let actual: [f32; CHANNEL_COUNT] = std::array::from_fn(|channel| first_values[channel] as f32);
		assert_eq!(actual, expected);
		Ok(())
	}

	#[test]
	fn test_main_replay_workers_sampler_error_keeps_capture() -> Result<()> {
		// -- Setup & Fixtures
		let tick_count = 20_000;
		let input = fixture_input("test_main_replay_workers_sampler_error_keeps_capture", tick_count)?;
		let out = fixture_directory("test_main_replay_workers_sampler_error_keeps_capture_out")?;
		let samples_path = out.join(MAIN_OSTEL_FILE_NAME);
		let sampler = SamplerSetup {
			source: Box::new(FailingSource),
			main_writer: Box::new(fs::File::create(&samples_path)?),
			sum_writer: None,
			interval: Duration::from_millis(10),
		};
		let observed = Arc::new(std::sync::Mutex::new(Observed::default()));
		let recorder = ObservingRecorder {
			observed: Arc::clone(&observed),
			failure: Failure::None,
		};

		// -- Exec
		let (produced, recorded) = replay_workers(
			input,
			Box::new(recorder),
			ReplaySpeed::Max,
			Duration::from_secs(60),
			Some(rec::MAX_BATCH_TICKS),
			1,
			Some(sampler),
		)?;
		let content = fs::read_to_string(&samples_path)?;

		// -- Check
		assert_eq!(produced.ticks_total, tick_count);
		assert_eq!(recorded.ticks_total, tick_count);
		let lines: Vec<&str> = content.lines().collect();
		assert_eq!(lines.len(), 1);
		let value: serde_json::Value = serde_json::from_str(lines[0])?;
		assert_eq!(value["kind"], "error");
		Ok(())
	}

	#[cfg(target_os = "macos")]
	#[test]
	fn test_main_replay_workers_cpu_samples_written() -> Result<()> {
		use crate::sampler::OsCpuSource;
		use std::collections::HashSet;

		// -- Setup & Fixtures
		let input = fixture_input("test_main_replay_workers_cpu_samples_written", 100)?;
		let out = fixture_directory("test_main_replay_workers_cpu_samples_written_out")?;
		let samples_path = out.join(MAIN_OSTEL_FILE_NAME);
		let sampler = SamplerSetup {
			source: Box::new(OsCpuSource::current()),
			main_writer: Box::new(fs::File::create(&samples_path)?),
			sum_writer: None,
			interval: Duration::from_millis(50),
		};

		// -- Exec
		replay_workers(
			input,
			Box::new(NoopRecorder),
			ReplaySpeed::Realtime,
			Duration::from_millis(10),
			Some(rec::MAX_BATCH_TICKS),
			16,
			Some(sampler),
		)?;
		let content = fs::read_to_string(&samples_path)?;

		// -- Check
		let mut labels: HashSet<String> = HashSet::new();
		let mut has_process = false;
		for line in content.lines() {
			let value: serde_json::Value = serde_json::from_str(line)?;
			match value["kind"].as_str() {
				Some("process") => has_process = true,
				Some("thread") => {
					let label = value["thread"].as_str().ok_or("thread line without label")?;
					if labels.insert(label.to_string()) {
						assert!(
							value["cpu_pct"].is_null(),
							"first sample of {label} must have null cpu_pct"
						);
					}
					if let Some(cpu_pct) = value["cpu_pct"].as_f64() {
						assert!(cpu_pct >= 0.0, "negative cpu_pct for {label}");
					}
				}
				_ => {}
			}
		}
		assert!(labels.contains("simrec-sim"));
		assert!(labels.contains("simrec-rec"));
		assert!(has_process);
		Ok(())
	}

	#[test]
	fn test_main_run_replay_sum_rows_conserved() -> Result<()> {
		// -- Setup & Fixtures
		let tick_count = 2_000;
		let input = fixture_input("test_main_run_replay_sum_rows_conserved", tick_count)?;
		let out = fixture_directory("test_main_run_replay_sum_rows_conserved_out")?;
		let observed = Arc::new(std::sync::Mutex::new(Observed::default()));
		let recorder = ObservingRecorder {
			observed: Arc::clone(&observed),
			failure: Failure::None,
		};
		let sum_setup = SumSetup {
			writer: Box::new(SumSqlite::new(&out)?),
			flush_interval: Duration::from_millis(10),
			ring_cap: 16,
		};

		// -- Exec
		let outcome = run_replay(
			input,
			Box::new(recorder),
			ReplayOptions {
				speed: ReplaySpeed::Max,
				flush_interval: Duration::from_secs(60),
				max_batch: Some(rec::MAX_BATCH_TICKS),
				ring_cap: 1,
			},
			None,
			Some(sum_setup),
		)?;

		// -- Check
		let feed = outcome.recorded.sum.ok_or("expected sum feed counters")?;
		let stats = outcome.summary.ok_or("expected summary outcome")??;
		assert_eq!(outcome.recorded.ticks_total, tick_count);
		assert_eq!(feed.rows_emitted, 10);
		assert_eq!(feed.rows_dropped, 0);
		assert_eq!(stats.rows_written, 10);
		assert_eq!(stats.ticks_summarized, tick_count);
		check_sum_conservation(&stats, &feed, tick_count)?;
		Ok(())
	}

	#[test]
	fn test_main_run_replay_sum_write_failure_keeps_capture() -> Result<()> {
		// -- Setup & Fixtures
		let tick_count = 2_000;
		let input = fixture_input("test_main_run_replay_sum_write_failure_keeps_capture", tick_count)?;
		let observed = Arc::new(std::sync::Mutex::new(Observed::default()));
		let recorder = ObservingRecorder {
			observed: Arc::clone(&observed),
			failure: Failure::None,
		};
		let sum_setup = SumSetup {
			writer: Box::new(FailingSumWriter),
			flush_interval: Duration::ZERO,
			ring_cap: 16,
		};

		// -- Exec
		let outcome = run_replay(
			input,
			Box::new(recorder),
			ReplayOptions {
				speed: ReplaySpeed::Max,
				flush_interval: Duration::from_secs(60),
				max_batch: Some(rec::MAX_BATCH_TICKS),
				ring_cap: 1,
			},
			None,
			Some(sum_setup),
		)?;

		// -- Check
		assert_eq!(outcome.recorded.ticks_total, tick_count);
		assert!(observed.lock().map_err(|_| "observation lock poisoned")?.finished);
		let error = outcome
			.summary
			.ok_or("expected summary outcome")?
			.err()
			.ok_or("expected summary failure")?;
		assert!(matches!(error, Error::Custom(message) if message == "injected summary write failure"));
		Ok(())
	}

	// -- Test Support

	const FIXTURE_START_US: i64 = 1_792_022_400_000_000;
	const TICK_BYTES: usize = CHANNEL_COUNT * 4;
	const RAW_RECORD_BYTES: usize = 8 + TICK_BYTES;

	#[derive(Default)]
	struct Observed {
		ticks: u64,
		finished: bool,
	}

	enum Failure {
		None,
		Error,
		FinishError,
		Panic,
	}

	struct ObservingRecorder {
		observed: Arc<std::sync::Mutex<Observed>>,
		failure: Failure,
	}

	impl Recorder for ObservingRecorder {
		fn write_batch(&mut self, ticks: &[recorder::Tick]) -> crate::Result<()> {
			match self.failure {
				Failure::Error => return Err(Error::custom("injected write failure")),
				Failure::Panic => panic!("injected recorder panic"),
				Failure::None | Failure::FinishError => {}
			}
			let mut observed = self.observed.lock().map_err(|_| Error::custom("observation lock poisoned"))?;
			for tick in ticks {
				let index = observed.ticks;
				if tick.epoch_micro != FIXTURE_START_US + index as i64 * mock_in::TICK_PERIOD_US
					|| tick.v
						!= std::array::from_fn(|channel| {
							(index * CHANNEL_COUNT as u64 + channel as u64) as f32
						})
				{
					return Err(Error::custom("recorded tick order or values changed"));
				}
				observed.ticks += 1;
			}
			Ok(())
		}

		fn finish(self: Box<Self>) -> crate::Result<u64> {
			if matches!(self.failure, Failure::FinishError) {
				return Err(Error::custom("injected finish failure"));
			}
			self.observed
				.lock()
				.map_err(|_| Error::custom("observation lock poisoned"))?
				.finished = true;
			Ok(0)
		}
	}

	struct FailingSource;

	impl CpuSource for FailingSource {
		fn threads(&mut self) -> lib_common::os::Result<Vec<lib_common::os::ThreadCpu>> {
			Err(lib_common::os::Error::Unsupported)
		}

		fn process(&mut self) -> lib_common::os::Result<lib_common::os::ProcessStats> {
			Err(lib_common::os::Error::Unsupported)
		}
	}

	struct FailingSumWriter;

	impl crate::sum_sqlite::SumWriter for FailingSumWriter {
		fn write_rows(&mut self, _rows: &[crate::sum_acc::SumRow]) -> crate::Result<()> {
			Err(Error::custom("injected summary write failure"))
		}

		fn finish(self: Box<Self>) -> crate::Result<u64> {
			Ok(0)
		}
	}

	fn fixture_input(test_name: &str, tick_count: u64) -> Result<MockInput> {
		let directory = fixture_dataset(test_name, tick_count)?;
		Ok(MockInput::open(&directory)?)
	}

	fn fixture_dataset(test_name: &str, tick_count: u64) -> Result<SPath> {
		let directory = fixture_directory(test_name)?;
		let manifest = serde_json::json!({
			"version": 3,
			"seed": 42,
			"rate_hz": 200,
			"channels": mock_in::CHANNELS,
			"tick_count": tick_count,
			"start_epoch_us": FIXTURE_START_US,
			"tick_period_us": mock_in::TICK_PERIOD_US,
			"bytes_per_tick": mock_in::BYTES_PER_TICK
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
