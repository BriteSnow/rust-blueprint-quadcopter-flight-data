use crate::channels::CHANNEL_COUNT;

#[derive(Debug, Clone, Copy)]
pub struct Tick {
	pub epoch_micro: i64,
	pub v: [f32; CHANNEL_COUNT],
}
