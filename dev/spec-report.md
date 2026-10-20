# Spec: Capture Report Dashboard

## Purpose

- Show a browser dashboard of the capture runs available under `sandbox/captured/`.
- Compare backends (`noop`, `raw`, `sqlite-pk-index`, `sqlite-all-index`) for each dataset.
- Use only the files the tools already write. Rust code does not change.

## Files

```text
report.html          entry page at the repository root
report/main.js       ES module: discovery, loading, rendering, Canvas 2D charts
report/info.js       label descriptions for hover tooltips
report/main.css      layout (CSS Grid), light and dark themes
```

- No build step and no dependencies.
- The page targets browsers released within the last two years.

## Serving

- A separate static server serves the repository root. For example:

```sh
python3 -m http.server 8000
```

- Open `http://localhost:8000/report.html`.
- All fetches use relative paths from the root and `cache: "no-store"`, so **Refresh** shows new runs.

## Data Sources

| File | Use |
| ---- | --- |
| `sandbox/captured/<name>-<speed>/<backend>/capture-main-metrics.json` | main run metrics, schema in `dev/spec-overall.md`, falls back to `capture-metrics.json` |
| `sandbox/mock-data/<name>/manifest.json` | expected tick count, input size, rate, seed |
| `sandbox/captured/<name>-<speed>/<backend>/capture-main-ostel.jsonl` | per-thread and process CPU samples, read line by line, partial lines skipped, falls back to `cpu-samples.jsonl` |
| `sandbox/captured/<name>-<speed>/<backend>/capture-sum-metrics.json` | summary metrics, schema in `dev/spec-overall.md` |
| `sandbox/captured/<name>-<speed>/<backend>/capture-sum-ostel.jsonl` | `simrec-sum` thread CPU samples, read line by line, partial lines skipped |

- A backend folder without `capture-main-metrics.json` is shown as incomplete. This covers runs that failed, were interrupted, or are still running.
- A summary without `capture-sum-metrics.json` is shown as incomplete, and its cost is still shown from `capture-sum-ostel.jsonl` when that file exists.
- A missing manifest disables the tick conservation check and shows a warning.

## Discovery

Run folders are `<name>-<speed>`, where `<name>` is the mock dataset and `<speed>` is `real` (realtime) or `max`. Run folders are found with the first method that works:

1. Query parameter: `report.html?datasets=d001,m030` lists dataset names explicitly. Both speeds are probed for each name.

2. Directory listing: an HTML listing of `sandbox/captured/` is parsed. Only direct child directories that match `^[dm]\d+-(real|max)$` are kept. Other folders, including the old `<name>/<backend>` layout, are ignored.

3. Probe: when no listing is available, the page tries the known names `m001`, `m005`, `m010`, `m030`, `m060`, `d001`, and `d010` with both `real` and `max`. Datasets with no run found are dropped.

Backends in each run folder are found the same way:

- With a directory listing, every child directory is shown, including incomplete runs and unknown backend names.

- Without a listing, the page probes the four known backends and shows only those that have metrics.

- Hidden entries, which start with `.`, are ignored.

## Views

- **Header:** title, centered **Real** and **Max** speed tabs, load status (dataset and run counts, discovery method, update time), and a **Refresh** button.

- **Speed tabs:** two main tabs, **Real** and **Max**, each with its dataset and run counts. A tab with no runs is disabled. Every view below shows only the runs of the selected speed, because realtime and max runs are not comparable. On load, the page uses the speed from the URL hash when it has runs, and otherwise the first speed that has runs.

- **Overview:** a table for the selected speed, with one row per dataset that has runs at that speed and one column per backend. Each cell shows throughput, output size, the average CPU of the rec thread, `incomplete`, or `-`. Clicking a row selects that dataset.

- **Dataset tabs:** dataset name, duration label, and run count for the selected speed. The selected speed and dataset are kept in the URL hash as `#<speed>/<dataset>`, for example `#real/m010`. The older `#<dataset>` form is still accepted.

- **Dataset panel:** shows the selected dataset at the selected speed.
  - Header: name, duration, expected ticks for the duration, and rate. A second line shows the manifest source folder, its tick count, input size, and seed, because a shared manifest may describe a longer dataset. A third line lists the manifest channels and the bytes per tick, so the ten-channel tick layout is visible.
  - Warnings: missing manifest, a manifest version older than 3, mixed run configurations (`flush_ms`, `ring_cap`), incomplete runs, summary telemetry without `capture-sum-metrics.json`, and probe-only discovery.
  - Run cards: throughput, wall time, ticks, speedup, output size, bytes per tick, flush p50, p99, and max, stalls, finish time, configuration, sim thread, rec thread, and process CPU (average and peak), process RSS peak, and the tick conservation badge. A run without metrics still shows its CPU when its samples file exists.
  - Summary cost: sum thread CPU (average and peak), rows written and dropped, sum flush p99, and sum output size, shown when summary files exist.
  - Charts: horizontal bar charts drawn with Canvas 2D for throughput, output size, bytes per tick, flush p99, and sim, rec, and sum thread CPU. Charts redraw on resize and support HiDPI screens.
  - Comparison table: one row per metric and one column per backend, including the summary rows. The best value in a row is highlighted. Cells that display the same text as the best value are all highlighted.

## Derived Values

- `speedup = ticks_total * 0.005 / wall_time_s`, the replayed sensor time divided by wall time. Shown as `n/a` when wall time is zero.
- `bytes_per_tick = output_bytes / ticks_total`. Shown as `n/a` when no ticks were recorded.
- Input size: `tick_count * bytes_per_tick` from the manifest.
- CPU averages are the mean of the non-null `cpu_pct` values in `cpu-samples.jsonl`, and CPU peaks are the maximum. The first sample of each thread is null and is excluded.
- The best value is the minimum or the maximum, depending on the metric. At least two recording backends must have a value. `noop` is excluded because it writes no recording.
- `noop` is also left out of the output size and bytes per tick charts.
- `noop` stays in the CPU charts, where it shows the pipeline overhead without recording.

## Formatting

- Byte sizes use decimal units: B, KB, MB, GB, TB.
- Rates are rounded to whole ticks/s, then use compact notation, for example `1.38M ticks/s`.
- Table, card, and chart labels show a short description on hover, from `report/info.js`.
- Milliseconds show 3 decimals below 1 ms, 2 decimals below 100 ms, and whole numbers above that.
- Wall time is shown in seconds, minutes and seconds, or hours and minutes.

## Constraints

- The DOM is built with `createElement` and text nodes, never with `innerHTML` from fetched data.
- Elements are styled by class name. No element IDs are used.
- Layout uses CSS Grid.

## Limitations

- Discovery without a directory listing sees only the known names, or the names passed with `?datasets=`.
- Probing missing files can log 404 errors in the browser console.
- CPU averages come from sampling, so activity after the last sample is not counted.
- File sizes on disk are not read. Output size comes from `output_bytes` in the metrics.
- The flush percentiles are power-of-two bucket estimates, as described in `dev/spec-overall.md`.

## Deferred

- Cross-dataset charts, such as throughput as a function of duration.
- A generated `sandbox/index.json` for servers without directory listing.
