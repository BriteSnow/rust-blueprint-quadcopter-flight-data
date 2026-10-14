use crate::RATE_HZ;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::Serialize;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Maneuver {
	Hover,
	Climb,
	Descend,
	Forward,
	Backward,
	BankLeft,
	BankRight,
	YawCw,
	YawCcw,
	Gust,
}

const MANEUVERS: [Maneuver; 10] = [
	Maneuver::Hover,
	Maneuver::Climb,
	Maneuver::Descend,
	Maneuver::Forward,
	Maneuver::Backward,
	Maneuver::BankLeft,
	Maneuver::BankRight,
	Maneuver::YawCw,
	Maneuver::YawCcw,
	Maneuver::Gust,
];

#[derive(Serialize)]
pub struct Segment {
	pub start_tick: u64,
	pub end_tick: u64,
	pub maneuver: Maneuver,
	pub intensity: f32,
	#[serde(skip)]
	pub ramp_ticks: u64,
}

pub struct Story {
	rng: ChaCha8Rng,
	previous: Option<Maneuver>,
	next_tick: u64,
	tick_count: u64,
}

impl Story {
	pub fn new(seed: u64, tick_count: u64) -> Self {
		Self {
			rng: ChaCha8Rng::seed_from_u64(seed),
			previous: None,
			next_tick: 0,
			tick_count,
		}
	}

	fn choose_maneuver(&mut self) -> Maneuver {
		let weights: [u32; 10] = match self.previous {
			None => return Maneuver::Hover,
			Some(Maneuver::Hover) => [2, 5, 3, 6, 2, 2, 2, 2, 2, 1],
			Some(Maneuver::Climb) => [9, 1, 1, 8, 1, 2, 2, 1, 1, 1],
			Some(Maneuver::Descend) => [9, 2, 1, 6, 1, 2, 2, 1, 1, 1],
			Some(Maneuver::Forward | Maneuver::Backward) => [6, 2, 2, 2, 2, 5, 5, 2, 2, 1],
			Some(Maneuver::BankLeft | Maneuver::BankRight) => [6, 1, 1, 8, 2, 1, 1, 3, 3, 1],
			Some(Maneuver::YawCw | Maneuver::YawCcw) => [8, 2, 2, 6, 2, 2, 2, 1, 1, 1],
			Some(Maneuver::Gust) => [12, 2, 2, 6, 1, 1, 1, 1, 1, 0],
		};
		let mut choice = self.rng.gen_range(0..weights.iter().sum::<u32>());
		for (maneuver, weight) in MANEUVERS.into_iter().zip(weights) {
			if choice < weight {
				return maneuver;
			}
			choice -= weight;
		}
		unreachable!("weighted choice is bounded by the sum of weights")
	}

	pub fn next_segment(&mut self) -> Option<Segment> {
		if self.next_tick == self.tick_count {
			return None;
		}
		let maneuver = self.choose_maneuver();
		let max_seconds = if matches!(maneuver, Maneuver::Gust) { 8 } else { 60 };
		let requested = self.rng.gen_range(3 * u64::from(RATE_HZ)..=max_seconds * u64::from(RATE_HZ));
		let length = requested.min(self.tick_count - self.next_tick);
		let segment = Segment {
			start_tick: self.next_tick,
			end_tick: self.next_tick + length,
			maneuver,
			intensity: self.rng.gen_range(0.2_f32..=1.0),
			ramp_ticks: self.rng.gen_range(u64::from(RATE_HZ) / 2..=u64::from(RATE_HZ)),
		};
		self.next_tick = segment.end_tick;
		self.previous = Some(maneuver);
		Some(segment)
	}
}
