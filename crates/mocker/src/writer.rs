use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
};

use serde::Serialize;
use simple_fs::SPath;

use crate::{
    error::{Error, Result},
    story::Story,
    synth::Synth,
    BYTES_PER_TICK, CHANNELS, RATE_HZ,
};

const CHUNK_TICKS: usize = RATE_HZ as usize;
const HOUR_TICKS: u64 = 3_600 * RATE_HZ as u64;

#[derive(Serialize)]
struct Manifest {
    version: u32,
    seed: u64,
    rate_hz: u32,
    channels: [&'static str; 5],
    tick_count: u64,
    start_ns: i64,
    bytes_per_tick: usize,
}

fn create_new(path: &SPath) -> Result<File> {
    Ok(OpenOptions::new().write(true).create_new(true).open(path)?)
}

pub fn generate(out: &SPath, seed: u64, tick_count: u64) -> Result<()> {
    fs::create_dir_all(out)?;
    if fs::read_dir(out)?.next().transpose()?.is_some() {
        return Err(Error::Invalid(format!(
            "output directory {} is not empty; use a new or empty directory",
            out
        )));
    }

    let mut ticks = BufWriter::with_capacity(1_048_576, create_new(&out.join("ticks.bin"))?);
    let mut story_output =
        BufWriter::with_capacity(65_536, create_new(&out.join("story.jsonl"))?);
    let mut story = Story::new(seed, tick_count);
    let mut synth = Synth::new(seed);
    let mut chunk = [0_u8; CHUNK_TICKS * BYTES_PER_TICK];
    let mut written = 0_u64;

    while let Some(segment) = story.next_segment() {
        serde_json::to_writer(&mut story_output, &segment)?;
        story_output.write_all(b"\n")?;
        synth.begin_segment();
        let mut tick = segment.start_tick;
        while tick < segment.end_tick {
            let until_hour = HOUR_TICKS - written % HOUR_TICKS;
            let count = (segment.end_tick - tick)
                .min(CHUNK_TICKS as u64)
                .min(until_hour) as usize;
            for offset in 0..count {
                let values = synth.sample(tick + offset as u64, &segment);
                for (channel, value) in values.into_iter().enumerate() {
                    let start = offset * BYTES_PER_TICK + channel * 4;
                    chunk[start..start + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
            ticks.write_all(&chunk[..count * BYTES_PER_TICK])?;
            tick += count as u64;
            written += count as u64;
            if written.is_multiple_of(HOUR_TICKS) {
                eprintln!(
                    "Generated {} simulated hours ({written}/{tick_count} ticks).",
                    written / HOUR_TICKS
                );
            }
        }
    }

    ticks.flush()?;
    story_output.flush()?;
    ticks.get_ref().sync_all()?;
    story_output.get_ref().sync_all()?;
    drop(ticks);
    drop(story_output);

    let manifest = Manifest {
        version: 1,
        seed,
        rate_hz: RATE_HZ,
        channels: CHANNELS,
        tick_count,
        start_ns: 0,
        bytes_per_tick: BYTES_PER_TICK,
    };
    let temporary_path = out.join(".manifest.json.tmp");
    let mut temporary = create_new(&temporary_path)?;
    serde_json::to_writer_pretty(&mut temporary, &manifest)?;
    temporary.write_all(b"\n")?;
    temporary.sync_all()?;
    drop(temporary);

    // Publish the completed manifest without replacing an existing pathname.
    // This requires hard-link support; it is not a crash-durability guarantee.
    fs::hard_link(&temporary_path, out.join("manifest.json"))?;
    if let Err(error) = fs::remove_file(&temporary_path) {
        eprintln!(
            "mocker: dataset completed, but temporary manifest cleanup failed: {error}"
        );
    }
    Ok(())
}
