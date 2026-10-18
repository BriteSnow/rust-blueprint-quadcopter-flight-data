//! Test support utilities shared by the mocker unit tests.

#![allow(unused)] // For test support.

use crate::error::{Error, Result};

/// Reads the fixture bytes, or returns `None` when the fixture has not been captured yet.
///
/// Set `MOCKER_REQUIRE_FIXTURES=1` to turn a missing fixture into a failure, so the
/// regression guard can never silently pass in a strict run.
pub fn opt_fixture_bytes(path: &str) -> Result<Option<Vec<u8>>> {
	match std::fs::read(path) {
		Ok(bytes) => Ok(Some(bytes)),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
			let message = format!(
				"{path} is missing; capture it from the pre-change build (see plan.md), or set MOCKER_REQUIRE_FIXTURES=1 to fail instead"
			);
			if std::env::var_os("MOCKER_REQUIRE_FIXTURES").is_some() {
				return Err(Error::Invalid(message));
			}
			eprintln!("mocker: skipping fixture check: {message}");
			Ok(None)
		}
		Err(error) => Err(Error::Invalid(format!("cannot read {path}: {error}"))),
	}
}
