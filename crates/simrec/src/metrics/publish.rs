use crate::Result;
use serde::Serialize;
use simple_fs::SPath;
use std::fs::{self, OpenOptions};
use std::io::Write;

// region:    --- Support

pub(super) fn publish_json<T: Serialize>(directory: &SPath, file_name: &str, value: &T) -> Result<()> {
	let temporary_path = directory.join(format!(".{file_name}.tmp"));
	let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary_path)?;
	serde_json::to_writer_pretty(&mut file, value)?;
	file.write_all(b"\n")?;
	file.sync_all()?;
	drop(file);
	fs::hard_link(&temporary_path, directory.join(file_name))?;
	if let Err(error) = fs::remove_file(&temporary_path) {
		eprintln!("simrec: {file_name} published, but temporary cleanup failed: {error}");
	}
	Ok(())
}

// endregion: --- Support
