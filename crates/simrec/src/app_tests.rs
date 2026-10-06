// region:    --- Tests

#[cfg(test)]
mod test_cases {
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    use crate::app::*;
    use crate::{mock_in, recorder, sim};
    use crate::sim::ReplaySpeed;
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
        let directory =
            fixture_directory("test_main_prepare_output_nonempty_retains_artifacts")?;
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
        let input = fixture_input(
            "test_main_replay_workers_conservation_and_order",
            tick_count,
        )?;
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
            1,
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
        let input = fixture_input(
            "test_main_replay_workers_write_failure_shutdown",
            20_000,
        )?;
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
            1,
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
        let input = fixture_input(
            "test_main_replay_workers_finish_failure_shutdown",
            tick_count,
        )?;
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
            1,
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
        let input = fixture_input(
            "test_main_replay_workers_recorder_panic_shutdown",
            20_000,
        )?;
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
            1,
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
        let directory = fixture_dataset(
            "test_main_replay_workers_producer_decode_failure_shutdown",
            20_000,
        )?;
        let input = MockInput::open(&directory)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .open(directory.join("ticks.bin"))?;
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
            1,
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
        let directory = fixture_dataset(
            "test_main_replay_workers_producer_read_failure_shutdown",
            20_000,
        )?;
        let input = MockInput::open(&directory)?;
        let file = fs::OpenOptions::new()
            .write(true)
            .open(directory.join("ticks.bin"))?;
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
            1,
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
            lib_common::event_base::EventBaseError::TxDisconnected {
                name: "sensor-capture",
            },
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
        let produced = Err(Error::Io(std::io::Error::from(
            std::io::ErrorKind::UnexpectedEof,
        )));
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

    // -- Test Support

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
            let mut observed = self.observed.lock()
                .map_err(|_| Error::custom("observation lock poisoned"))?;
            for tick in ticks {
                let index = observed.ticks;
                if tick.ts_ns != (index * mock_in::TICK_NS) as i64
                    || tick.v != [index as f32; 5]
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
            self.observed.lock()
                .map_err(|_| Error::custom("observation lock poisoned"))?
                .finished = true;
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
            "version": 1,
            "seed": 42,
            "rate_hz": 200,
            "channels": ["rotor_fl", "rotor_fr", "rotor_rl", "rotor_rr", "accel_z"],
            "tick_count": tick_count,
            "start_ns": 0,
            "bytes_per_tick": mock_in::BYTES_PER_TICK
        });
        fs::write(directory.join("manifest.json"), serde_json::to_vec(&manifest)?)?;
        let mut output = std::io::BufWriter::new(
            fs::File::create(directory.join("ticks.bin"))?,
        );
        for index in 0..tick_count {
            for _ in 0..5 {
                std::io::Write::write_all(&mut output, &(index as f32).to_le_bytes())?;
            }
        }
        std::io::Write::flush(&mut output)?;
        drop(output);
        fs::write(directory.join("story.jsonl"), b"")?;
        Ok(directory)
    }

    fn fixture_directory(test_name: &str) -> Result<SPath> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let parent = SPath::new("tests-data/.tmp").join(test_name);
        fs::create_dir_all(&parent)?;
        let directory = parent.join(format!("{}-{unique}", std::process::id()));
        fs::create_dir(&directory)?;
        Ok(directory)
    }
}

// endregion: --- Tests
