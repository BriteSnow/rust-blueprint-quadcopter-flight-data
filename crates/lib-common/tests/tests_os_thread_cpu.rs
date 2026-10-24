#![cfg(target_os = "macos")]

type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

#[test]
fn test_cpu_thread_cpu_all_named_thread() -> Result<()> {
	// -- Setup & Fixtures
	let (tx, rx) = std::sync::mpsc::channel::<()>();
	let handle = std::thread::Builder::new().name("lib-os-named".into()).spawn(move || {
		let _ = rx.recv();
	})?;

	// -- Exec
	let found = wait_for_thread_named(std::process::id() as i32, "lib-os-named")?;

	// -- Check
	assert!(found, "named thread should be listed with its name");

	// -- Clean up
	let _ = tx.send(());
	handle.join().map_err(|_| "named thread panicked")?;

	Ok(())
}

#[test]
fn test_cpu_process_stats_rss_positive() -> Result<()> {
	// -- Exec
	let stats = lib_common::os::process_stats(std::process::id() as i32)?;

	// -- Check
	assert!(stats.rss_bytes > 0, "process rss should be positive");

	Ok(())
}

#[test]
fn test_cpu_thread_cpu_all_busy_spin_one_second() -> Result<()> {
	// -- Setup & Fixtures
	let pid = std::process::id() as i32;

	// -- Exec
	let handle = std::thread::Builder::new().name("lib-os-spin".into()).spawn(
		move || -> core::result::Result<u64, String> {
			let before = thread_cpu_total_ns(pid, "lib-os-spin")?;
			spin_for(std::time::Duration::from_secs(1));
			let after = thread_cpu_total_ns(pid, "lib-os-spin")?;
			Ok(after.saturating_sub(before))
		},
	)?;
	let delta = handle.join().map_err(|_| "spin thread panicked")??;

	// -- Check
	assert!(
		(700_000_000..=1_300_000_000).contains(&delta),
		"spin thread cpu delta should be about 1e9 ns, got {delta}"
	);

	Ok(())
}

#[test]
fn test_cpu_process_stats_busy_spin_delta() -> Result<()> {
	// -- Setup & Fixtures
	let pid = std::process::id() as i32;
	let before = lib_common::os::process_stats(pid)?;

	// -- Exec
	spin_for(std::time::Duration::from_secs(1));
	let after = lib_common::os::process_stats(pid)?;

	// -- Check
	let before_ns = before.user_ns + before.system_ns;
	let after_ns = after.user_ns + after.system_ns;
	let delta = after_ns.saturating_sub(before_ns);
	assert!(
		delta >= 700_000_000,
		"process cpu delta should be at least 0.7e9 ns, got {delta}"
	);

	Ok(())
}

// region:    --- Test Support

// The spawned thread sets its pthread name from inside its start routine, so the
// parent can list the thread before its name is visible. Poll until it shows up.
fn wait_for_thread_named(pid: i32, name: &str) -> Result<bool> {
	let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
	loop {
		let threads = lib_common::os::thread_cpu_all(pid)?;
		if threads.iter().any(|thread| thread.name == name) {
			return Ok(true);
		}
		if std::time::Instant::now() >= deadline {
			return Ok(false);
		}
		std::thread::sleep(std::time::Duration::from_millis(2));
	}
}

fn spin_for(duration: std::time::Duration) {
	let started = std::time::Instant::now();
	let mut acc: u64 = 0;
	while started.elapsed() < duration {
		acc = std::hint::black_box(acc.wrapping_add(1));
	}
}

fn thread_cpu_total_ns(pid: i32, name: &str) -> core::result::Result<u64, String> {
	let threads = lib_common::os::thread_cpu_all(pid).map_err(|e| e.to_string())?;
	let thread = threads
		.iter()
		.find(|thread| thread.name == name)
		.ok_or_else(|| format!("thread {name} not found"))?;
	Ok(thread.user_ns + thread.system_ns)
}

// endregion: --- Test Support
