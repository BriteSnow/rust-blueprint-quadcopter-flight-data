// region:    --- Modules

mod app;
mod cli;
mod dispatcher;
mod error;
mod metrics;
mod ostel;
mod rec;
mod run;
mod sim;
mod support;

#[cfg(test)]
mod _test_support;

pub use error::{Error, Result};

// endregion: --- Modules

fn main() {
	if let Err(error) = app::run() {
		eprintln!("simrec: {error}");
		std::process::exit(1);
	}
}
