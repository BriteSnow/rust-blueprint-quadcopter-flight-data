pub const CHANNEL_COUNT: usize = 10;
pub const CHANNELS: [&str; CHANNEL_COUNT] = [
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

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

	use super::*;

	#[test]
	fn test_channels_channels_order_simple() -> Result<()> {
		// -- Setup & Fixtures
		let expected_first = "rotor_fl_rpm";
		let expected_last = "gyro_z_rads";

		// -- Exec
		let first = CHANNELS.first().ok_or("CHANNELS should not be empty")?;
		let last = CHANNELS.last().ok_or("CHANNELS should not be empty")?;

		// -- Check
		assert_eq!(CHANNEL_COUNT, 10);
		assert_eq!(CHANNELS.len(), CHANNEL_COUNT);
		assert_eq!(*first, expected_first);
		assert_eq!(*last, expected_last);
		Ok(())
	}
}

// endregion: --- Tests
