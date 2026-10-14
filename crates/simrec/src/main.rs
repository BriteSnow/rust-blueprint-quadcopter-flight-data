// region:    --- Modules

mod cmd;
mod app;
mod error;
mod metrics;
#[path = "mock-in.rs"]
mod mock_in;
mod rec;
#[path = "rec-raw.rs"]
mod rec_raw;
#[path = "rec-sqlite.rs"]
mod rec_sqlite;
mod recorder;
mod sampler;
mod sim;
mod support;

pub use error::{Error, Result};

// endregion: --- Modules

fn main() {
    if let Err(error) = app::run() {
        eprintln!("simrec: {error}");
        std::process::exit(1);
    }
}
