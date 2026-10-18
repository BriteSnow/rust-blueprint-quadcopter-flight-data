use crate::os::error::Result;

#[derive(Debug, Clone)]
pub struct ThreadCpu {
	pub thread_handle: u64, // kernel thread handle from ListThreads, stable while the thread lives
	pub name: String,       // pthread name, empty if unnamed
	pub user_ns: u64,
	pub system_ns: u64,
	pub sched_usage: f32, // kernel decayed usage, percent of one core
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessStats {
	pub user_ns: u64,
	pub system_ns: u64,
	pub rss_bytes: u64,
}

#[cfg(target_os = "macos")]
pub fn thread_cpu_all(pid: i32) -> Result<Vec<ThreadCpu>> {
	macos::thread_cpu_all(pid)
}

#[cfg(not(target_os = "macos"))]
pub fn thread_cpu_all(_pid: i32) -> Result<Vec<ThreadCpu>> {
	Err(crate::os::error::Error::Unsupported)
}

#[cfg(target_os = "macos")]
pub fn process_stats(pid: i32) -> Result<ProcessStats> {
	macos::process_stats(pid)
}

#[cfg(not(target_os = "macos"))]
pub fn process_stats(_pid: i32) -> Result<ProcessStats> {
	Err(crate::os::error::Error::Unsupported)
}

// region:    --- Support

#[cfg(target_os = "macos")]
mod macos {
	use super::{ProcessStats, ThreadCpu};
	use crate::os::error::{Error, Result};
	// TaskAllInfo and ThreadInfo are not re-exported by proc_pid in libproc 0.14.
	use libproc::libproc::task_info::TaskAllInfo;
	use libproc::libproc::thread_info::ThreadInfo;
	use libproc::proc_pid::{listpidinfo, pidinfo, ListThreads};
	use std::ffi::c_char;

	pub fn thread_cpu_all(pid: i32) -> Result<Vec<ThreadCpu>> {
		let task = task_all_info(pid)?;
		let max_len = usize::try_from(task.ptinfo.pti_threadnum).unwrap_or(0) + 16;
		let handles = listpidinfo::<ListThreads>(pid, max_len).map_err(|message| Error::ProcInfo {
			call: "proc_pidinfo(PROC_PIDLISTTHREADS)",
			message,
		})?;

		let mut threads = Vec::with_capacity(handles.len());
		for handle in handles {
			let info = match pidinfo::<ThreadInfo>(pid, handle) {
				Ok(info) => info,
				// The thread exited between listing and reading, so it is skipped.
				Err(_) => continue,
			};
			threads.push(ThreadCpu {
				thread_handle: handle,
				name: thread_name(&info.pth_name),
				user_ns: info.pth_user_time,
				system_ns: info.pth_system_time,
				sched_usage: info.pth_cpu_usage as f32 / 10.0,
			});
		}

		Ok(threads)
	}

	pub fn process_stats(pid: i32) -> Result<ProcessStats> {
		let task = task_all_info(pid)?;
		// pti_total_* only accumulates exited threads, so live thread times are added.
		let live = thread_cpu_all(pid)?;
		let live_user: u64 = live.iter().map(|thread| thread.user_ns).sum();
		let live_system: u64 = live.iter().map(|thread| thread.system_ns).sum();
		Ok(ProcessStats {
			user_ns: task.ptinfo.pti_total_user + live_user,
			system_ns: task.ptinfo.pti_total_system + live_system,
			rss_bytes: task.ptinfo.pti_resident_size,
		})
	}

	fn task_all_info(pid: i32) -> Result<TaskAllInfo> {
		pidinfo::<TaskAllInfo>(pid, 0).map_err(|message| Error::ProcInfo {
			call: "proc_pidinfo(PROC_PIDTASKALLINFO)",
			message,
		})
	}

	fn thread_name(raw: &[c_char]) -> String {
		let bytes: Vec<u8> = raw.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
		String::from_utf8_lossy(&bytes).into_owned()
	}
}

// endregion: --- Support
