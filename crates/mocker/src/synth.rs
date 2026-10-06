use std::f64::consts::{PI, TAU};

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::{
    story::{Maneuver, Segment},
    RATE_HZ,
};

pub struct Synth {
    rng: ChaCha8Rng,
    ramp_from: [f32; 5],
    last_effect: [f32; 5],
}

impl Synth {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed ^ 0x5359_4e54_485f_5631),
            ramp_from: [0.0; 5],
            last_effect: [0.0; 5],
        }
    }

    pub fn begin_segment(&mut self) {
        self.ramp_from = self.last_effect;
    }

    pub fn sample(&mut self, tick: u64, segment: &Segment) -> [f32; 5] {
        let elapsed = tick - segment.start_tick;
        let mut target = match segment.maneuver {
            Maneuver::Hover => [0.0; 5],
            Maneuver::Climb => [800.0, 800.0, 800.0, 800.0, 2.0],
            Maneuver::Descend => [-650.0, -650.0, -650.0, -650.0, -1.6],
            Maneuver::Forward => [-450.0, -450.0, 450.0, 450.0, 0.0],
            Maneuver::Backward => [450.0, 450.0, -450.0, -450.0, 0.0],
            Maneuver::BankLeft => [-600.0, 600.0, -600.0, 600.0, -0.4],
            Maneuver::BankRight => [600.0, -600.0, 600.0, -600.0, -0.4],
            Maneuver::YawCw => [350.0, -350.0, -350.0, 350.0, 0.0],
            Maneuver::YawCcw => [-350.0, 350.0, 350.0, -350.0, 0.0],
            Maneuver::Gust => [-450.0, 650.0, -300.0, 500.0, 3.0],
        };
        let envelope = if matches!(segment.maneuver, Maneuver::Gust) {
            let phase = elapsed as f64 / (segment.end_tick - segment.start_tick) as f64;
            (PI * phase).sin().powi(2) as f32
        } else {
            1.0
        };
        for value in &mut target {
            *value *= segment.intensity * envelope;
        }
        let fraction = (elapsed as f32 / segment.ramp_ticks as f32).min(1.0);
        let blend = fraction * fraction * (3.0 - 2.0 * fraction);
        let effect: [f32; 5] = std::array::from_fn(|index| {
            self.ramp_from[index] + (target[index] - self.ramp_from[index]) * blend
        });
        self.last_effect = effect;

        // Keep time in f64 so sub-tick ripple phase is not lost on multi-day runs.
        let seconds = tick as f64 / f64::from(RATE_HZ);
        let baseline = 5_200.0
            + 180.0 * (TAU * seconds / (6.0 * 3_600.0)).sin()
            + 95.0 * (TAU * seconds / (2.3 * 3_600.0)).sin();
        let mut values = [0.0; 5];
        for index in 0..4 {
            values[index] = baseline as f32
                + effect[index]
                + self.rng.gen_range(-12.0_f32..=12.0);
        }
        let ripple = 0.055 * (TAU * 37.0 * seconds).sin()
            + 0.025 * (TAU * 61.0 * seconds).sin();
        values[4] = 9.81
            + effect[4]
            + ripple as f32
            + self.rng.gen_range(-0.035_f32..=0.035);
        values
    }
}
