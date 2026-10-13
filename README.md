# rust-poc-sensor-capture

Proof of concept comparing SQLite WAL capture with append-only raw binary capture of the same quadcopter sensor stream.

## Workspace

- `crates/lib-common/`: shared named, bounded SPSC facade.

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
  -o, --out <OUT>            [default: sandbox]
      --rate-hz <RATE_HZ>    [default: 200]
  -h, --help                 Print help
```

- `--duration` uses the prefix form `d<N>`, `h<N>`, or `m<N>`, with N a positive integer. Whole days are named `dNNN`, other spans `mNNN`, so `h1` gives `m060`.

- The dataset is written to `<out>/mock-data/<name>`, which must be new or empty.

- `--rate-hz` accepts only `200`.

## simrec

Replays a mock dataset through the capture pipeline into exactly one backend.

```sh
cargo run --release -p simrec -- --noop --duration d1 --speed max
cargo run --release -p simrec -- --sql --duration d1 --speed max
cargo run --release -p simrec -- --sql --duration d1 --speed max --with-index --flush-ms 1000
cargo run --release -p simrec -- --raw --duration d1 --speed max
cargo run --release -p simrec -- --raw --duration d1 --speed realtime
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
      --flush-ms <FLUSH_MS>  [default: 10]
      --speed <SPEED>        [default: realtime] [possible values: realtime, max]
      --ring-cap <RING_CAP>  [default: 4096]
  -h, --help                 Print help
  -V, --version              Print version
```

- Exactly one of `--noop`, `--sql`, or `--raw` is required.

- `--with-index` is opt-in, valid only with `--sql`, and creates the `accel_z` index. Its output goes to `sqlite-with-index/`, so it can run beside `sqlite-no-index/` for the same duration.

- The input is read from `<out>/mock-data/<name>`, selected by `--duration`. Repeating the same backend for one duration needs its `captured/<name>/<backend>/` folder removed or moved first, because output directories must be empty.

- `--noop` is a pipeline diagnostic and writes no recording.

- `--flush-ms` must be positive; `--ring-cap` accepts 1 to 65536 messages.

## Output

- Recordings go to `<out>/captured/<name>/<backend>/`, such as `sandbox/captured/d001/sqlite-no-index/capture.sqlite`, `sandbox/captured/d001/sqlite-with-index/capture.sqlite`, or `sandbox/captured/d001/raw/seg-000000.bin`.

- `metrics.json` is published only after successful replay and tick conservation. SQLite uses `capture-metrics.json`.

- Output directories must be new or empty. Nothing is removed automatically.

## Limitations

- Completion markers and metrics show success, not crash durability.

- Floating-point output is reproducible only on the same implementation, dependencies, and platform.

- Cancellation cannot interrupt a blocked file read or recorder call.

## Status

- Implemented: mocker, validated replay, no-op, raw, and SQLite backends.

- Dataset generation, end-to-end comparisons, and benchmark measurements are pending. No measured results are claimed.

See `dev/spec-overall.md` for the full specification.
