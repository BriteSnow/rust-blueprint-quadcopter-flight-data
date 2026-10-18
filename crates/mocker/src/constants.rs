pub(crate) const RATE_HZ: u32 = 200;
pub(crate) const TICK_NS: u64 = 5_000_000;
pub(crate) const CHANNEL_COUNT: usize = 10;
pub(crate) const BYTES_PER_TICK: usize = CHANNEL_COUNT * 4;
pub(crate) const CHANNELS: [&str; CHANNEL_COUNT] = [
	"rotor_fl_rpm",
	"rotor_fr_rpm",
	"rotor_rl_rpm",
	"rotor_rr_rpm",
	"accel_x_mps2",
	"accel_y_mps2",
	"accel_z_mps2",
	"gyro_x_rads",
	"gyro_y_rads",
	"gyro_z_rads",
];
