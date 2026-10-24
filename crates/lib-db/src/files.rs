use crate::error::Result;
use std::path::Path;

/// Returns the database size plus its `-wal` and `-shm` side files when present.
pub fn db_file_bytes(path: impl AsRef<Path>) -> Result<u64> {
	let base = path.as_ref();
	let mut total = file_len(base)?;
	for suffix in ["-wal", "-shm"] {
		let mut sidecar = base.as_os_str().to_owned();
		sidecar.push(suffix);
		total += file_len(Path::new(&sidecar))?;
	}
	Ok(total)
}

fn file_len(path: &Path) -> Result<u64> {
	match std::fs::metadata(path) {
		Ok(meta) => Ok(meta.len()),
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
		Err(err) => Err(err.into()),
	}
}

// region:    --- Tests

#[cfg(test)]
mod tests {
	use super::db_file_bytes;
	use crate::_test_support::fixture_dir;

	#[test]
	fn test_files_db_file_bytes_counts_sidecars() -> crate::Result<()> {
		let dir = fixture_dir("test_files_db_file_bytes_counts_sidecars")?;
		let base = dir.join("capture-main.sqlite");
		std::fs::write(&base, "0123456789")?;
		std::fs::write(format!("{}-wal", base.display()), "abcde")?;
		std::fs::write(format!("{}-shm", base.display()), "xyz")?;

		assert_eq!(db_file_bytes(&base)?, 18);
		Ok(())
	}
}

// endregion: --- Tests
