import { LABEL_INFO } from "./info.js";

// region:    --- Constants

const CAPTURED_DIR = "sandbox/captured/";
const MOCK_DIR = "sandbox/mock-data/";
const METRICS_FILE = "capture-main-metrics.json";
const LEGACY_METRICS_FILE = "capture-metrics.json";
const MANIFEST_FILE = "manifest.json";
const MANIFEST_VERSION = 3;
const MAIN_OSTEL_FILE = "capture-main-ostel.jsonl";
const LEGACY_OSTEL_FILE = "cpu-samples.jsonl";
const SUM_METRICS_FILE = "capture-sum-metrics.json";
const SUM_OSTEL_FILE = "capture-sum-ostel.jsonl";
const SIM_THREAD = "simrec-sim";
const DISPATCH_THREAD = "simrec-dispatch";
const REC_THREADS = ["simrec-rec-main", "simrec-rec"];
const SUM_THREADS = ["simrec-rec-sum", "simrec-sum"];
const TICK_SECONDS = 0.005;
const DATASET_NAME = /^[dm]\d+$/;
const RUN_DIR_NAME = /^([dm]\d+)-(real|max)$/;
const SPEEDS = ["real", "max"];
const MINUTES_PER_DAY = 1440;
const HORIZONS = [
  { label: "None", days: null },
  { label: "1 day", days: 1 },
  { label: "10 days", days: 10 },
];
const SCALED_FIELDS = ["ticks_total", "wall_time_s", "output_bytes", "flush_count", "stall_count", "dispatch_stall_count"];
const SCALED_SUM_FIELDS = [
  "rows_emitted",
  "rows_written",
  "rows_dropped",
  "ticks_summarized",
  "ticks_received",
  "ticks_dropped",
  "batches_dropped",
  "partial_rows",
  "output_bytes",
];

const KNOWN_BACKENDS = ["raw", "sqlite-pk-index", "sqlite-all-index", "noop"];
const FALLBACK_DATASETS = ["m001", "m005", "m010", "m030", "m060", "d001", "d010"];
const MANIFEST_FALLBACKS = ["d001", "d010"];

const BACKEND_COLORS = {
  noop: "#8a94a6",
  raw: "#2f9e6e",
  "sqlite-pk-index": "#3b7dd8",
  "sqlite-all-index": "#d9822b",
};
const EXTRA_COLORS = ["#a855f7", "#e11d48", "#0891b2", "#65a30d"];

const CHARTS = [
  { title: "Throughput", unit: "ticks/s, higher is better", get: (m) => m.ticks_per_s, format: formatRate },
  { title: "Output size", unit: "lower is better", get: (m) => m.output_bytes, format: formatBytes, skipNoop: true },
  { title: "Bytes per tick", unit: "lower is better", get: bytesPerTick, format: formatDecimal, skipNoop: true },
  { title: "Flush latency p99", unit: "ms, lower is better", get: (m) => m.flush_ms_p99, format: formatMs },
  { title: "Sim thread CPU", unit: "% of one core, lower is better", get: (m) => m.cpu?.sim?.avg, format: formatPercent },
  { title: "Dispatch thread CPU", unit: "% of one core, lower is better", get: (m) => m.cpu?.dispatch?.avg, format: formatPercent },
  { title: "Rec thread CPU", unit: "% of one core, lower is better", get: (m) => m.cpu?.rec?.avg, format: formatPercent },
  { title: "Sum thread CPU", unit: "% of one core, lower is better", get: (m) => m.sumCpu?.avg, format: formatPercent },
];

const TABLE_ROWS = [
  { label: "Speed", get: (m) => m.speed, format: String },
  { label: "Flush interval", get: (m) => m.flush_ms, format: (v) => `${v} ms` },
  { label: "Ring capacity", get: (m) => m.ring_cap, format: formatInt },
  { label: "Ticks recorded", get: (m) => m.ticks_total, format: formatInt },
  { label: "Wall time", get: (m) => m.wall_time_s, format: formatSeconds, better: "low" },
  { label: "Throughput", get: (m) => m.ticks_per_s, format: formatRate, better: "high" },
  { label: "Speedup vs realtime", get: speedup, format: (v) => `${v.toFixed(1)}x`, better: "high" },
  { label: "Output size", get: (m) => m.output_bytes, format: formatBytes, better: "low" },
  { label: "Bytes per tick", get: bytesPerTick, format: formatDecimal, better: "low" },
  { label: "Stalls", get: (m) => m.stall_count, format: formatInt, better: "low" },
  { label: "Dispatch stalls", get: (m) => m.dispatch_stall_count, format: formatInt, better: "low" },
  { label: "Max pending ticks", get: (m) => m.max_pending_len, format: formatInt, better: "low" },
  { label: "Flush count", get: (m) => m.flush_count, format: formatInt },
  { label: "Flush p50", get: (m) => m.flush_ms_p50, format: formatMs, better: "low" },
  { label: "Flush p99", get: (m) => m.flush_ms_p99, format: formatMs, better: "low" },
  { label: "Flush max", get: (m) => m.flush_ms_max, format: formatMs, better: "low" },
  { label: "Finish", get: (m) => m.finish_ms, format: formatMs, better: "low" },
  { label: "Sim thread CPU", get: (m) => m.cpu?.sim?.avg ?? null, format: formatPercent, better: "low" },
  { label: "Dispatch thread CPU", get: (m) => m.cpu?.dispatch?.avg ?? null, format: formatPercent, better: "low" },
  { label: "Rec thread CPU", get: (m) => m.cpu?.rec?.avg ?? null, format: formatPercent, better: "low" },
  { label: "Process CPU", get: (m) => m.cpu?.process?.avg ?? null, format: formatPercent, better: "low" },
  { label: "Process RSS peak", get: (m) => m.cpu?.rssPeak ?? null, format: formatBytes, better: "low" },
  { label: "Sum thread CPU", get: (m) => m.sumCpu?.avg ?? null, format: formatPercent, better: "low" },
  { label: "Sum rows written", get: (m) => m.sum?.rows_written, format: formatInt },
  { label: "Sum rows dropped", get: (m) => m.sum?.rows_dropped, format: formatInt, better: "low" },
  { label: "Sum ticks dropped", get: (m) => m.sum?.ticks_dropped, format: formatInt, better: "low" },
  { label: "Sum batches dropped", get: (m) => m.sum?.batches_dropped, format: formatInt, better: "low" },
  { label: "Sum partial rows", get: (m) => m.sum?.partial_rows, format: formatInt, better: "low" },
  { label: "Sum flush p99", get: (m) => m.sum?.flush_ms_p99, format: formatMs, better: "low" },
  { label: "Sum output size", get: (m) => m.sum?.output_bytes, format: formatBytes, better: "low" },
];

// endregion: --- Constants

// region:    --- State

const ui = {
  status: document.querySelector(".app-status"),
  refresh: document.querySelector(".refresh-button"),
  speedNav: document.querySelector(".speed-nav"),
  overview: document.querySelector(".overview"),
  nav: document.querySelector(".dataset-nav"),
  panel: document.querySelector(".dataset-panel"),
};

const hashSelection = readHash();

const state = {
  datasets: [],
  discovery: null,
  speed: hashSelection.speed,
  selected: hashSelection.dataset,
  horizon: HORIZONS[0],
  charts: [],
  loadedAt: null,
  loading: false,
  error: null,
};

// endregion: --- State

// region:    --- Loading

async function loadReport() {
  state.loading = true;
  state.error = null;
  ui.refresh.disabled = true;
  renderStatus();
  try {
    const discovery = await discoverDatasets();
    const groups = groupRunDirs(discovery.runDirs);
    const datasets = await Promise.all(
      groups.map(({ name, speeds }) => loadDataset(name, speeds, discovery.mode === "listing")),
    );
    state.discovery = discovery.mode;
    // Probed names are guesses, so only datasets with real runs are kept.
    state.datasets = datasets
      .filter((dataset) => discovery.mode !== "probe" || dataset.runs.length > 0)
      .sort(compareDatasets);
    reconcileSelection();
    state.loadedAt = new Date();
  } catch (error) {
    state.error = String(error);
  } finally {
    state.loading = false;
    ui.refresh.disabled = false;
  }
  render();
}

async function discoverDatasets() {
  const explicit = new URLSearchParams(location.search).get("datasets");
  if (explicit) {
    const names = explicit.split(",").map((name) => name.trim()).filter(Boolean);
    return { mode: "query", runDirs: names.flatMap(speedRunDirs) };
  }
  const listed = await listDirectories(CAPTURED_DIR);
  if (listed) {
    return { mode: "listing", runDirs: listed.map(parseRunDir).filter(Boolean) };
  }
  return { mode: "probe", runDirs: FALLBACK_DATASETS.flatMap(speedRunDirs) };
}

async function loadDataset(name, speeds, listed) {
  const [found, runs] = await Promise.all([
    loadManifest(name),
    Promise.all(speeds.map((speed) => loadRunGroup(name, speed))),
  ]);
  return {
    name,
    manifest: found?.manifest ?? null,
    manifestSource: found?.source ?? null,
    listed,
    runs: runs.flat().sort(compareRuns),
  };
}

/// Manifests are shared mock data, so a dataset without its own manifest uses the d001 or d010 one.
async function loadManifest(name) {
  for (const source of manifestSources(name)) {
    const manifest = await fetchJson(`${MOCK_DIR}${encodeURIComponent(source)}/${MANIFEST_FILE}`);
    if (manifest) {
      return { source, manifest };
    }
  }
  return null;
}

/// Returns the dataset names whose manifest may describe this dataset, in lookup order.
function manifestSources(name) {
  return [...new Set([name, ...MANIFEST_FALLBACKS])];
}

async function loadRunGroup(name, speed) {
  const datasetUrl = `${CAPTURED_DIR}${encodeURIComponent(`${name}-${speed}`)}/`;
  const listed = await listDirectories(datasetUrl);
  const backends = listed ?? KNOWN_BACKENDS;
  const runs = await Promise.all(backends.map((backend) => loadRun(datasetUrl, backend, speed)));
  return runs.filter((run) => listed !== null || run.metrics !== null || run.cpu !== null);
}

async function loadRun(datasetUrl, backend, speed) {
  const base = `${datasetUrl}${encodeURIComponent(backend)}/`;
  const [metrics, cpuText, sumMetrics, sumCpuText] = await Promise.all([
    fetchFirst(base, [METRICS_FILE, LEGACY_METRICS_FILE], fetchJson),
    fetchFirst(base, [MAIN_OSTEL_FILE, LEGACY_OSTEL_FILE], fetchText),
    fetchJson(`${base}${SUM_METRICS_FILE}`),
    fetchText(`${base}${SUM_OSTEL_FILE}`),
  ]);
  const cpu = summarizeCpu(cpuText);
  const sumCpu = summarizeSumCpu(sumCpuText);
  return {
    backend,
    speed,
    cpu,
    sumCpu,
    sum: sumMetrics,
    sumOstel: sumCpuText != null,
    metrics: metrics && { ...metrics, cpu, sumCpu, sum: sumMetrics },
  };
}

/// Returns the direct child directory names of a server directory listing, or null when unavailable.
async function listDirectories(url) {
  let response;
  try {
    response = await fetch(url, { cache: "no-store" });
  } catch {
    return null;
  }
  const contentType = response.headers.get("content-type") ?? "";
  if (!response.ok || !contentType.includes("html")) {
    return null;
  }
  const doc = new DOMParser().parseFromString(await response.text(), "text/html");
  const base = new URL(url, location.href);
  const names = new Set();
  for (const anchor of doc.querySelectorAll("a[href]")) {
    const target = new URL(anchor.getAttribute("href"), base);
    if (target.origin !== base.origin || !target.pathname.startsWith(base.pathname)) {
      continue;
    }
    const rest = target.pathname.slice(base.pathname.length);
    if (!rest.endsWith("/")) {
      continue;
    }
    const name = decodeURIComponent(rest.slice(0, -1));
    if (name && !name.includes("/") && !name.startsWith(".")) {
      names.add(name);
    }
  }
  return [...names];
}

async function fetchJson(url) {
  try {
    const response = await fetch(url, { cache: "no-store" });
    if (!response.ok) {
      return null;
    }
    return await response.json();
  } catch {
    return null;
  }
}

async function fetchText(url) {
  try {
    const response = await fetch(url, { cache: "no-store" });
    return response.ok ? await response.text() : null;
  } catch {
    return null;
  }
}

/// Returns the first named file that loads, trying the current name before the legacy one.
async function fetchFirst(base, names, read) {
  for (const name of names) {
    const value = await read(`${base}${name}`);
    if (value != null) {
      return value;
    }
  }
  return null;
}

/// Parses JSON Lines one line at a time, skipping blank lines and lines that do not parse, such as a trailing partial line.
function parseJsonLines(text) {
  const lines = [];
  for (const line of text.split("\n")) {
    if (!line.trim()) {
      continue;
    }
    try {
      lines.push(JSON.parse(line));
    } catch {
      // Partial or invalid line, skipped.
    }
  }
  return lines;
}

/// Summarizes one run's `cpu-samples.jsonl`. The first sample of each thread has a null `cpu_pct` and is excluded.
function summarizeCpu(text) {
  if (text == null) {
    return null;
  }
  const threads = new Map([
    [SIM_THREAD, []],
    [DISPATCH_THREAD, []],
    ...REC_THREADS.map((name) => [name, []]),
  ]);
  const processPct = [];
  let rssPeak = null;
  let error = null;
  for (const line of parseJsonLines(text)) {
    const values = line.kind === "thread" ? threads.get(line.thread) : null;
    if (values) {
      values.push(line.cpu_pct);
    } else if (line.kind === "process") {
      processPct.push(line.cpu_pct);
      if (typeof line.rss_bytes === "number") {
        rssPeak = Math.max(rssPeak ?? 0, line.rss_bytes);
      }
    } else if (line.kind === "error") {
      error ??= line.message;
    }
  }
  return {
    sim: pctStats(threads.get(SIM_THREAD) ?? []),
    dispatch: pctStats(threads.get(DISPATCH_THREAD) ?? []),
    rec: statsForThreads(threads, REC_THREADS),
    process: pctStats(processPct),
    rssPeak,
    error,
  };
}

/// Returns the average and peak of the numeric values, or null when there are none.
function pctStats(values) {
  const numbers = values.filter((value) => typeof value === "number" && Number.isFinite(value));
  if (!numbers.length) {
    return null;
  }
  const total = numbers.reduce((sum, value) => sum + value, 0);
  const peak = numbers.reduce((max, value) => Math.max(max, value), 0);
  return { avg: total / numbers.length, peak };
}

/// Returns the stats of the first thread name that has samples, so a capture never mixes the new and legacy thread names.
function statsForThreads(threads, names) {
  for (const name of names) {
    const values = threads.get(name) ?? [];
    if (values.length) {
      return pctStats(values);
    }
  }
  return null;
}

/// Summarizes the `simrec-rec-sum` thread lines of `capture-sum-ostel.jsonl`, or returns null when the file is missing.
function summarizeSumCpu(text) {
  if (text == null) {
    return null;
  }
  const threads = new Map(SUM_THREADS.map((name) => [name, []]));
  for (const line of parseJsonLines(text)) {
    const values = line.kind === "thread" ? threads.get(line.thread) : null;
    if (values) {
      values.push(line.cpu_pct);
    }
  }
  return statsForThreads(threads, SUM_THREADS);
}

// endregion: --- Loading

// region:    --- Rendering

function render() {
  renderStatus();
  renderView();
}

function renderView() {
  renderSpeedNav();
  renderOverview();
  renderNav();
  renderPanel();
}

function renderStatus() {
  if (state.loading) {
    ui.status.textContent = "Loading...";
    return;
  }
  if (state.error) {
    ui.status.textContent = `Load failed: ${state.error}`;
    return;
  }
  const runCount = state.datasets.reduce((total, dataset) => total + dataset.runs.length, 0);
  const discovery = {
    listing: "directory listing",
    probe: "probe of known names",
    query: "query parameter",
  }[state.discovery];
  const time = state.loadedAt?.toLocaleTimeString() ?? "";
  ui.status.textContent = `${state.datasets.length} dataset(s), ${runCount} run(s), via ${discovery}, updated ${time}`;
}

function renderSpeedNav() {
  ui.speedNav.replaceChildren(
    ...SPEEDS.map((speed) => {
      const datasets = speedDatasets(speed);
      const runCount = datasets.reduce((total, dataset) => total + speedRuns(dataset, speed).length, 0);
      return h(
        "button",
        {
          type: "button",
          class: "speed-tab",
          "aria-pressed": speed === state.speed ? "true" : "false",
          disabled: datasets.length === 0,
          onclick: () => selectSpeed(speed),
        },
        h("span", { class: "speed-tab-name" }, speed === "real" ? "Real" : "Max"),
        h("span", { class: "speed-tab-meta" }, `${datasets.length} dataset(s), ${runCount} run(s)`),
      );
    }),
  );
}

function renderOverview() {
  const datasets = speedDatasets(state.speed);
  const backends = sortBackends([
    ...new Set(datasets.flatMap((dataset) => speedRuns(dataset, state.speed).map((run) => run.backend))),
  ]);
  if (!backends.length) {
    ui.overview.replaceChildren();
    return;
  }
  const table = h(
    "table",
    { class: "overview-table" },
    h(
      "thead",
      {},
      h(
        "tr",
        {},
        h("th", { scope: "col" }, "Dataset"),
        backends.map((backend) => h("th", { scope: "col" }, renderBackendLabel(backend))),
      ),
    ),
    h(
      "tbody",
      {},
      datasets.map((dataset) => {
        const runs = speedRuns(dataset, state.speed);
        return h(
          "tr",
          {
            class: dataset.name === state.selected ? "is-selected" : null,
            onclick: () => selectDataset(dataset.name),
          },
          h(
            "th",
            { scope: "row" },
            h("span", { class: "overview-name" }, dataset.name),
            h("span", { class: "overview-duration" }, durationLabel(dataset.name)),
          ),
          backends.map((backend) => renderOverviewCell(runs.find((run) => run.backend === backend))),
        );
      }),
    ),
  );
  ui.overview.replaceChildren(
    h("h2", { class: "section-title" }, `Overview, ${speedLabel(state.speed)}`),
    h("div", { class: "table-wrap" }, table),
  );
}

function renderOverviewCell(run) {
  if (!run) {
    return h("td", { class: "overview-cell is-absent" }, "-");
  }
  const cpu = run.cpu ? h(
    "span",
    { class: "overview-cpu" },
    `CPU rec ${formatOptional(run.cpu?.rec?.avg, formatPercent)}`,
  ) : null;
  if (!run.metrics) {
    return h("td", { class: "overview-cell is-incomplete" }, "incomplete", cpu);
  }
  return h(
    "td",
    { class: "overview-cell" },
    h("span", { class: "overview-rate" }, formatRate(run.metrics.ticks_per_s)),
    h("span", { class: "overview-size" }, formatBytes(run.metrics.output_bytes)),
    cpu,
  );
}

function renderNav() {
  ui.nav.replaceChildren(
    ...speedDatasets(state.speed).map((dataset) =>
      h(
        "button",
        {
          type: "button",
          class: "dataset-tab",
          "aria-pressed": dataset.name === state.selected ? "true" : "false",
          onclick: () => selectDataset(dataset.name),
        },
        h("span", { class: "dataset-tab-name" }, dataset.name),
        h(
          "span",
          { class: "dataset-tab-meta" },
          `${durationLabel(dataset.name)}, ${speedRuns(dataset, state.speed).length} run(s)`,
        ),
      ),
    ),
  );
}

function renderPanel() {
  state.charts = [];
  const dataset = speedDatasets(state.speed).find((item) => item.name === state.selected);
  if (!dataset) {
    ui.panel.replaceChildren(renderEmptyState());
    return;
  }
  const runs = speedRuns(dataset, state.speed);
  const complete = runs.filter((run) => run.metrics);
  const horizon = state.speed === "real" ? state.horizon : HORIZONS[0];
  const factor = horizonFactor(dataset, horizon);
  const scaled = complete.map((run) => ({ ...run, metrics: extrapolateMetrics(run.metrics, factor) }));
  const sections = [
    renderDatasetHeader(dataset),
    renderWarnings(dataset, runs),
    h(
      "section",
      { class: "report-section" },
      h("h3", { class: "section-title" }, `Runs, ${speedLabel(state.speed)}`),
      h("div", { class: "cards-grid" }, runs.map((run) => renderRunCard(run, dataset))),
    ),
    renderHorizonToggle(dataset),
    renderCharts(scaled, state.speed, horizon),
    renderComparisonTable(scaled, state.speed),
  ];
  ui.panel.replaceChildren(...sections.filter(Boolean));
  drawCharts();
}

function renderEmptyState() {
  if (state.loading) {
    return h("p", { class: "muted" }, "Loading captured data...");
  }
  return h(
    "div",
    { class: "empty-state" },
    h("h2", {}, "No captured data found"),
    h(
      "ul",
      {},
      h("li", {}, "Generate a dataset with mocker and replay it with simrec into sandbox/."),
      h("li", {}, "Serve the repository root so that sandbox/captured/ is reachable from this page."),
      h("li", {}, "Without server directory listing, pass names explicitly, for example ?datasets=d001,m030."),
    ),
  );
}

function renderDatasetHeader(dataset) {
  const manifest = dataset.manifest;
  const facts = [durationLabel(dataset.name)];
  const minutes = durationMinutes(dataset.name);
  if (manifest && minutes !== null) {
    facts.push(`${formatInt(minutes * 60 * manifest.rate_hz)} ticks at ${manifest.rate_hz} Hz`);
  }
  const source = manifest
    ? h(
        "p",
        { class: "dataset-facts small" },
        `Manifest ${dataset.manifestSource}: ${formatInt(manifest.tick_count)} ticks, input ${formatBytes(manifest.tick_count * manifest.bytes_per_tick)}, seed ${manifest.seed}`,
      )
    : null;
  const channels = manifest?.channels?.length
    ? h(
        "p",
        { class: "dataset-facts small" },
        `Channels, ${manifest.channels.length} at ${manifest.bytes_per_tick} bytes/tick: ${manifest.channels.join(", ")}`,
      )
    : null;
  return h(
    "header",
    { class: "dataset-header" },
    h("h2", { class: "dataset-title" }, dataset.name),
    h("p", { class: "dataset-facts" }, facts.join(", ")),
    source,
    channels,
  );
}

function renderWarnings(dataset, runs) {
  const messages = [];
  const complete = runs.filter((run) => run.metrics);
  if (!dataset.manifest) {
    const paths = manifestSources(dataset.name).map((source) => `${MOCK_DIR}${source}/${MANIFEST_FILE}`);
    messages.push(`Manifest not found at ${paths.join(", ")}; tick conservation is not checked.`);
  }
  if (dataset.manifest && dataset.manifest.version !== MANIFEST_VERSION) {
    messages.push(
      `Manifest ${dataset.manifestSource} is version ${dataset.manifest.version ?? "unknown"}; regenerate it and its captures with the current mocker (version ${MANIFEST_VERSION}).`,
    );
  }
  const configs = new Set(complete.map((run) => configKey(run.metrics)));
  if (configs.size > 1) {
    messages.push(`${speedLabel(state.speed)} runs use different flush interval or ring capacity; comparisons are not like-for-like.`);
  }
  const incomplete = runs.length - complete.length;
  if (incomplete > 0) {
    messages.push(`${incomplete} run(s) without ${METRICS_FILE}: incomplete, failed, or still running.`);
  }
  const withoutCpu = runs.filter((run) => !run.cpu).length;
  if (withoutCpu > 0) {
    messages.push(`${withoutCpu} run(s) without ${MAIN_OSTEL_FILE}: CPU is not shown for them.`);
  }
  const withoutSum = runs.filter((run) => run.sumOstel && !run.sum).length;
  if (withoutSum > 0) {
    messages.push(`${withoutSum} run(s) without ${SUM_METRICS_FILE}: summary is incomplete, failed, or still running.`);
  }
  if (!dataset.listed) {
    messages.push("Directory listing is unavailable, so runs were probed by known names only.");
  }
  if (!messages.length) {
    return null;
  }
  return h("ul", { class: "warnings" }, messages.map((message) => h("li", {}, message)));
}

function renderRunCard(run, dataset) {
  const metrics = run.metrics;
  const header = h(
    "header",
    { class: "run-card-header" },
    h("h3", { class: "run-card-title" }, renderBackendLabel(run.backend)),
    metrics ? h("span", { class: "badge" }, metrics.speed) : h("span", { class: "badge is-warn" }, "incomplete"),
  );
  const card = h("article", { class: "run-card", style: `--backend-color: ${backendColor(run.backend)}` }, header);
  if (!metrics) {
    card.append(h("p", { class: "run-card-empty" }, `No ${METRICS_FILE}. The run is incomplete, failed, or still running.`));
    if (run.cpu) {
      card.append(renderStatList(cpuRows(run.cpu)), ...renderCpuError(run.cpu));
    }
    return card;
  }
  const stats = [
    ["Wall time", formatSeconds(metrics.wall_time_s)],
    ["Ticks", formatInt(metrics.ticks_total)],
    ["Speedup", formatOptional(speedup(metrics), (v) => `${v.toFixed(1)}x realtime`)],
    ["Output", formatBytes(metrics.output_bytes)],
    ["Bytes/tick", formatOptional(bytesPerTick(metrics), formatDecimal)],
    ["Flush p50 / p99", `${formatMs(metrics.flush_ms_p50)} / ${formatMs(metrics.flush_ms_p99)}`],
    ["Flush max", formatMs(metrics.flush_ms_max)],
    ["Stalls", formatInt(metrics.stall_count)],
    ["Dispatch stalls", formatOptional(metrics.dispatch_stall_count, formatInt)],
    ["Finish", formatMs(metrics.finish_ms)],
    ["Config", `flush ${metrics.flush_ms} ms, ring ${formatInt(metrics.ring_cap)}`],
    ...cpuRows(run.cpu),
    ...(run.sumOstel || run.sum ? sumRows(run) : []),
  ];
  card.append(
    h(
      "div",
      { class: "run-card-hero" },
      h("span", { class: "run-card-hero-value" }, formatCompact(metrics.ticks_per_s)),
      h("span", { class: "run-card-hero-label" }, "ticks/s"),
    ),
    renderStatList(stats),
  );
  const conservation = renderConservation(metrics, dataset.manifest);
  if (conservation) {
    card.append(conservation);
  }
  card.append(...renderCpuError(run.cpu));
  return card;
}

function renderStatList(stats) {
  return h(
    "dl",
    { class: "run-card-stats" },
    stats.map(([label, value]) => [h("dt", { "data-tip": LABEL_INFO[label] }, label), h("dd", {}, value)]),
  );
}

function cpuRows(cpu) {
  return [
    ["Sim thread CPU", formatCpuStat(cpu?.sim)],
    ["Dispatch thread CPU", formatCpuStat(cpu?.dispatch)],
    ["Rec thread CPU", formatCpuStat(cpu?.rec)],
    ["Process CPU", formatCpuStat(cpu?.process)],
    ["Process RSS peak", formatOptional(cpu?.rssPeak, formatBytes)],
  ];
}

function sumRows(run) {
  return [
    ["Sum thread CPU", formatCpuStat(run.sumCpu)],
    ["Sum rows written", formatOptional(run.sum?.rows_written, formatInt)],
    ["Sum rows dropped", formatOptional(run.sum?.rows_dropped, formatInt)],
    ["Sum ticks dropped", formatOptional(run.sum?.ticks_dropped, formatInt)],
    ["Sum batches dropped", formatOptional(run.sum?.batches_dropped, formatInt)],
    ["Sum partial rows", formatOptional(run.sum?.partial_rows, formatInt)],
    ["Sum flush p99", formatOptional(run.sum?.flush_ms_p99, formatMs)],
    ["Sum output size", formatOptional(run.sum?.output_bytes, formatBytes)],
  ];
}

function formatCpuStat(stat) {
  return stat ? `${formatPercent(stat.avg)} avg, ${formatPercent(stat.peak)} peak` : "n/a";
}

function renderCpuError(cpu) {
  if (!cpu?.error) {
    return [];
  }
  return [h("p", { class: "run-card-check is-warn" }, `CPU sampling stopped: ${cpu.error}`)];
}

function renderConservation(metrics, manifest) {
  if (!manifest) {
    return null;
  }
  if (metrics.ticks_total === manifest.tick_count) {
    return h("p", { class: "run-card-check is-ok" }, `${formatInt(metrics.ticks_total)} ticks conserved`);
  }
  return h(
    "p",
    { class: "run-card-check is-neutral" },
    `Tick window: ${formatInt(metrics.ticks_total)} of ${formatInt(manifest.tick_count)}`,
  );
}

function renderCharts(runs, speed, horizon) {
  if (!runs.length) {
    return null;
  }
  const figures = CHARTS.map((chart) => {
    const bars = runs
      .filter((run) => !(chart.skipNoop && run.backend === "noop"))
      .filter((run) => chart.get(run.metrics) != null)
      .map((run) => ({
        label: run.backend,
        value: chart.get(run.metrics) ?? 0,
        color: backendColor(run.backend),
      }));
    if (!bars.length) {
      return null;
    }
    const description = bars.map((bar) => `${bar.label} ${chart.format(bar.value)}`).join(", ");
    const canvas = h("canvas", { class: "chart-canvas", role: "img", "aria-label": `${chart.title}: ${description}` });
    state.charts.push({ canvas, bars, format: chart.format });
    return h(
      "figure",
      { class: "chart" },
      h(
        "figcaption",
        { class: "chart-caption" },
        h("span", { class: "chart-title", "data-tip": LABEL_INFO[chart.title] }, chart.title),
        h("span", { class: "chart-unit" }, chart.unit),
      ),
      canvas,
    );
  });
  return h(
    "section",
    { class: "report-section" },
    h("h3", { class: "section-title" }, `Charts, ${speedLabel(speed)}${horizon.days ? `, extrapolated to ${horizon.label}` : ""}`),
    h("div", { class: "charts-grid" }, figures),
  );
}

function renderComparisonTable(runs, speed) {
  if (!runs.length) {
    return null;
  }
  const head = h(
    "thead",
    {},
    h(
      "tr",
      {},
      h("th", { scope: "col" }, "Metric"),
      runs.map((run) => h("th", { scope: "col" }, renderBackendLabel(run.backend))),
    ),
  );
  const body = h(
    "tbody",
    {},
    TABLE_ROWS.map((row) => {
      const values = runs.map((run) => row.get(run.metrics));
      const best = bestValue(row, runs, values);
      const bestText = best === null ? null : formatOptional(best, row.format);
      return h(
        "tr",
        {},
        h("th", { scope: "row", "data-tip": LABEL_INFO[row.label] }, row.label),
        values.map((value, index) => {
          const text = formatOptional(value, row.format);
          return h(
            "td",
            { class: text === bestText && runs[index].backend !== "noop" ? "is-best" : null },
            text,
          );
        }),
      );
    }),
  );
  return h(
    "section",
    { class: "report-section" },
    h("h3", { class: "section-title" }, `Comparison, ${speedLabel(speed)}`),
    h("div", { class: "table-wrap" }, h("table", { class: "comparison-table" }, head, body)),
    h("p", { class: "muted small" }, "Highlighted cells are the best value among recording backends; noop is excluded."),
  );
}

/// Shows the extrapolation toggle for the Real view, where wall time matches the recorded duration.
function renderHorizonToggle(dataset) {
  if (state.speed !== "real") {
    return null;
  }
  const note = state.horizon.days
    ? `Output size, ticks, wall time, flush count, stalls, dispatch stalls, summary rows, ticks and batches dropped before the summary, partial rows, and summary output are scaled by ${formatInt(horizonFactor(dataset, state.horizon))}x for ${dataset.name}. Rates and per-tick values are unchanged.`
    : null;
  return h(
    "section",
    { class: "report-section" },
    h("h3", { class: "section-title" }, "Extrapolation"),
    h(
      "div",
      { class: "horizon-toggle", role: "group", "aria-label": "Extrapolate to" },
      HORIZONS.map((horizon) =>
        h(
          "button",
          {
            type: "button",
            class: "horizon-tab",
            "aria-pressed": horizon === state.horizon ? "true" : "false",
            onclick: () => selectHorizon(horizon),
          },
          horizon.label,
        ),
      ),
    ),
    note ? h("p", { class: "muted small" }, note) : null,
  );
}

function renderBackendLabel(backend) {
  return h("span", { class: "backend-label", style: `--backend-color: ${backendColor(backend)}` }, backend);
}

// endregion: --- Rendering

// region:    --- Charts

function drawCharts() {
  for (const chart of state.charts) {
    drawBarChart(chart.canvas, chart.bars, chart.format);
  }
}

function drawBarChart(canvas, bars, format) {
  const rowHeight = 30;
  const padding = 8;
  const height = bars.length * rowHeight + padding * 2;
  canvas.style.height = `${height}px`;
  const width = canvas.clientWidth;
  if (width === 0) {
    return;
  }
  const ratio = window.devicePixelRatio || 1;
  canvas.width = Math.round(width * ratio);
  canvas.height = Math.round(height * ratio);

  const ctx = canvas.getContext("2d");
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx.clearRect(0, 0, width, height);
  ctx.font = "12px system-ui, -apple-system, Segoe UI, sans-serif";
  ctx.textBaseline = "middle";

  const textColor = getComputedStyle(canvas).color;
  const labels = bars.map((bar) => bar.label);
  const values = bars.map((bar) => format(bar.value));
  const labelWidth = Math.min(Math.max(...labels.map((label) => ctx.measureText(label).width)) + 12, width * 0.4);
  const valueWidth = Math.max(...values.map((value) => ctx.measureText(value).width)) + 12;
  const barArea = Math.max(width - labelWidth - valueWidth, 10);
  const max = Math.max(...bars.map((bar) => bar.value), 0);

  bars.forEach((bar, index) => {
    const top = padding + index * rowHeight;
    const middle = top + rowHeight / 2;
    const barWidth = max > 0 ? (bar.value / max) * barArea : 0;

    ctx.fillStyle = textColor;
    ctx.textAlign = "right";
    ctx.fillText(bar.label, labelWidth - 8, middle, labelWidth - 8);

    ctx.fillStyle = bar.color;
    ctx.fillRect(labelWidth, top + 6, bar.value > 0 ? Math.max(barWidth, 2) : 0, rowHeight - 12);

    ctx.fillStyle = textColor;
    ctx.textAlign = "left";
    ctx.fillText(values[index], labelWidth + barWidth + 6, middle);
  });
}

// endregion: --- Charts

// region:    --- Support

/// Sets the extrapolation horizon used by the Real view charts and comparison table.
function selectHorizon(horizon) {
  state.horizon = horizon;
  renderPanel();
}

/// Returns how many times the dataset recording must repeat to cover the horizon, or 1 when there is none.
function horizonFactor(dataset, horizon) {
  const minutes = durationMinutes(dataset.name);
  if (!horizon.days || !minutes) {
    return 1;
  }
  return (horizon.days * MINUTES_PER_DAY) / minutes;
}

/// Scales the fields that grow with recording length. Rates, per-tick values, latencies, and CPU stay as measured.
function extrapolateMetrics(metrics, factor) {
  if (factor === 1) {
    return metrics;
  }
  const scaled = { ...metrics };
  for (const field of SCALED_FIELDS) {
    if (typeof scaled[field] === "number") {
      scaled[field] *= factor;
    }
  }
  if (metrics.sum) {
    scaled.sum = { ...metrics.sum };
    for (const field of SCALED_SUM_FIELDS) {
      if (typeof scaled.sum[field] === "number") {
        scaled.sum[field] *= factor;
      }
    }
  }
  return scaled;
}

function selectSpeed(speed) {
  state.speed = speed;
  reconcileSelection();
  writeHash();
  renderView();
}

function selectDataset(name) {
  state.selected = name;
  writeHash();
  renderView();
}

/// Keeps the selected speed and dataset valid, falling back to the first ones that have runs.
function reconcileSelection() {
  if (!speedDatasets(state.speed).length) {
    state.speed = SPEEDS.find((speed) => speedDatasets(speed).length > 0) ?? SPEEDS[0];
  }
  const datasets = speedDatasets(state.speed);
  if (!datasets.some((dataset) => dataset.name === state.selected)) {
    state.selected = datasets[0]?.name ?? null;
  }
}

/// Reads `#<speed>/<dataset>`, `#<speed>`, or the older `#<dataset>` form.
function readHash() {
  const [first = "", second = ""] = location.hash.slice(1).split("/").map(decodeURIComponent);
  if (SPEEDS.includes(first)) {
    return { speed: first, dataset: second || null };
  }
  return { speed: null, dataset: first || null };
}

function writeHash() {
  const dataset = state.selected ? `/${encodeURIComponent(state.selected)}` : "";
  history.replaceState(null, "", `#${state.speed}${dataset}`);
}

/// Creates an element; `class`, `style`, and `on...` props are handled, other props become attributes.
function h(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value == null || value === false) {
      continue;
    }
    if (key === "class") {
      node.className = value;
    } else if (key === "style") {
      node.style.cssText = value;
    } else if (key.startsWith("on")) {
      node.addEventListener(key.slice(2), value);
    } else {
      node.setAttribute(key, value === true ? "" : String(value));
    }
  }
  node.append(...children.flat(Infinity).filter((child) => child != null && child !== false));
  return node;
}

function backendColor(backend) {
  if (BACKEND_COLORS[backend]) {
    return BACKEND_COLORS[backend];
  }
  let hash = 0;
  for (const char of backend) {
    hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  }
  return EXTRA_COLORS[hash % EXTRA_COLORS.length];
}

function sortBackends(backends) {
  return [...backends].sort(compareBackendNames);
}

function compareRuns(a, b) {
  return compareSpeeds(a.speed, b.speed) || compareBackendNames(a.backend, b.backend);
}

function compareSpeeds(a, b) {
  return SPEEDS.indexOf(a) - SPEEDS.indexOf(b);
}

function speedRuns(dataset, speed) {
  return dataset.runs.filter((run) => run.speed === speed);
}

function speedDatasets(speed) {
  return state.datasets.filter((dataset) => dataset.runs.some((run) => run.speed === speed));
}

function speedLabel(speed) {
  return speed === "real" ? "Realtime" : "Max speed";
}

function speedRunDirs(name) {
  return SPEEDS.map((speed) => ({ dataset: name, speed }));
}

function parseRunDir(dirName) {
  const match = RUN_DIR_NAME.exec(dirName);
  return match ? { dataset: match[1], speed: match[2] } : null;
}

function groupRunDirs(runDirs) {
  const byDataset = new Map();
  for (const { dataset, speed } of runDirs) {
    if (!byDataset.has(dataset)) {
      byDataset.set(dataset, []);
    }
    byDataset.get(dataset).push(speed);
  }
  return [...byDataset].map(([name, speeds]) => ({ name, speeds }));
}

function compareBackendNames(a, b) {
  const rank = (name) => {
    const index = KNOWN_BACKENDS.indexOf(name);
    return index === -1 ? KNOWN_BACKENDS.length : index;
  };
  return rank(a) - rank(b) || a.localeCompare(b);
}

function compareDatasets(a, b) {
  return (durationMinutes(a.name) ?? Infinity) - (durationMinutes(b.name) ?? Infinity) || a.name.localeCompare(b.name);
}

function durationMinutes(name) {
  if (!DATASET_NAME.test(name)) {
    return null;
  }
  const count = Number(name.slice(1));
  return name.startsWith("d") ? count * 1440 : count;
}

function durationLabel(name) {
  const minutes = durationMinutes(name);
  if (minutes === null) {
    return name;
  }
  if (name.startsWith("d")) {
    const days = minutes / 1440;
    return `${days} day${days === 1 ? "" : "s"}`;
  }
  return minutes % 60 === 0 ? `${minutes / 60} h` : `${minutes} min`;
}

function configKey(metrics) {
  return `${metrics.speed}|${metrics.flush_ms}|${metrics.ring_cap}`;
}

function bestValue(row, runs, values) {
  if (!row.better) {
    return null;
  }
  const candidates = values.filter(
    (value, index) => typeof value === "number" && Number.isFinite(value) && runs[index].backend !== "noop",
  );
  if (candidates.length < 2) {
    return null;
  }
  return row.better === "low" ? Math.min(...candidates) : Math.max(...candidates);
}

function speedup(metrics) {
  return metrics.wall_time_s > 0 ? (metrics.ticks_total * TICK_SECONDS) / metrics.wall_time_s : null;
}

function bytesPerTick(metrics) {
  return metrics.ticks_total > 0 ? metrics.output_bytes / metrics.ticks_total : null;
}

function formatOptional(value, format) {
  return value == null || (typeof value === "number" && !Number.isFinite(value)) ? "n/a" : format(value);
}

function formatInt(value) {
  return Math.round(value).toLocaleString("en-US");
}

function formatDecimal(value) {
  return value.toFixed(2);
}

function formatPercent(value) {
  return `${value.toFixed(1)}%`;
}

function formatCompact(value) {
  return new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 2 }).format(Math.round(value));
}

function formatRate(value) {
  return `${formatCompact(value)} ticks/s`;
}

function formatBytes(value) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let size = value;
  let unit = 0;
  while (size >= 1000 && unit < units.length - 1) {
    size /= 1000;
    unit += 1;
  }
  return unit === 0 ? `${size} B` : `${size.toFixed(size < 10 ? 2 : 1)} ${units[unit]}`;
}

function formatMs(value) {
  if (value < 1) {
    return `${value.toFixed(3)} ms`;
  }
  return value < 100 ? `${value.toFixed(2)} ms` : `${Math.round(value)} ms`;
}

function formatSeconds(seconds) {
  if (seconds < 60) {
    return `${seconds.toFixed(2)} s`;
  }
  if (seconds < 3600) {
    return `${Math.floor(seconds / 60)} min ${Math.round(seconds % 60)} s`;
  }
  return `${Math.floor(seconds / 3600)} h ${Math.round((seconds % 3600) / 60)} min`;
}

// endregion: --- Support

// region:    --- Startup

let resizeFrame = 0;
window.addEventListener("resize", () => {
  cancelAnimationFrame(resizeFrame);
  resizeFrame = requestAnimationFrame(drawCharts);
});

window.addEventListener("hashchange", () => {
  const { speed, dataset } = readHash();
  state.speed = speed ?? state.speed;
  state.selected = dataset ?? state.selected;
  reconcileSelection();
  renderView();
});

ui.refresh.addEventListener("click", loadReport);

loadReport();

// endregion: --- Startup
