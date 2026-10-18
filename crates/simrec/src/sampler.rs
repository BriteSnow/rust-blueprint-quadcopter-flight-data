use crate::support::wait_until;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const CPU_SAMPLES_FILE_NAME: &str = "cpu-samples.jsonl";

pub trait CpuSource: Send {
	fn threads(&mut self) -> lib_common::os::Result<Vec<lib_common::os::ThreadCpu>>;
	fn process(&mut self) -> lib_common::os::Result<lib_common::os::ProcessStats>;
}

pub struct OsCpuSource {
	pid: i32,
}

pub struct SamplerSetup {
	pub source: Box<dyn CpuSource>,
	pub writer: Box<dyn Write + Send>,
	pub interval: Duration,
}

impl OsCpuSource {
	pub fn current() -> Self {
		Self {
			pid: std::process::id() as i32,
		}
	}
}

impl CpuSource for OsCpuSource {
	fn threads(&mut self) -> lib_common::os::Result<Vec<lib_common::os::ThreadCpu>> {
		lib_common::os::thread_cpu_all(self.pid)
	}

	fn process(&mut self) -> lib_common::os::Result<lib_common::os::ProcessStats> {
		lib_common::os::process_stats(self.pid)
	}
}

/// Runs the sampling loop until `stop` is set or the interval wait is cancelled.
///
/// Never fails the capture: errors are written as an `error` line and sampling stops.
pub fn run_sampler(setup: SamplerSetup, stop: &AtomicBool) {
	let SamplerSetup {
		mut source,
		mut writer,
		interval,
	} = setup;
	let mut state = SamplerState::default();
	let mut next = Instant::now();

	loop {
		if stop.load(Ordering::Acquire) {
			return;
		}

		let ts_epoch_us = lib_common::time::now();
		let now = Instant::now();

		match state.sample(source.as_mut(), now, ts_epoch_us) {
			Ok(lines) => {
				if let Err(error) = write_lines(writer.as_mut(), &lines) {
					eprintln!("simrec: cpu sampler write failed: {error}");
					return;
				}
			}
			Err(error) => {
				let line = SampleLine::Error {
					seq: state.seq,
					ts_epoch_us,
					message: error.to_string(),
				};
				let _ = write_lines(writer.as_mut(), &[line]);
				eprintln!("simrec: cpu sampler stopped: {error}");
				return;
			}
		}

		let Some(deadline) = next.checked_add(interval) else {
			return;
		};
		next = deadline;

		if wait_until(next, stop).is_err() {
			return;
		}
	}
}

// region:    --- Support

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum SampleLine {
	Thread {
		seq: u64,
		ts_epoch_us: i64,
		interval_us: u64,
		thread: String,
		user_ns: u64,
		system_ns: u64,
		cpu_pct: Option<f64>,
		sched_usage: f32,
	},
	Process {
		seq: u64,
		ts_epoch_us: i64,
		interval_us: u64,
		user_ns: u64,
		system_ns: u64,
		cpu_pct: Option<f64>,
		rss_bytes: u64,
	},
	Error {
		seq: u64,
		ts_epoch_us: i64,
		message: String,
	},
}

#[derive(Default)]
struct SamplerState {
	seq: u64,
	prev_at: Option<Instant>,
	prev_threads: HashMap<(u64, String), u64>, // (thread_handle, label) -> user_ns + system_ns
	prev_process: Option<u64>,
}

impl SamplerState {
	fn sample(
		&mut self,
		source: &mut dyn CpuSource,
		now: Instant,
		ts_epoch_us: i64,
	) -> lib_common::os::Result<Vec<SampleLine>> {
		let threads = source.threads()?;
		let process = source.process()?;

		let interval = self.prev_at.map(|prev| now.duration_since(prev));
		let interval_us = interval.map_or(0, duration_micros);

		let mut lines = Vec::with_capacity(threads.len() + 1);
		let mut next_threads = HashMap::with_capacity(threads.len());

		for thread in threads {
			let total = thread.user_ns.saturating_add(thread.system_ns);
			let label = thread_label(&thread);
			let key = (thread.thread_handle, label.clone());
			let prev = self.prev_threads.get(&key).copied();
			lines.push(SampleLine::Thread {
				seq: self.seq,
				ts_epoch_us,
				interval_us,
				thread: label,
				user_ns: thread.user_ns,
				system_ns: thread.system_ns,
				cpu_pct: cpu_pct(prev, total, interval),
				sched_usage: thread.sched_usage,
			});
			next_threads.insert(key, total);
		}

		let process_total = process.user_ns.saturating_add(process.system_ns);
		lines.push(SampleLine::Process {
			seq: self.seq,
			ts_epoch_us,
			interval_us,
			user_ns: process.user_ns,
			system_ns: process.system_ns,
			cpu_pct: cpu_pct(self.prev_process, process_total, interval),
			rss_bytes: process.rss_bytes,
		});

		self.prev_threads = next_threads;
		self.prev_process = Some(process_total);
		self.seq += 1;
		self.prev_at = Some(now);

		Ok(lines)
	}
}

fn write_lines(writer: &mut dyn Write, lines: &[SampleLine]) -> std::io::Result<()> {
	for line in lines {
		serde_json::to_writer(&mut *writer, line).map_err(std::io::Error::other)?;
		writer.write_all(b"\n")?;
	}
	writer.flush()
}

fn cpu_pct(prev: Option<u64>, current: u64, interval: Option<Duration>) -> Option<f64> {
	let prev = prev?;
	let interval = interval?;
	if interval.is_zero() {
		return None;
	}
	Some(current.saturating_sub(prev) as f64 / interval.as_nanos() as f64 * 100.0)
}

fn thread_label(thread: &lib_common::os::ThreadCpu) -> String {
	if thread.name.is_empty() {
		format!("tid-{}", thread.thread_handle)
	} else {
		thread.name.clone()
	}
}

fn duration_micros(duration: Duration) -> u64 {
	u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

	use super::*;
	use std::collections::VecDeque;
	use std::io::Write;
	use std::sync::{Arc, Mutex};

	#[test]
	fn test_sampler_sampler_state_sample_first_null() -> Result<()> {
		// -- Setup & Fixtures
		let mut state = SamplerState::default();
		let mut source = FakeSource::default();
		source.thread_script.push_back(Ok(vec![make_thread("simrec-sim", 1, 0)]));
		source.process_script.push_back(Ok(make_process(0)));

		// -- Exec
		let lines = state.sample(&mut source, Instant::now(), 42)?;

		// -- Check
		assert_eq!(lines.len(), 2);
		let thread = serde_json::to_value(&lines[0])?;
		assert_eq!(thread["kind"], "thread");
		assert_eq!(thread["interval_us"], 0);
		assert!(thread["cpu_pct"].is_null());
		let process = serde_json::to_value(&lines[1])?;
		assert_eq!(process["kind"], "process");
		assert!(process["cpu_pct"].is_null());
		Ok(())
	}

	#[test]
	fn test_sampler_sampler_state_sample_cpu_pct() -> Result<()> {
		// -- Setup & Fixtures
		let mut state = SamplerState::default();
		let mut source = FakeSource::default();
		source.thread_script.push_back(Ok(vec![make_thread("simrec-rec", 2, 0)]));
		source
			.thread_script
			.push_back(Ok(vec![make_thread("simrec-rec", 2, 500_000_000)]));
		source.process_script.push_back(Ok(make_process(0)));
		source.process_script.push_back(Ok(make_process(0)));
		let start = Instant::now();
		let later = start
			.checked_add(Duration::from_secs(1))
			.ok_or("test instant exceeds clock range")?;
		state.sample(&mut source, start, 1)?;

		// -- Exec
		let lines = state.sample(&mut source, later, 2)?;

		// -- Check
		let thread = serde_json::to_value(&lines[0])?;
		assert_eq!(thread["interval_us"], 1_000_000);
		assert_eq!(thread["cpu_pct"].as_f64(), Some(50.0));
		Ok(())
	}

	#[test]
	fn test_sampler_sampler_state_sample_unnamed_label() -> Result<()> {
		// -- Setup & Fixtures
		let mut state = SamplerState::default();
		let mut source = FakeSource::default();
		source.thread_script.push_back(Ok(vec![make_thread("", 7, 0)]));
		source.process_script.push_back(Ok(make_process(0)));

		// -- Exec
		let lines = state.sample(&mut source, Instant::now(), 0)?;

		// -- Check
		let thread = serde_json::to_value(&lines[0])?;
		assert_eq!(thread["thread"], "tid-7");
		Ok(())
	}

	#[test]
	fn test_sampler_run_sampler_source_error_line() -> Result<()> {
		// -- Setup & Fixtures
		let stop = AtomicBool::new(false);
		let buf = SharedBuf::default();
		let mut source = FakeSource::default();
		source.thread_script.push_back(Err(lib_common::os::Error::Unsupported));
		let setup = SamplerSetup {
			source: Box::new(source),
			writer: Box::new(buf.clone()),
			interval: Duration::from_millis(1),
		};

		// -- Exec
		run_sampler(setup, &stop);

		// -- Check
		let lines = read_lines(&buf)?;
		assert_eq!(lines.len(), 1);
		let value: serde_json::Value = serde_json::from_str(&lines[0])?;
		assert_eq!(value["kind"], "error");
		assert_eq!(value["seq"], 0);
		Ok(())
	}

	#[test]
	fn test_sampler_run_sampler_stop_preset() -> Result<()> {
		// -- Setup & Fixtures
		let stop = AtomicBool::new(true);
		let buf = SharedBuf::default();
		let setup = SamplerSetup {
			source: Box::new(FakeSource::default()),
			writer: Box::new(buf.clone()),
			interval: Duration::from_millis(1),
		};

		// -- Exec
		run_sampler(setup, &stop);

		// -- Check
		assert!(read_lines(&buf)?.is_empty());
		Ok(())
	}

	// -- Test Support

	#[derive(Default)]
	struct FakeSource {
		thread_script: VecDeque<lib_common::os::Result<Vec<lib_common::os::ThreadCpu>>>,
		process_script: VecDeque<lib_common::os::Result<lib_common::os::ProcessStats>>,
	}

	impl CpuSource for FakeSource {
		fn threads(&mut self) -> lib_common::os::Result<Vec<lib_common::os::ThreadCpu>> {
			self.thread_script.pop_front().unwrap_or(Err(lib_common::os::Error::Unsupported))
		}

		fn process(&mut self) -> lib_common::os::Result<lib_common::os::ProcessStats> {
			self.process_script.pop_front().unwrap_or(Err(lib_common::os::Error::Unsupported))
		}
	}

	#[derive(Clone, Default)]
	struct SharedBuf(Arc<Mutex<Vec<u8>>>);

	impl Write for SharedBuf {
		fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
			self.0
				.lock()
				.map_err(|_| std::io::Error::other("buffer lock poisoned"))?
				.extend_from_slice(buf);
			Ok(buf.len())
		}

		fn flush(&mut self) -> std::io::Result<()> {
			Ok(())
		}
	}

	fn make_thread(name: &str, thread_handle: u64, user_ns: u64) -> lib_common::os::ThreadCpu {
		lib_common::os::ThreadCpu {
			thread_handle,
			name: name.to_string(),
			user_ns,
			system_ns: 0,
			sched_usage: 0.0,
		}
	}

	fn make_process(user_ns: u64) -> lib_common::os::ProcessStats {
		lib_common::os::ProcessStats {
			user_ns,
			system_ns: 0,
			rss_bytes: 0,
		}
	}

	fn read_lines(buf: &SharedBuf) -> Result<Vec<String>> {
		let bytes = buf.0.lock().map_err(|_| "buffer lock poisoned")?.clone();
		Ok(String::from_utf8(bytes)?.lines().map(str::to_owned).collect())
	}
}

// endregion: --- Tests
