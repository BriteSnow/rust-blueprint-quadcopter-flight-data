pub(crate) use lib_common::channels::{CHANNELS, CHANNEL_COUNT};

pub(crate) const RATE_HZ: u32 = 200;
pub(crate) const TICK_NS: u64 = 5_000_000;
pub(crate) const BYTES_PER_TICK: usize = CHANNEL_COUNT * 4;
