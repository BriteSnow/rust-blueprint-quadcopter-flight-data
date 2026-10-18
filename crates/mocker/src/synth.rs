use crate::story::{Maneuver, Segment};
use crate::{CHANNEL_COUNT, RATE_HZ};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::f64::consts::{PI, TAU};

// Effect layout: rotors fl/fr/rl/rr, accel x/y/z, roll, pitch, yaw rate.
const EFFECT_LEN: usize = 10;

const ACCEL_NOISE_STD: f64 = 0.05;
const GYRO_NOISE_STD: f64 = 0.005;
const GYRO_BIAS_STEP: f32 = 2e-6;
const GYRO_BIAS_LIMIT: f32 = 0.002;

pub struct Synth {
	rng: ChaCha8Rng,
	ramp_from: [f32; EFFECT_LEN],
	last_effect: [f32; EFFECT_LEN],
	prev_attitude: [f32; 2],
	gust: GustParams,
	gyro_bias: [f32; 3],
}

// Random burst parameters, drawn once when a gust segment begins.
#[derive(Default)]
struct GustParams {
	accel: [f32; 2],
	attitude: [f32; 2],
	gyro_burst: [f32; 2],
	yaw_rate: f32,
}

impl Synth {
	pub fn new(seed: u64) -> Self {
		Self {
			rng: ChaCha8Rng::seed_from_u64(seed ^ 0x5359_4e54_485f_5631),
			ramp_from: [0.0; EFFECT_LEN],
			last_effect: [0.0; EFFECT_LEN],
			prev_attitude: [0.0; 2],
			gust: GustParams::default(),
			gyro_bias: [0.0; 3],
		}
	}

	pub fn begin_segment(&mut self, segment: &Segment) {
		self.ramp_from = self.last_effect;
		let gust = if matches!(segment.maneuver, Maneuver::Gust) {
			let accel = [
				self.rng.gen_range(-3.0_f32..=3.0),
				self.rng.gen_range(-3.0_f32..=3.0),
			];
			let attitude = [
				self.rng.gen_range(-0.1_f32..=0.1),
				self.rng.gen_range(-0.1_f32..=0.1),
			];
			let yaw_rate = self.rng.gen_range(-0.5_f32..=0.5);
			let gyro_burst = [
				self.rng.gen_range(-1.5_f32..=1.5),
				self.rng.gen_range(-1.5_f32..=1.5),
			];
			GustParams {
				accel,
				attitude,
				gyro_burst,
				yaw_rate,
			}
		} else {
			GustParams::default()
		};
		self.gust = gust;
	}

	pub fn sample(&mut self, tick: u64, segment: &Segment) -> [f32; CHANNEL_COUNT] {
		let elapsed = tick - segment.start_tick;
		let mut target = match segment.maneuver {
			Maneuver::Hover => [0.0; EFFECT_LEN],
			Maneuver::Climb => [800.0, 800.0, 800.0, 800.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0],
			Maneuver::Descend => [-650.0, -650.0, -650.0, -650.0, 0.0, 0.0, -1.6, 0.0, 0.0, 0.0],
			Maneuver::Forward => [-450.0, -450.0, 450.0, 450.0, 2.0, 0.0, 0.0, 0.0, 0.35, 0.0],
			Maneuver::Backward => [450.0, 450.0, -450.0, -450.0, -2.0, 0.0, 0.0, 0.0, -0.35, 0.0],
			Maneuver::BankLeft => [-600.0, 600.0, -600.0, 600.0, 0.0, 2.0, -0.4, -0.35, 0.0, 0.0],
			Maneuver::BankRight => [600.0, -600.0, 600.0, -600.0, 0.0, -2.0, -0.4, 0.35, 0.0, 0.0],
			Maneuver::YawCw => [350.0, -350.0, -350.0, 350.0, 0.0, 0.0, 0.0, 0.0, 0.0, -0.8],
			Maneuver::YawCcw => [-350.0, 350.0, 350.0, -350.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.8],
			Maneuver::Gust => [
				-450.0,
				650.0,
				-300.0,
				500.0,
				self.gust.accel[0],
				self.gust.accel[1],
				3.0,
				self.gust.attitude[0],
				self.gust.attitude[1],
				self.gust.yaw_rate,
			],
		};
		let envelope = if matches!(segment.maneuver, Maneuver::Gust) {
			let phase = elapsed as f64 / (segment.end_tick - segment.start_tick) as f64;
			(PI * phase).sin().powi(2) as f32
		} else {
			1.0
		};
		let scale = segment.intensity * envelope;
		for value in &mut target {
			*value *= scale;
		}
		let fraction = (elapsed as f32 / segment.ramp_ticks as f32).min(1.0);
		let blend = fraction * fraction * (3.0 - 2.0 * fraction);
		let effect: [f32; EFFECT_LEN] =
			std::array::from_fn(|index| self.ramp_from[index] + (target[index] - self.ramp_from[index]) * blend);
		self.last_effect = effect;

		// Gyro x/y are the per-tick derivative of the ramped roll and pitch angles.
		let rate_hz = RATE_HZ as f32;
		let attitude_rate = [
			(effect[7] - self.prev_attitude[0]) * rate_hz,
			(effect[8] - self.prev_attitude[1]) * rate_hz,
		];
		self.prev_attitude = [effect[7], effect[8]];
		let gyro_burst = if matches!(segment.maneuver, Maneuver::Gust) {
			[self.gust.gyro_burst[0] * scale, self.gust.gyro_burst[1] * scale]
		} else {
			[0.0; 2]
		};

		// Keep time in f64 so sub-tick ripple phase is not lost on multi-day runs.
		let seconds = tick as f64 / f64::from(RATE_HZ);
		let baseline =
			5_200.0 + 180.0 * (TAU * seconds / (6.0 * 3_600.0)).sin() + 95.0 * (TAU * seconds / (2.3 * 3_600.0)).sin();
		let mut values = [0.0; CHANNEL_COUNT];
		for index in 0..4 {
			values[index] = baseline as f32 + effect[index] + self.rng.gen_range(-12.0_f32..=12.0);
		}
		let ripple = 0.055 * (TAU * 37.0 * seconds).sin() + 0.025 * (TAU * 61.0 * seconds).sin();
		// Channel layout: 0..4 rotors, 4..6 accel x/y, 6 accel z, 7..10 gyro x/y/z.
		values[6] = 9.81 + effect[6] + ripple as f32 + self.rng.gen_range(-0.035_f32..=0.035);
		values[4] = effect[4] + gaussian(&mut self.rng, ACCEL_NOISE_STD) as f32;
		values[5] = effect[5] + gaussian(&mut self.rng, ACCEL_NOISE_STD) as f32;
		for bias in &mut self.gyro_bias {
			let step = self.rng.gen_range(-GYRO_BIAS_STEP..=GYRO_BIAS_STEP);
			*bias = (*bias + step).clamp(-GYRO_BIAS_LIMIT, GYRO_BIAS_LIMIT);
		}
		let mut gyro = [0.0_f32; 3];
		for (axis, value) in gyro.iter_mut().enumerate() {
			*value = self.gyro_bias[axis] + gaussian(&mut self.rng, GYRO_NOISE_STD) as f32;
		}
		gyro[0] += attitude_rate[0] + gyro_burst[0];
		gyro[1] += attitude_rate[1] + gyro_burst[1];
		gyro[2] += effect[9];
		values[7..10].copy_from_slice(&gyro);
		values
	}
}

// region:    --- Support

/// Box-Muller normal draw over the shared synthesis stream, so no extra dependency is needed.
fn gaussian(rng: &mut ChaCha8Rng, std: f64) -> f64 {
	let u1 = 1.0 - rng.gen::<f64>();
	let u2 = rng.gen::<f64>();
	std * (-2.0 * u1.ln()).sqrt() * (TAU * u2).cos()
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
	type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

	use super::*;

	#[test]
	fn test_synth_sample_hover_means() -> Result<()> {
		// -- Setup & Fixtures
		let segments = [segment(0, 2_000, Maneuver::Hover, 1.0, 200)];

		// -- Exec
		let ticks = run_segments(42, &segments);
		let ticks = ticks.first().ok_or("hover segment should produce ticks")?;
		let count = ticks.len() as f32;

		// -- Check
		for axis in 0..3 {
			let mean = ticks.iter().map(|values| values[7 + axis]).sum::<f32>() / count;
			assert!(mean.abs() <= 0.01, "gyro axis {axis} mean must stay near zero, got {mean}");
		}
		let mean_accel_z = ticks.iter().map(|values| values[6]).sum::<f32>() / count;
		assert!(
			(mean_accel_z - 9.81).abs() <= 0.05,
			"accel z mean must be near 9.81, got {mean_accel_z}"
		);
		for index in [4, 5] {
			let mean = ticks.iter().map(|values| values[index]).sum::<f32>() / count;
			assert!(mean.abs() <= 0.02, "accel index {index} mean must stay near zero, got {mean}");
		}
		Ok(())
	}

	#[test]
	fn test_synth_sample_yaw_direction() -> Result<()> {
		// -- Setup & Fixtures
		let segments = [
			segment(0, 200, Maneuver::Hover, 1.0, 200),
			segment(200, 1_000, Maneuver::YawCw, 1.0, 200),
			segment(1_200, 1_000, Maneuver::YawCcw, 1.0, 200),
		];

		// -- Exec
		let ticks = run_segments(42, &segments);
		let cw = ticks.get(1).ok_or("yaw_cw segment should produce ticks")?;
		let ccw = ticks.get(2).ok_or("yaw_ccw segment should produce ticks")?;
		let cw_mean = mean(&cw[200..], 9);
		let ccw_mean = mean(&ccw[200..], 9);

		// -- Check
		assert!(cw_mean < 0.0, "yaw_cw gyro z mean must be negative, got {cw_mean}");
		assert!(ccw_mean > 0.0, "yaw_ccw gyro z mean must be positive, got {ccw_mean}");
		Ok(())
	}

	#[test]
	fn test_synth_sample_bank_right_returns_level() -> Result<()> {
		// -- Setup & Fixtures
		let segments = [
			segment(0, 200, Maneuver::Hover, 1.0, 200),
			segment(200, 600, Maneuver::BankRight, 1.0, 200),
			segment(800, 600, Maneuver::Hover, 1.0, 200),
		];

		// -- Exec
		let ticks = run_segments(42, &segments);
		let bank = ticks.get(1).ok_or("bank_right segment should produce ticks")?;
		let level = ticks.get(2).ok_or("hover segment should produce ticks")?;
		let entry_mean = mean(&bank[..200], 7);
		let recovered: f32 = bank
			.iter()
			.chain(level.iter())
			.map(|values| values[7] / RATE_HZ as f32)
			.sum();

		// -- Check
		assert!(
			entry_mean > 0.0,
			"bank_right entry gyro x mean must be positive, got {entry_mean}"
		);
		assert!(
			recovered.abs() <= 0.02,
			"bank_right attitude must return to level, got {recovered} rad"
		);
		Ok(())
	}

	#[test]
	fn test_synth_sample_forward_effects() -> Result<()> {
		// -- Setup & Fixtures
		let segments = [
			segment(0, 200, Maneuver::Hover, 1.0, 200),
			segment(200, 1_000, Maneuver::Forward, 1.0, 200),
		];

		// -- Exec
		let ticks = run_segments(42, &segments);
		let forward = ticks.get(1).ok_or("forward segment should produce ticks")?;
		let accel_mean = mean(forward, 4);
		let entry_pitch = mean(&forward[..200], 8);

		// -- Check
		assert!(
			accel_mean > 0.0,
			"forward accel x mean must be positive, got {accel_mean}"
		);
		assert!(
			entry_pitch > 0.0,
			"forward entry gyro y mean must be positive, got {entry_pitch}"
		);
		Ok(())
	}

	// -- Test Support

	fn segment(start_tick: u64, len: u64, maneuver: Maneuver, intensity: f32, ramp_ticks: u64) -> Segment {
		Segment {
			start_tick,
			end_tick: start_tick + len,
			maneuver,
			intensity,
			ramp_ticks,
		}
	}

	fn run_segments(seed: u64, segments: &[Segment]) -> Vec<Vec<[f32; CHANNEL_COUNT]>> {
		let mut synth = Synth::new(seed);
		let mut ticks_by_segment = Vec::new();
		for segment in segments {
			synth.begin_segment(segment);
			let mut ticks = Vec::new();
			for tick in segment.start_tick..segment.end_tick {
				ticks.push(synth.sample(tick, segment));
			}
			ticks_by_segment.push(ticks);
		}
		ticks_by_segment
	}

	fn mean(ticks: &[[f32; CHANNEL_COUNT]], channel: usize) -> f32 {
		let count = ticks.len() as f32;
		ticks.iter().map(|values| values[channel]).sum::<f32>() / count
	}
}

// endregion: --- Tests
