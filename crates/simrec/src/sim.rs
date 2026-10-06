use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

use lib_common::event_base::SyncSpscTx;

use crate::{
    mock_in::{MockInput, TICK_NS},
    recorder::Tick,
    support::{check_cancelled, idle_wait, wait_until},
    Result,
};

#[derive(Debug, Clone, Copy)]
pub enum ReplaySpeed {
    Realtime,
    Max,
}

#[derive(Debug, Default)]
pub struct ProducerStats {
    pub ticks_total: u64,
    pub stall_count: u64,
    pub max_pending_len: usize,
}

pub const MAX_PENDING_TICKS: usize = 1024;

pub fn produce(
    mut input: MockInput,
    mut producer: SyncSpscTx<Vec<Tick>>,
    speed: ReplaySpeed,
    cancelled: &AtomicBool,
) -> Result<ProducerStats> {
    let started = Instant::now();
    let mut stats = ProducerStats::default();
    let mut pending = Vec::new();

    while let Some(tick) = input.next_tick()? {
        check_cancelled(cancelled)?;
        if matches!(speed, ReplaySpeed::Realtime) {
            let offset = stats
                .ticks_total
                .checked_mul(TICK_NS)
                .ok_or("realtime deadline overflow")?;
            let deadline = started
                .checked_add(Duration::from_nanos(offset))
                .ok_or("realtime deadline exceeds the clock range")?;
            wait_until(deadline, cancelled)?;
        }
        pending.push(tick);
        stats.ticks_total += 1;
        stats.max_pending_len = stats.max_pending_len.max(pending.len());
        pending = try_send(&mut producer, pending, &mut stats)?;
        if pending.len() == MAX_PENDING_TICKS {
            pending = drain_pending(&mut producer, pending, &mut stats, cancelled)?;
        }
    }
    drain_pending(&mut producer, pending, &mut stats, cancelled)?;
    Ok(stats)
}

// region:    --- Support

fn try_send(
    producer: &mut SyncSpscTx<Vec<Tick>>,
    pending: Vec<Tick>,
    stats: &mut ProducerStats,
) -> Result<Vec<Tick>> {
    match producer.try_send(pending)? {
        None => Ok(Vec::new()),
        Some(pending) => {
            stats.stall_count = stats.stall_count.saturating_add(1);
            Ok(pending)
        }
    }
}

fn drain_pending(
    producer: &mut SyncSpscTx<Vec<Tick>>,
    mut pending: Vec<Tick>,
    stats: &mut ProducerStats,
    cancelled: &AtomicBool,
) -> Result<Vec<Tick>> {
    while !pending.is_empty() {
        check_cancelled(cancelled)?;
        pending = try_send(producer, pending, stats)?;
        if !pending.is_empty() {
            idle_wait(cancelled)?;
        }
    }
    Ok(pending)
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    use super::*;
    use lib_common::event_base::new_spsc_bounded;

    #[test]
    fn test_sim_try_send_full_retains_ticks() -> Result<()> {
        // -- Setup & Fixtures
        let (mut producer, mut consumer) = new_spsc_bounded("sim-test", 1)?;
        let tick = Tick {
            ts_ns: 0,
            v: [1.0; 5],
        };
        let mut stats = ProducerStats::default();

        // -- Exec
        let first = try_send(&mut producer, vec![tick], &mut stats)?;
        let pending = try_send(&mut producer, vec![tick, tick], &mut stats)?;
        let queued = consumer.try_recv()?.ok_or("expected queued ticks")?;

        // -- Check
        assert!(first.is_empty());
        assert_eq!(pending.len(), 2);
        assert_eq!(queued.len(), 1);
        assert_eq!(stats.stall_count, 1);
        Ok(())
    }
}

// endregion: --- Tests
