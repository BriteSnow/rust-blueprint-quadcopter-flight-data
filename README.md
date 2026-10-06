# rust-poc-sensor-capture

A Rust proof of concept comparing SQLite WAL capture with append-only binary capture of the same quadcopter sensor stream.

## Current status


`mocker` implements deterministic streaming dataset generation. `simrec` implements validated replay through a dedicated producer/consumer pipeline, with no-op diagnostic, hourly raw binary, and SQLite WAL backends.
Unit tests cover replay validation, decoding, queue backpressure, consumer draining and cancellation, metrics, raw segment rotation, and SQLite batch transactions. The latest user-reported `cargo build`, `cargo test`, and `cargo clippy` checks passed after the SQLite changes. They were not independently rerun here. Dataset generation, end-to-end CLI comparisons, and benchmark measurements remain outstanding.

## Planned workloads

- Five channels sampled together at 200 Hz: four rotor RPM values and vertical acceleration.

- Seeded, streaming generation of one-day and ten-day datasets without retaining the full dataset in memory.

- A dedicated producer and consumer connected through an SPSC ring buffer, with realtime and maximum-speed replay.

- SQLite WAL and hourly raw binary segments, with throughput, backlog, flush latency, and disk-size metrics.

## Workspace

- `crates/lib-common/`: shared named, bounded synchronous SPSC facade.

- `crates/mocker/`: implemented canonical mock data generator.

- `crates/simrec/`: simulator, recording backends, and metrics.

- `data/mock/`: generated input datasets, excluded from Git.

- `.out/`: recording output and metrics, excluded from Git.

The player and query benchmarks are deferred. Dataset generation, end-to-end CLI replay comparisons, and benchmark measurements remain for a later validation loop.

## Generate mock input

```sh
cargo run --release -p mocker -- --duration 1d --seed 42 --out data/mock/1d
cargo run --release -p mocker -- --duration 10d --seed 42 --out data/mock/10d
```

These commands are usage examples, not executed results.

- `--duration` accepts a positive integer followed by `d`, `h`, or `m`, defaulting to `1d`. Arithmetic and the signed nanosecond timestamp range are checked before output creation.

- `--seed` defaults to `42`. `--out` defaults to `data/mock`. The output directory must be new or empty; existing contents are never intentionally overwritten.

- `--rate-hz` defaults to `200` and rejects every other value.

Generation uses fixed-size chunks and buffered output, with progress reported every simulated hour. Story and synthesis use separate seeded ChaCha8 random streams. Reproducibility targets the same implementation, dependency versions, and platform; cross-platform bitwise equality is not guaranteed for floating-point transcendental functions.

## Mock format and completion

- `ticks.bin` contains headerless little-endian `[f32; 5]` records, exactly 20 bytes per tick. Channel order is `rotor_fl`, `rotor_fr`, `rotor_rl`, `rotor_rr`, `accel_z`. RPM and m/s² are the respective units.

- Manifest version `1` contains `version`, `seed`, `rate_hz`, `channels`, `tick_count`, `start_ns`, and `bytes_per_tick`. The timestamp origin is synthetic: `start_ns = 0`. Tick index `i` has timestamp `i * 5_000_000` nanoseconds.

- `story.jsonl` contains contiguous, half-open `[start_tick, end_tick)` segments with `maneuver` and `intensity`. Maneuvers last 3 to 60 seconds, with gusts limited to 8 seconds; the final segment can be shorter when clipped to the requested duration.

- Maneuver effects ease from the preceding effect over 0.5 to 1 second. Slow periodic baseline drift, small rotor noise, and acceleration ripple remain separate from the maneuver ramp. Gusts use a brief smooth pulse. This is synthetic signal modeling, not a physical flight dynamics model.

- Data files are flushed and synchronized before the manifest is serialized into `.manifest.json.tmp`. The completed manifest is published last using a non-overwriting hard link, so the output filesystem must support hard links.

- A generation failure before manifest publication leaves partial artifacts without `manifest.json`. Use a different output directory or manually remove the partial directory before retrying. A leftover temporary manifest after successful publication is harmless and produces a cleanup warning.

The manifest is the generation-completion marker, not a checksum or a crash-safety guarantee. Directory entries are not explicitly synchronized. Builds, tests, deterministic comparisons, filesystem failure checks, and actual dataset generation remain deferred.

## Replay and recording

```sh
cargo run --release -p simrec -- --noop --mock-dir data/mock/1d --speed max
cargo run --release -p simrec -- --noop --mock-dir data/mock/1d --speed realtime --out .out/realtime
cargo run --release -p simrec -- --raw --mock-dir data/mock/1d --speed max
cargo run --release -p simrec -- --sql --mock-dir data/mock/1d --speed max
```

These commands are usage examples, not executed results. The no-op backend is a pipeline diagnostic, not a storage benchmark or a simulator-only baseline.

- Exactly one of `--noop`, `--sql`, or `--raw` is required. Outputs are placed under `.out/noop`, `.out/sqlite`, or `.out/raw`, respectively.

- `--flush-ms` defaults to `10` and must be positive and representable by the monotonic clock. `--ring-cap` defaults to `4096` and accepts `1` through `65536` messages.

- Startup checks manifest version, channel order, 200 Hz rate, 20-byte record width, positive tick count, checked payload size, signed timestamp range, exact input size, and required regular input files. The story must exist, but its JSONL content is not parsed.

- Input records are streamed through a bounded reader and explicitly decoded as little-endian floats. Non-finite values, premature EOF, and trailing bytes fail replay. Inputs must remain unchanged during replay; validation does not provide a filesystem snapshot.

- Output is `.out/noop/` by default and must be new or empty. Nothing is automatically removed. A failed run may leave partial artifacts; use another output directory before retrying.

## SQLite recording

The SQLite backend writes `.out/sqlite/capture.db` using the bundled SQLite engine, WAL journal mode, and `synchronous=NORMAL`. Each consumer batch is inserted in its own transaction with a cached prepared statement.

The `samples` table uses `ts_ns` as its primary key and stores the five sensor channels as SQLite `REAL` values. Finalization closes the database connection before counting `capture.db` and any remaining `capture.db-wal` and `capture.db-shm` files. A successful transaction is committed, but `synchronous=NORMAL` is not a guarantee against loss of the latest commits after power failure.

## Raw recording

The `--raw` backend writes each tick as a 28-byte little-endian record: an `i64` nanosecond timestamp followed by five `f32` channel values. Files are named `seg-000000.bin`, `seg-000001.bin`, and so on beneath `.out/raw`.

- Each segment contains one hour of input data, or 720,000 ticks. Rotation follows the validated tick sequence and can split a consumer batch at the hour boundary.

- `BufWriter` contents are flushed when a segment is completed and when recording finishes. Consumer batch submission is not an operating-system flush, and the backend does not call `sync_all`; it does not claim crash safety or filesystem durability.

- `metrics.json` reports the summed sizes of finalized raw segment files. Metrics and temporary metadata files are excluded. No-op replay continues to report zero output bytes.

## Pipeline and memory policy

The producer and consumer run on named dedicated threads. The ring contains `Vec<Tick>` messages, not individual ticks. Realtime deadlines are anchored to a monotonic start and the original tick index; maximum-speed replay skips pacing.

The shared `lib-common::event_base` facade encapsulates Crossfire handles and errors. Endpoints are single-owner and non-clonable; nonblocking sends return the owned message when full. Queued messages remain readable after producer shutdown. The consumer still waits for the separate successful-completion flag or cancellation, since endpoint disconnection alone does not establish successful replay.

- On a full ring, the producer retains its pending ticks, increments `stall_count`, and appends subsequent input until the pending vector reaches `1024` ticks. At that bound it pauses input and retries, preserving every tick rather than allowing unbounded memory growth.

- This deliberately bounds the original pending-vector backlog design. Under sustained backpressure, realtime replay falls behind its deadlines and later catches up without resetting its time origin. `max_pending_len` measures local pending ticks, not total queue occupancy or wall-clock lag.

- The consumer flushes nonempty batches when the interval elapses or the batch reaches `8192` ticks. It checks the interval while draining messages, preventing a continuously busy ring from postponing flushing indefinitely.

- Batch timing is best effort and uses short polling sleeps when idle. Slow recorder calls can exceed the requested interval.

- Each queued message contains at most `1024` ticks. Memory is bounded by the configured message capacity, vector allocation overhead, producer pending storage, consumer message and batch storage, input buffering, and fixed histogram storage. A large ring capacity can still consume substantial memory.

- Worker failures and panics signal cancellation through a drop guard. Pacing and retry waits check cancellation. The consumer acquires successful producer completion and rechecks the ring before final draining. Both workers are joined before completion is reported.

- Cancellation cannot interrupt a blocked filesystem read or a recorder call that never returns. Future recorder implementations must return errors normally; bounded peer waits do not impose I/O timeouts.

## Diagnostic metrics

`metrics.json` is published only after both workers succeed, recorder finalization succeeds, and produced and recorded tick counts equal the manifest count. Publication uses a synchronized temporary file and a non-overwriting hard link, requiring hard-link support.

- Wall time and throughput cover ring allocation, worker startup, replay, final draining, finalization, and joins. Input validation, output setup, and metrics serialization are excluded.

- `stall_count` counts failed queue push attempts, including repeated retries. `max_pending_len` includes ordinary one-tick pending messages.

- Flush count and latency cover successful nonempty `write_batch` calls. Finalization latency is reported separately as `finish_ms`.

- Percentiles use nearest-rank lookup in 65 power-of-two nanosecond buckets. Values are conservative bucket upper bounds, capped by the observed maximum; they are approximations, not exact samples. Maximum latency is retained directly.

- No-op `output_bytes` is zero because it produces no recording files. Raw `output_bytes` is the summed size of finalized segment files. Metrics and temporary metadata files are excluded from recording size.

Metrics publication indicates successful replay, not crash durability. The user-reported workspace checks pass; the comparison procedure and remaining end-to-end validation are listed below. Dataset generation and measurement runs have not been performed.

## Comparison procedure

Build release binaries before timing, then generate each input dataset separately. These setup commands are not part of the capture measurements.

```sh
cargo build --release -p mocker -p simrec
target/release/mocker --duration 1d --seed 42 --out data/mock/1d
target/release/mocker --duration 10d --seed 42 --out data/mock/10d
target/release/mocker --duration 1m --seed 42 --out data/mock/realtime-1m
```

Run both storage backends against the same input for each workload:

| Workload | Input | Replay speed | Backends |
| --- | --- | --- | --- |
| One-day maximum-speed comparison | `data/mock/1d` | `max` | SQLite and raw |
| Ten-day maximum-speed comparison | `data/mock/10d` | `max` | SQLite and raw |
| Short realtime lag comparison | `data/mock/realtime-1m` | `realtime` | SQLite and raw |

Use GNU `time` to measure each already-built capture process. Each `--out` value below is unique because `simrec` refuses a nonempty backend output directory.

```sh
/usr/bin/time -v target/release/simrec --sql --mock-dir data/mock/1d --speed max --out .out/compare/1d-sql
/usr/bin/time -v target/release/simrec --raw --mock-dir data/mock/1d --speed max --out .out/compare/1d-raw
/usr/bin/time -v target/release/simrec --sql --mock-dir data/mock/10d --speed max --out .out/compare/10d-sql
/usr/bin/time -v target/release/simrec --raw --mock-dir data/mock/10d --speed max --out .out/compare/10d-raw
/usr/bin/time -v target/release/simrec --sql --mock-dir data/mock/realtime-1m --speed realtime --out .out/compare/realtime-sql
/usr/bin/time -v target/release/simrec --raw --mock-dir data/mock/realtime-1m --speed realtime --out .out/compare/realtime-raw
```

Record GNU `time` wall time, CPU time, and maximum resident set size alongside each output's `metrics.json`. The program's `wall_time_s`, throughput, stalls, flush latency, and `output_bytes` provide the capture metrics. Use a fresh output path for every repetition, run one capture at a time, and record the machine and run order. `/usr/bin/time -v` is GNU-specific; use the platform's equivalent resource measurement tool elsewhere. These commands are a comparison procedure, not reported measurements.

## Deferred validation checklist

- Run `cargo fmt --all -- --check`. The user-reported passing checks cover `cargo build`, `cargo test`, and `cargo clippy`; those results were not independently rerun for this documentation update.

- Compare `ticks.bin` and `story.jsonl` from two fresh same-seed generator runs on the same platform and dependency set. Confirm contiguous half-open story ranges and the manifest's tick count and byte width.

- Smoke-test no-op, SQLite, and raw replay on a short fixture. Confirm each run records exactly the manifest tick count, timestamps are ordered, and metrics are published only after successful completion.

- Exercise invalid manifests and payloads, including version, channel order, rate, record width, size, timestamp bounds, non-finite values, truncation, and trailing data.

- Verify raw little-endian records and exact hourly segment sizes, including a segment boundary crossed inside one batch. Verify SQLite WAL settings, stored timestamps and channel values, transaction rollback, and post-finalization output-size accounting.

- Exercise producer and consumer error and panic shutdown paths. Confirm the originating failure is reported, queued data is drained on successful completion, and both workers are joined.

- Check `metrics.json` against the input and output artifacts, including tick totals, throughput, flush counts and latency, finish latency, and backend-specific output bytes.

- Run the one-day, ten-day, and short realtime comparison matrix above. Preserve actual results for the later comparison report; no dataset-scale measurements are claimed here.
