# Project Spec: rust-poc-sensor-capture

## Purpose

- Compare SQLite WAL capture with append-only raw binary capture for the same quadcopter sensor stream.
- Measure on-disk size, write throughput, backlog and lag, flush latency, and CPU and RSS of the capture process.
- Datasets: one day (17,280,000 ticks) and ten days (172,800,000 ticks).
- Player and query benchmarks are deferred.

## Sensor Model

Five channels are sampled together at 200 Hz, one tick every 5 ms.

| # | Channel  | Unit | Notes                                       |
| - | -------- | ---- | ------------------------------------------- |
| 0 | rotor_fl | RPM  | front-left                                  |
| 1 | rotor_fr | RPM  | front-right                                 |
| 2 | rotor_rl | RPM  | rear-left                                   |
| 3 | rotor_rr | RPM  | rear-right                                  |
| 4 | accel_z  | m/s2 | vertical acceleration, about 9.81 at hover  |

- A tick is `[f32; 5]` in little-endian order, 20 bytes.
- Timestamps are implicit in the mock input: `ts_ns = i * 5_000_000` with origin 0.

## Workspace

```text
Cargo.toml              workspace, shared dependencies, lints, profiles
README.md               usage, formats, limitations, comparison procedure
.gitignore              data/, .out/
crates/
  lib-common/           shared named bounded SPSC facade (event_base)
  mocker/               deterministic mock dataset generator
  simrec/               validated replay, pipeline, recorders, metrics
data/mock/              mocker output (not versioned)
.out/                   simrec output (not versioned)
dev/                    project rules and specs
.workbench/             planning and chat workspace
```

- Workspace resolver is 3, edition stays 2021.
- Workspace lints forbid `unsafe_code`.
- Dependencies are inherited from `[workspace.dependencies]`.

## Crate: lib-common

- Provides `event_base`, a named, bounded, synchronous SPSC facade over Crossfire.
- Endpoints are single-owner and non-clonable.
- `try_send` returns the owned message when the queue is full, so no data is lost.
- `try_recv` drains queued messages before reporting sender disconnection.
- Backend handles and errors stay private.
- Errors: `InvalidCapacity`, `TxDisconnected`, `RxDisconnected`, with `is_disconnected()`.

## Crate: mocker

### CLI

| Arg          | Default     | Notes                                      |
| ------------ | ----------- | ------------------------------------------ |
| `--duration` | `1d`        | positive integer followed by `d`, `h`, `m` |
| `--seed`     | `42`        | reproducible for the same build            |
| `--out`      | `data/mock` | must be new or empty                       |
| `--rate-hz`  | `200`       | only `200` is accepted                     |

### Output

- `ticks.bin`: headerless little-endian `[f32; 5]` records, 20 bytes per tick.
- `story.jsonl`: contiguous half-open `[start_tick, end_tick)` maneuver segments with `maneuver` and `intensity`.
- `manifest.json`: version 1 with `seed`, `rate_hz`, `channels`, `tick_count`, `start_ns`, `bytes_per_tick`.
- The manifest is written last and is the completion marker.

### Synthesis

- Seeded weighted random walk over ten maneuvers, lasting 3 to 60 seconds (gusts up to 8 seconds).
- Smooth ramps of 0.5 to 1 second between effects.
- Slow baseline drift, rotor noise, and acceleration ripple.
- Story and synthesis use separate seeded ChaCha8 streams.
- Data is streamed in one-second chunks through buffered output. Memory use does not grow with duration.

### Completion

- Data files are flushed and synchronized before the manifest is published.
- The manifest is published with a non-overwriting hard link, so hard-link support is required.
- A failure before publication leaves partial output without `manifest.json`.

## Crate: simrec

### CLI

| Arg           | Default     | Notes                                       |
| ------------- | ----------- | ------------------------------------------- |
| `--noop`      | (one of)    | pipeline diagnostic, writes no recording    |
| `--sql`       | (one of)    | SQLite WAL backend                          |
| `--raw`       | (one of)    | hourly raw binary backend                   |
| `--mock-dir`  | `data/mock` | input dataset directory                     |
| `--out`       | `.out`      | backend writes to `<out>/<backend>/`        |
| `--flush-ms`  | `10`        | positive consumer flush interval            |
| `--speed`     | `realtime`  | `realtime` (deadline pacing) or `max`       |
| `--ring-cap`  | `4096`      | ring capacity in messages, 1 to 65536       |

- Exactly one backend flag is required.
- Output directories must be new or empty. Nothing is removed automatically.

### Startup Validation

- Manifest version 1, channel order, 200 Hz, 20-byte records, positive tick count.
- Checked payload size and signed nanosecond timestamp range.
- `ticks.bin` size must equal `tick_count * 20` exactly.
- `manifest.json`, `ticks.bin`, and `story.jsonl` must be regular files. The story is not parsed.

### Pipeline

- Producer thread reads `ticks.bin` through a bounded buffered reader and decodes little-endian floats.
- Non-finite values, premature EOF, and trailing bytes fail replay.
- Realtime pacing uses deadlines anchored to a monotonic start, so backpressure creates lateness rather than timestamp drift.
- Ticks travel in `Vec<Tick>` messages of at most 1024 ticks.
- A full ring keeps pending ticks in the producer. Stalls are counted. Input pauses when pending reaches 1024 ticks.
- Consumer thread appends ticks to a batch and flushes when the interval elapses or the batch reaches 8192 ticks.
- Final draining happens only after producer success is published and the ring is rechecked.
- Both workers are joined before completion is reported.
- Tick conservation is required: produced and recorded counts must equal the manifest count.

### Recorder Trait

- `write_batch(&mut self, ticks: &[Tick]) -> Result<()>`
- `finish(self: Box<Self>) -> Result<u64>`, returning output bytes.

### Backends

- No-op: discards ticks. Reports zero output bytes. It measures pipeline overhead only.
- SQLite: `.out/sqlite/capture.sqlite`, WAL journal mode, `synchronous=NORMAL`. Table `samples` with `ts_ns` as primary key and five `REAL` channels. Each nonempty batch is one transaction with a cached prepared statement. Finalization closes the connection and counts the database plus any remaining `-wal` and `-shm` files.
- Raw: `.out/raw/seg-000000.bin` and so on. Each record is 28 bytes: `i64` timestamp followed by five `f32` values. Each segment holds one hour (720,000 ticks), including a rotation inside a batch. Writes use `BufWriter`, flushed on rotation and finalization. `output_bytes` is the sum of finalized segment sizes.

### Failure Handling

- Each worker holds a cancellation guard that signals the peer on error or panic.
- Pacing and retry waits check cancellation.
- When both workers fail, the originating error is preserved rather than the peer's cancellation or disconnection.
- Cancellation cannot interrupt a blocked file read or a recorder call that never returns.

### Metrics

Published to `<out>/<backend>/metrics.json` only after successful replay, finalization, and tick conservation.

- Configuration: backend, speed, `flush_ms`, `ring_cap`, batch and pending limits.
- Throughput: `wall_time_s`, `ticks_total`, `ticks_per_s`.
- Pipeline: `stall_count`, `max_pending_len`.
- Flushes: `flush_count`, `flush_ms_p50`, `flush_ms_p99`, `flush_ms_max`, with power-of-two bucket estimates.
- Finalization: `finish_ms`.
- Size: `output_bytes`.
- Publication uses a synced temporary file and a non-overwriting hard link.

## Conventions

- One `Error` enum and `Result` alias per crate.
- Paths use `simple_fs::SPath`.
- `main.rs` only wires modules and delegates to `app.rs`.
- Tests live in each module under `// region: --- Tests`, or in sibling files for large suites.
- Test fixtures live under `tests-data/.tmp` and are retained without automatic cleanup.
- Module-level docs are kept in Markdown and mirrored under `docs/rustdoc` per project guidance.

## Status

- Done: workspace scaffold, streaming mocker, validated replay, no-op pipeline, raw recording, SQLite WAL recording, README documentation.
- Reported by the user as passing: `cargo build`, `cargo test`, and `cargo clippy`. Not independently rerun.
- Pending: `cargo fmt --all -- --check`, dataset generation, short end-to-end replays, failure-shutdown runtime checks, and the comparison matrix with CPU and RSS measurements.
- No benchmark results are claimed yet.

## Known Limits

- Metrics and manifests show completion, not crash durability.
- Raw and SQLite outputs are not crash-safe. `synchronous=NORMAL` can lose the latest commits after power failure.
- Input files must not change during replay.
- Floating-point output is reproducible only on the same implementation, dependency set, and platform.
- Large ring capacities can consume substantial memory.

## Deferred and Later

- Baseline run with the sim only, to subtract harness cost.
- Crash-safe raw format with length-prefixed or CRC records and an fsync policy.
- Parquet backend and compression comparison.
- Query benchmarks: time window and min/max downsampling per bucket.
- Split simulator and recorder into separate processes over a Unix domain socket.
- `player` crate: localhost web viewer with WebSocket control and binary frames, channel selection, and a maneuver timeline.
