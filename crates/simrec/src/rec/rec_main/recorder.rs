use crate::Result;
use lib_common::Tick;

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
