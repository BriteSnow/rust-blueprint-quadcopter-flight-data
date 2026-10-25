// region:    --- Modules

mod app;
mod constants;
mod error;
mod story;
mod synth;
mod writer;

#[cfg(test)]
mod _test_support;

pub(crate) use constants::{BYTES_PER_TICK, CHANNELS, CHANNEL_COUNT, RATE_HZ, TICK_NS};

// endregion: --- Modules

fn main() {
	if let Err(error) = app::run() {
		eprintln!("mocker: {error}");
		std::process::exit(1);
	}
}
