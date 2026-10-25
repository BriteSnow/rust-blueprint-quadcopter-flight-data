// region:    --- Tests

#[cfg(test)]
mod test_cases {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

	use crate::_test_support::fixture_dataset;
	use crate::app::{prepare_output, run_capture, Backend, CaptureConfig, SumOptions};
	use crate::cli::{dataset_candidates, duration_tick_count};
	use crate::sim::ReplaySpeed;
	use simple_fs::SPath;
	use std::fs;

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
	fn test_main_run_capture_sql_writes_all_files() -> Result<()> {
		// -- Setup & Fixtures
		let root = fixture_directory("test_main_run_capture_sql_writes_all_files")?;
		let out = root.join("out");
		let ticks = fixture_mock_dataset("test_main_run_capture_sql_writes_all_files", &out)?;

		// -- Exec
		let report = run_capture(capture_config(out, Backend::Sqlite { with_index: false }, true))?;

		// -- Check
		assert_eq!(report.ticks, ticks);
		for name in [
			"capture-main.sqlite",
			"capture-main-metrics.json",
			"capture-main-ostel.jsonl",
			"capture-sum.sqlite",
			"capture-sum-metrics.json",
			"capture-sum-ostel.jsonl",
		] {
			let path = report.out.join(name);
			assert!(
				fs::metadata(&path).is_ok_and(|metadata| metadata.is_file()),
				"{name} must exist"
			);
		}
		Ok(())
	}

	#[test]
	fn test_main_run_capture_no_sum() -> Result<()> {
		// -- Setup & Fixtures
		let root = fixture_directory("test_main_run_capture_no_sum")?;
		let out = root.join("out");
		let ticks = fixture_mock_dataset("test_main_run_capture_no_sum", &out)?;

		// -- Exec
		let report = run_capture(capture_config(out, Backend::Noop, false))?;

		// -- Check
		assert_eq!(report.ticks, ticks);
		let metrics = report.out.join("capture-main-metrics.json");
		assert!(fs::metadata(&metrics).is_ok_and(|metadata| metadata.is_file()));
		for name in ["capture-sum.sqlite", "capture-sum-metrics.json", "capture-sum-ostel.jsonl"] {
			assert!(fs::metadata(report.out.join(name)).is_err(), "{name} must not exist");
		}
		Ok(())
	}

	#[test]
	fn test_main_run_capture_raw_and_noop_write_sum() -> Result<()> {
		// -- Setup & Fixtures
		let backends = [Backend::Raw, Backend::Noop];

		// -- Exec & Check
		for backend in backends {
			let root = fixture_directory("test_main_run_capture_raw_and_noop_write_sum")?;
			let out = root.join("out");
			let ticks = fixture_mock_dataset("test_main_run_capture_raw_and_noop_write_sum", &out)?;
			let report = run_capture(capture_config(out, backend, true))?;

			assert_eq!(report.ticks, ticks);
			let summary = report.out.join("capture-sum.sqlite");
			assert!(
				fs::metadata(&summary).is_ok_and(|metadata| metadata.is_file()),
				"{} must write the summary",
				report.backend
			);
		}
		Ok(())
	}

	// -- Test Support

	fn capture_config(out: SPath, backend: Backend, with_sum: bool) -> CaptureConfig {
		CaptureConfig {
			duration: DURATION.to_string(),
			out,
			speed: ReplaySpeed::Max,
			flush_ms: Some(100),
			ring_cap: 16,
			sample_ms: 100,
			backend,
			sum: with_sum.then_some(SumOptions {
				flush_s: 1,
				ring_cap: 256,
			}),
		}
	}

	fn fixture_mock_dataset(test_name: &str, out: &SPath) -> Result<u64> {
		let ticks = duration_tick_count(DURATION)?;
		let dataset = fixture_dataset(test_name, ticks)?;
		let candidates = dataset_candidates(DURATION)?;
		let name = *candidates.first().ok_or("expected a dataset candidate")?;
		let mock_root = out.join("mock-data");
		fs::create_dir_all(&mock_root)?;
		fs::rename(&dataset, mock_root.join(name))?;
		Ok(ticks)
	}

	fn fixture_directory(test_name: &str) -> Result<SPath> {
		let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
		let parent = SPath::new("tests-data/.tmp").join(test_name);
		fs::create_dir_all(&parent)?;
		let directory = parent.join(format!("{}-{unique}", std::process::id()));
		fs::create_dir(&directory)?;
		Ok(directory)
	}

	const DURATION: &str = "m1";
}

// endregion: --- Tests
