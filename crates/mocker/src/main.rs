// region:    --- Modules

mod app;
mod constants;
mod error;
mod story;
mod synth;
mod writer;

pub(crate) use constants::{BYTES_PER_TICK, CHANNELS, RATE_HZ, TICK_NS};

// endregion: --- Modules

fn main() {
	if let Err(error) = app::run() {
		eprintln!("mocker: {error}");
		std::process::exit(1);
	}
}
