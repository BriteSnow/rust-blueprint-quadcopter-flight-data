use crate::Result;

pub const CHANNEL_COUNT: usize = 10;

#[derive(Debug, Clone, Copy)]
#[allow(dead_code, reason = "storage backends are not implemented yet")]
pub struct Tick {
	pub epoch_micro: i64,
	pub v: [f32; CHANNEL_COUNT],
}

pub trait Recorder: Send {
	fn write_batch(&mut self, ticks: &[Tick]) -> Result<()>;
	fn finish(self: Box<Self>) -> Result<u64>;
}

pub struct NoopRecorder;

impl Recorder for NoopRecorder {
	fn write_batch(&mut self, _ticks: &[Tick]) -> Result<()> {
		Ok(())
	}

	fn finish(self: Box<Self>) -> Result<u64> {
		Ok(0)
	}
}
