# Spec: Capture Report Dashboard

## Purpose

- Show a browser dashboard of the capture runs available under `sandbox/captured/`.
- Compare backends (`noop`, `raw`, `sqlite-no-index`, `sqlite-with-index`) for each dataset.
- Use only the files the tools already write. Rust code does not change.

## Files

```text
report.html          entry page at the repository root
report/main.js       ES module: discovery, loading, rendering, Canvas 2D charts
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
| `sandbox/captured/<name>/<backend>/capture-metrics.json` | run metrics, schema in `dev/spec-overall.md` |
| `sandbox/mock-data/<name>/manifest.json` | expected tick count, input size, rate, seed |

- A backend folder without `capture-metrics.json` is shown as incomplete. This covers runs that failed, were interrupted, or are still running.
- A missing manifest disables the tick conservation check and shows a warning.

## Discovery

Datasets are found with the first method that works:

1. Query parameter: `report.html?datasets=d001,m030` lists datasets explicitly.

2. Directory listing: an HTML listing of `sandbox/captured/` is parsed. Only direct child directories that match `^[dm]\d+$` are kept.

3. Probe: when no listing is available, the page tries the known names `m001`, `m010`, `m030`, `m060`, `d001`, and `d010`. Datasets with no run found are dropped.

Backends in each dataset are found the same way:

- With a directory listing, every child directory is shown, including incomplete runs and unknown backend names.

- Without a listing, the page probes the four known backends and shows only those that have metrics.

- Hidden entries, which start with `.`, are ignored.

## Views

- **Header:** title, load status (dataset and run counts, discovery method, update time), and a **Refresh** button.

- **Overview:** a table with one row per dataset and one column per backend. Each cell shows throughput and output size, `incomplete`, or `-`. Clicking a row selects that dataset.

- **Dataset tabs:** dataset name, duration label, and run count. The selected dataset is kept in the URL hash, for example `#d001`.

- **Dataset panel:**
  - Header: name, duration, tick count, input size, rate, and seed.
  - Warnings: missing manifest, mixed run configurations (speed, `flush_ms`, `ring_cap`), incomplete runs, and probe-only discovery.
  - Run cards: throughput, wall time, speedup, output size, bytes per tick, flush p50, p99, and max, stalls, finish time, configuration, and the tick conservation badge.
  - Charts: horizontal bar charts drawn with Canvas 2D for throughput, output size, bytes per tick, and flush p99. Charts redraw on resize and support HiDPI screens.
  - Comparison table: one row per metric and one column per backend. The best value in a row is highlighted.

## Derived Values

- `speedup = ticks_total * 0.005 / wall_time_s`, the replayed sensor time divided by wall time. Shown as `n/a` when wall time is zero.
- `bytes_per_tick = output_bytes / ticks_total`. Shown as `n/a` when no ticks were recorded.
- Input size: `tick_count * bytes_per_tick` from the manifest.
- The best value is the minimum or the maximum, depending on the metric. At least two recording backends must have a value. `noop` is excluded because it writes no recording.
- `noop` is also left out of the output size and bytes per tick charts.

## Formatting

- Byte sizes use decimal units: B, KB, MB, GB, TB.
- Rates use compact notation, for example `1.38M ticks/s`.
- Milliseconds show 3 decimals below 1 ms, 2 decimals below 100 ms, and whole numbers above that.
- Wall time is shown in seconds, minutes and seconds, or hours and minutes.

## Constraints

- The DOM is built with `createElement` and text nodes, never with `innerHTML` from fetched data.
- Elements are styled by class name. No element IDs are used.
- Layout uses CSS Grid.

## Limitations

- Discovery without a directory listing sees only the known names, or the names passed with `?datasets=`.
- Probing missing files can log 404 errors in the browser console.
- File sizes on disk are not read. Output size comes from `output_bytes` in the metrics.
- The flush percentiles are power-of-two bucket estimates, as described in `dev/spec-overall.md`.

## Deferred

- Cross-dataset charts, such as throughput as a function of duration.
- CPU and RSS measurements, once they are captured.
- A generated `sandbox/index.json` for servers without directory listing.
