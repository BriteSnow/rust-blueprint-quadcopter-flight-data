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
cargo run --release -p simrec -- --raw --duration d1 --speed max
cargo run --release -p simrec -- --sql --duration d1 --speed max

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
  -h, --help                 Print help
  -V, --version              Print version
```

- Exactly one of `--noop`, `--sql`, or `--raw` is required.

- `--with-index` is opt-in, valid only with `--sql`, and creates the `accel_z` index. Its output goes to `sqlite-with-index/`, so it can run beside `sqlite-no-index/` for the same duration.

- The input is read from `<out>/mock-data/<name>`. Durations up to one day use `d001`, falling back to `d010`; longer durations up to ten days use `d010`. Each run writes to `captured/<name>-<speed>/<backend>/`, using the requested duration name (for example `m001` for `--duration m1`), where `<speed>` is `real` for `--speed real` and `max` for `--speed max`. The replay covers only the requested span from the start of the dataset, so `--duration m1` replays the first 12,000 ticks of `d001`. Real and max runs of the same dataset and backend can run in parallel. Repeating the same run needs its folder removed or moved first, because output directories must be empty.

- `--noop` is a pipeline diagnostic and writes no recording.

- `--flush-ms` must be positive; `--ring-cap` accepts 1 to 65536 messages.

- Without `--flush-ms`, a batch is written every 100 ms or at 8192 ticks, whichever comes first.

- With `--flush-ms`, a batch is written only when that age is reached, with no tick cap.

## Output

- Recordings go to `<out>/captured/<name>-<speed>/<backend>/`, such as `sandbox/captured/d001-max/sqlite-no-index/capture.sqlite`, `sandbox/captured/d001-real/sqlite-with-index/capture.sqlite`, or `sandbox/captured/d001-max/raw/seg-000000.bin`.

- `capture-metrics.json` is published in each backend folder only after successful replay and tick conservation.

- Output directories must be new or empty. Nothing is removed automatically.

## Limitations

- Completion markers and metrics show success, not crash durability.

- Floating-point output is reproducible only on the same implementation, dependencies, and platform.

- Cancellation cannot interrupt a blocked file read or recorder call.

## Status

- Implemented: mocker, validated replay, no-op, raw, and SQLite backends.

- Dataset generation, end-to-end comparisons, and benchmark measurements are pending. No measured results are claimed.

See `dev/spec-overall.md` for the full specification.
