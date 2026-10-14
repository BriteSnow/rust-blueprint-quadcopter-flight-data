# rust-poc-sensor-capture

Proof of concept comparing SQLite WAL capture with append-only raw binary capture of the same quadcopter sensor stream.

## Workspace

- `crates/lib-common/`: shared named, bounded SPSC facade.

- `crates/lib-os/`: per-thread and process CPU and RSS sampling (macOS only).

- `crates/mocker/`: deterministic mock dataset generator.

- `crates/simrec/`: replay pipeline, recording backends, and metrics.

- `sandbox/`: mock datasets (`mock-data/`) and captured recordings (`captured/`), not versioned.

## mocker

Generates `ticks.bin`, `story.jsonl`, and `manifest.json` in the output directory.

```sh
cargo run --release -p mocker -- --duration d1 --seed 42
cargo run --release -p mocker -- --duration d10 --seed 42
```

Help output from `cargo run --release -p mocker -- --help`:

```text
Generate a deterministic, streaming quadcopter sensor dataset

Usage: mocker [OPTIONS]

Options:
      --duration <DURATION>  [default: d1]
      --seed <SEED>          [default: 42]
      --start-us <START_US>
  -o, --out <OUT>            [default: sandbox]
      --rate-hz <RATE_HZ>    [default: 200]
  -h, --help                 Print help
```

- `--duration` uses the prefix form `d<N>`, `h<N>`, or `m<N>`, with N a positive integer. Whole days are named `dNNN`, other spans `mNNN`, so `h1` gives `m060`.

- The dataset is written to `<out>/mock-data/<name>`, which must be new or empty.

- `--rate-hz` accepts only `200`.

- `--start-us` sets the dataset start in epoch microseconds (positive). Without it, the current time is captured once at generation start.

- `manifest.json` is version 2 and records `start_epoch_us` and `tick_period_us` (5000). Tick timestamps follow `ts_us = start_epoch_us + i * tick_period_us`.

- Same-seed datasets differ in `start_epoch_us` unless `--start-us` is given. The tick data is still identical.

## simrec

Replays a mock dataset through the capture pipeline into exactly one backend.

```sh
cargo run --release -p simrec -- --noop --duration d1 --speed max

cargo run --release -p simrec -- --raw --duration d1 --speed max
cargo run --release -p simrec -- --sql --duration d1 --speed max
cargo run --release -p simrec -- --sql --duration d1 --speed max --with-index

# real mode (just 1 minute window, put m5, m10, or m30 to have bigger capture window)
cargo run --release -p simrec -- --raw --duration m1 --speed real
cargo run --release -p simrec -- --sql --duration m1 --speed real
cargo run --release -p simrec -- --sql --duration m1 --speed real --with-index
```

Help output from `cargo run --release -p simrec -- --help`:

```text
Replay sensor ticks through a dedicated capture pipeline

Usage: simrec [OPTIONS] <--noop|--sql|--raw>

Options:
      --noop
      --sql
      --raw
      --with-index
      --duration <DURATION>  [default: d1]
  -o, --out <OUT>            [default: sandbox]
      --flush-ms <FLUSH_MS>
      --speed <SPEED>        [default: real] [possible values: real, max]
      --ring-cap <RING_CAP>  [default: 4096]
      --sample-ms <SAMPLE_MS>  [default: 1000]
  -h, --help                 Print help
  -V, --version              Print version
```

- Exactly one of `--noop`, `--sql`, or `--raw` is required.

- `--with-index` is opt-in, valid only with `--sql`, and creates the `accel_z` index. Its output goes to `sqlite-with-index/`, so it can run beside `sqlite-no-index/` for the same duration.

- The input is read from `<out>/mock-data/<name>`. Durations up to one day use `d001`, falling back to `d010`; longer durations up to ten days use `d010`. Each run writes to `captured/<name>-<speed>/<backend>/`, using the requested duration name (for example `m001` for `--duration m1`), where `<speed>` is `real` for `--speed real` and `max` for `--speed max`. The replay covers only the requested span from the start of the dataset, so `--duration m1` replays the first 12,000 ticks of `d001`. Real and max runs of the same dataset and backend can run in parallel. Repeating the same run needs its folder removed or moved first, because output directories must be empty.

- `--noop` is a pipeline diagnostic and writes no recording.

- Only version 2 manifests are accepted. Regenerate older datasets with the current mocker.

- `--flush-ms` must be positive; `--ring-cap` accepts 1 to 65536 messages.

- `--sample-ms` sets the CPU sampling interval in milliseconds and must be positive.

- Worker threads are named `simrec-sim`, `simrec-rec`, and `simrec-sampler`, so their CPU usage can be told apart.

- A CPU sampler failure never fails the capture.

- Without `--flush-ms`, a batch is written every 100 ms or at 8192 ticks, whichever comes first.

- With `--flush-ms`, a batch is written only when that age is reached, with no tick cap.

## Output

- Recordings go to `<out>/captured/<name>-<speed>/<backend>/`, such as `sandbox/captured/d001-max/sqlite-no-index/capture.sqlite`, `sandbox/captured/d001-real/sqlite-with-index/capture.sqlite`, or `sandbox/captured/d001-max/raw/seg-000000.bin`.

- `capture-metrics.json` is published in each backend folder only after successful replay and tick conservation.

- Raw records are 28 bytes: little-endian `i64` `ts_us` (epoch microseconds) followed by five `f32` values.

- SQLite table `samples` uses `ts_epoch_micro` (epoch microseconds) as its primary key.

- Output directories must be new or empty. Nothing is removed automatically.

- `cpu-samples.jsonl` is written in each backend folder for every backend, including `--noop`. It is not covered by the metrics completion marker, so it may be partial after a failed run.

### cpu-samples.jsonl

One compact JSON object per line, with a `kind` discriminator. All lines for one sample share the same `seq` and `ts_epoch_us`.

```json
{"kind":"thread","seq":3,"ts_epoch_us":1792022403000000,"interval_us":1000412,"thread":"simrec-rec","user_ns":812000000,"system_ns":95000000,"cpu_pct":41.7,"sched_usage":40.3}
{"kind":"process","seq":3,"ts_epoch_us":1792022403000000,"interval_us":1000412,"user_ns":1020000000,"system_ns":140000000,"cpu_pct":48.2,"rss_bytes":38211584}
{"kind":"error","seq":4,"ts_epoch_us":1792022404000000,"message":"proc_pidinfo failed: ..."}
```

- Every thread of the process is listed, not only the named ones, so the main thread and any SQLite threads are visible.

- Unnamed threads use the label `tid-<n>`.

- A UI can read the file line by line, `JSON.parse` each line, and switch on `kind`. A trailing partial line must be skipped.

- Group by `thread` and plot `cpu_pct` against `ts_epoch_us`.

## Limitations

- Completion markers and metrics show success, not crash durability.

- Floating-point output is reproducible only on the same implementation, dependencies, and platform.

- Cancellation cannot interrupt a blocked file read or recorder call.

- CPU sampling is macOS only. Other platforms write one `error` line and sampling stops.

- Activity after the last sample (up to one interval) is not captured.

## Status

- Implemented: mocker, validated replay, no-op, raw, and SQLite backends.

- Dataset generation, end-to-end comparisons, and benchmark measurements are pending. No measured results are claimed.

See `dev/spec-overall.md` for the full specification.
