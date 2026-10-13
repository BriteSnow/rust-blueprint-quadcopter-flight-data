# rust-poc-sensor-capture

Proof of concept comparing SQLite WAL capture with append-only raw binary capture of the same quadcopter sensor stream.

## Workspace

- `crates/lib-common/`: shared named, bounded SPSC facade.

- `crates/mocker/`: deterministic mock dataset generator.

- `crates/simrec/`: replay pipeline, recording backends, and metrics.

- `data/mock/`: generated input datasets, not versioned.

- `.out/`: recording output and metrics, not versioned.

## mocker

Generates `ticks.bin`, `story.jsonl`, and `manifest.json` in the output directory.

```sh
cargo run --release -p mocker -- --duration 1d --seed 42 --out data/mock/1d
cargo run --release -p mocker -- --duration 10d --seed 42 --out data/mock/10d
```

Help output from `cargo run --release -p mocker -- --help`:

```text
Generate a deterministic, streaming quadcopter sensor dataset

Usage: mocker [OPTIONS]

Options:
      --duration <DURATION>  [default: 1d]
      --seed <SEED>          [default: 42]
      --out <OUT>            [default: data/mock]
      --rate-hz <RATE_HZ>    [default: 200]
  -h, --help                 Print help
```

- `--duration` accepts a positive integer followed by `d`, `h`, or `m`.

- The output directory must be new or empty.

- `--rate-hz` accepts only `200`.

## simrec

Replays a mock dataset through the capture pipeline into exactly one backend.

```sh
cargo run --release -p simrec -- --noop --mock-dir data/mock/1d --speed max
cargo run --release -p simrec -- --sql --mock-dir data/mock/1d --speed max --out .out/sql-no-index
cargo run --release -p simrec -- --sql --mock-dir data/mock/1d --speed max --out .out/sql-with-index --with-index --flush-ms 100
cargo run --release -p simrec -- --raw --mock-dir data/mock/1d --speed max
cargo run --release -p simrec -- --raw --mock-dir data/mock/1d --speed realtime --out .out/realtime
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
      --mock-dir <MOCK_DIR>  [default: data/mock]
      --out <OUT>            [default: .out]
      --flush-ms <FLUSH_MS>  [default: 10]
      --speed <SPEED>        [default: realtime] [possible values: realtime, max]
      --ring-cap <RING_CAP>  [default: 4096]
  -h, --help                 Print help
  -V, --version              Print version
```

- Exactly one of `--noop`, `--sql`, or `--raw` is required.

- `--with-index` is opt-in, valid only with `--sql`, and creates the `accel_z` index.

- Use a separate `--out` directory for each SQLite variant, because output directories must be empty.

- `--noop` is a pipeline diagnostic and writes no recording.

- `--flush-ms` must be positive; `--ring-cap` accepts 1 to 65536 messages.

## Output

- Recordings go to `<out>/<backend>/`, such as `.out/sqlite/capture.sqlite` or `.out/raw/seg-000000.bin`.

- `metrics.json` is published only after successful replay and tick conservation.

- Output directories must be new or empty. Nothing is removed automatically.

## Limitations

- Completion markers and metrics show success, not crash durability.

- Floating-point output is reproducible only on the same implementation, dependencies, and platform.

- Cancellation cannot interrupt a blocked file read or recorder call.

## Status

- Implemented: mocker, validated replay, no-op, raw, and SQLite backends.

- Dataset generation, end-to-end comparisons, and benchmark measurements are pending. No measured results are claimed.

See `dev/spec-overall.md` for the full specification.
