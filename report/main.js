// region:    --- Constants

const CAPTURED_DIR = "sandbox/captured/";
const MOCK_DIR = "sandbox/mock-data/";
const METRICS_FILE = "capture-metrics.json";
const MANIFEST_FILE = "manifest.json";
const TICK_SECONDS = 0.005;
const DATASET_NAME = /^[dm]\d+$/;

const KNOWN_BACKENDS = ["noop", "raw", "sqlite-no-index", "sqlite-with-index"];
const FALLBACK_DATASETS = ["m001", "m010", "m030", "m060", "d001", "d010"];

const BACKEND_COLORS = {
  noop: "#8a94a6",
  raw: "#2f9e6e",
  "sqlite-no-index": "#3b7dd8",
  "sqlite-with-index": "#d9822b",
};
const EXTRA_COLORS = ["#a855f7", "#e11d48", "#0891b2", "#65a30d"];

const CHARTS = [
  { title: "Throughput", unit: "ticks/s, higher is better", get: (m) => m.ticks_per_s, format: formatRate },
  { title: "Output size", unit: "lower is better", get: (m) => m.output_bytes, format: formatBytes, skipNoop: true },
  { title: "Bytes per tick", unit: "lower is better", get: bytesPerTick, format: formatDecimal, skipNoop: true },
  { title: "Flush latency p99", unit: "ms, lower is better", get: (m) => m.flush_ms_p99, format: formatMs },
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
  { label: "Max pending ticks", get: (m) => m.max_pending_len, format: formatInt, better: "low" },
  { label: "Flush count", get: (m) => m.flush_count, format: formatInt },
  { label: "Flush p50", get: (m) => m.flush_ms_p50, format: formatMs, better: "low" },
  { label: "Flush p99", get: (m) => m.flush_ms_p99, format: formatMs, better: "low" },
  { label: "Flush max", get: (m) => m.flush_ms_max, format: formatMs, better: "low" },
  { label: "Finish", get: (m) => m.finish_ms, format: formatMs, better: "low" },
];

// endregion: --- Constants

// region:    --- State

const ui = {
  status: document.querySelector(".app-status"),
  refresh: document.querySelector(".refresh-button"),
  overview: document.querySelector(".overview"),
  nav: document.querySelector(".dataset-nav"),
  panel: document.querySelector(".dataset-panel"),
};

const state = {
  datasets: [],
  discovery: null,
  selected: readHash(),
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
    const datasets = await Promise.all(discovery.names.map(loadDataset));
    state.discovery = discovery.mode;
    // Probed names are guesses, so only datasets with real runs are kept.
    state.datasets = datasets
      .filter((dataset) => discovery.mode !== "probe" || dataset.runs.length > 0)
      .sort(compareDatasets);
    if (!state.datasets.some((dataset) => dataset.name === state.selected)) {
      state.selected = state.datasets[0]?.name ?? null;
    }
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
    return { mode: "query", names };
  }
  const listed = await listDirectories(CAPTURED_DIR);
  if (listed) {
    return { mode: "listing", names: listed.filter((name) => DATASET_NAME.test(name)) };
  }
  return { mode: "probe", names: FALLBACK_DATASETS };
}

async function loadDataset(name) {
  const datasetUrl = `${CAPTURED_DIR}${encodeURIComponent(name)}/`;
  const listed = await listDirectories(datasetUrl);
  const backends = listed ?? KNOWN_BACKENDS;
  const [manifest, runs] = await Promise.all([
    fetchJson(`${MOCK_DIR}${encodeURIComponent(name)}/${MANIFEST_FILE}`),
    Promise.all(backends.map((backend) => loadRun(datasetUrl, backend))),
  ]);
  return {
    name,
    manifest,
    listed: listed !== null,
    runs: runs.filter((run) => listed !== null || run.metrics !== null).sort(compareRuns),
  };
}

async function loadRun(datasetUrl, backend) {
  const metrics = await fetchJson(`${datasetUrl}${encodeURIComponent(backend)}/${METRICS_FILE}`);
  return { backend, metrics };
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

// endregion: --- Loading

// region:    --- Rendering

function render() {
  renderStatus();
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

function renderOverview() {
  const backends = sortBackends([
    ...new Set(state.datasets.flatMap((dataset) => dataset.runs.map((run) => run.backend))),
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
      state.datasets.map((dataset) =>
        h(
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
          backends.map((backend) =>
            renderOverviewCell(dataset.runs.find((run) => run.backend === backend)),
          ),
        ),
      ),
    ),
  );
  ui.overview.replaceChildren(
    h("h2", { class: "section-title" }, "Overview"),
    h("div", { class: "table-wrap" }, table),
  );
}

function renderOverviewCell(run) {
  if (!run) {
    return h("td", { class: "overview-cell is-absent" }, "-");
  }
  if (!run.metrics) {
    return h("td", { class: "overview-cell is-incomplete" }, "incomplete");
  }
  return h(
    "td",
    { class: "overview-cell" },
    h("span", { class: "overview-rate" }, formatRate(run.metrics.ticks_per_s)),
    h("span", { class: "overview-size" }, formatBytes(run.metrics.output_bytes)),
  );
}

function renderNav() {
  ui.nav.replaceChildren(
    ...state.datasets.map((dataset) =>
      h(
        "button",
        {
          type: "button",
          class: "dataset-tab",
          "aria-pressed": dataset.name === state.selected ? "true" : "false",
          onclick: () => selectDataset(dataset.name),
        },
        h("span", { class: "dataset-tab-name" }, dataset.name),
        h("span", { class: "dataset-tab-meta" }, `${durationLabel(dataset.name)}, ${dataset.runs.length} run(s)`),
      ),
    ),
  );
}

function renderPanel() {
  state.charts = [];
  const dataset = state.datasets.find((item) => item.name === state.selected);
  if (!dataset) {
    ui.panel.replaceChildren(renderEmptyState());
    return;
  }
  const complete = dataset.runs.filter((run) => run.metrics);
  const sections = [
    renderDatasetHeader(dataset),
    renderWarnings(dataset),
    h(
      "section",
      { class: "report-section" },
      h("h3", { class: "section-title" }, "Runs"),
      dataset.runs.length
        ? h("div", { class: "cards-grid" }, dataset.runs.map((run) => renderRunCard(run, dataset)))
        : h("p", { class: "muted" }, "No backend folders found for this dataset."),
    ),
    renderCharts(complete),
    renderComparisonTable(complete),
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
  const facts = [durationLabel(dataset.name)];
  const manifest = dataset.manifest;
  if (manifest) {
    facts.push(
      `${formatInt(manifest.tick_count)} ticks`,
      `input ${formatBytes(manifest.tick_count * manifest.bytes_per_tick)}`,
      `${manifest.rate_hz} Hz`,
      `seed ${manifest.seed}`,
    );
  }
  return h(
    "header",
    { class: "dataset-header" },
    h("h2", { class: "dataset-title" }, dataset.name),
    h("p", { class: "dataset-facts" }, facts.join(", ")),
  );
}

function renderWarnings(dataset) {
  const messages = [];
  const complete = dataset.runs.filter((run) => run.metrics);
  if (!dataset.manifest) {
    messages.push(`Manifest not found at ${MOCK_DIR}${dataset.name}/${MANIFEST_FILE}; tick conservation is not checked.`);
  }
  const configs = new Set(complete.map((run) => configKey(run.metrics)));
  if (configs.size > 1) {
    messages.push("Runs use different speed, flush interval, or ring capacity; comparisons are not like-for-like.");
  }
  const incomplete = dataset.runs.length - complete.length;
  if (incomplete > 0) {
    messages.push(`${incomplete} run(s) without ${METRICS_FILE}: incomplete, failed, or still running.`);
  }
  if (!dataset.listed) {
    messages.push("Directory listing is unavailable, so backends were probed by known names only.");
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
    return card;
  }
  const stats = [
    ["Wall time", formatSeconds(metrics.wall_time_s)],
    ["Speedup", formatOptional(speedup(metrics), (v) => `${v.toFixed(1)}x realtime`)],
    ["Output", formatBytes(metrics.output_bytes)],
    ["Bytes/tick", formatOptional(bytesPerTick(metrics), formatDecimal)],
    ["Flush p50 / p99", `${formatMs(metrics.flush_ms_p50)} / ${formatMs(metrics.flush_ms_p99)}`],
    ["Flush max", formatMs(metrics.flush_ms_max)],
    ["Stalls", formatInt(metrics.stall_count)],
    ["Finish", formatMs(metrics.finish_ms)],
    ["Config", `flush ${metrics.flush_ms} ms, ring ${formatInt(metrics.ring_cap)}`],
  ];
  card.append(
    h(
      "div",
      { class: "run-card-hero" },
      h("span", { class: "run-card-hero-value" }, formatCompact(metrics.ticks_per_s)),
      h("span", { class: "run-card-hero-label" }, "ticks/s"),
    ),
    h("dl", { class: "run-card-stats" }, stats.map(([label, value]) => [h("dt", {}, label), h("dd", {}, value)])),
    renderConservation(metrics, dataset.manifest),
  );
  return card;
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
    { class: "run-card-check is-warn" },
    `Tick mismatch: ${formatInt(metrics.ticks_total)} of ${formatInt(manifest.tick_count)}`,
  );
}

function renderCharts(runs) {
  if (!runs.length) {
    return null;
  }
  const figures = CHARTS.map((chart) => {
    const bars = runs
      .filter((run) => !(chart.skipNoop && run.backend === "noop"))
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
        h("span", { class: "chart-title" }, chart.title),
        h("span", { class: "chart-unit" }, chart.unit),
      ),
      canvas,
    );
  });
  return h(
    "section",
    { class: "report-section" },
    h("h3", { class: "section-title" }, "Charts"),
    h("div", { class: "charts-grid" }, figures),
  );
}

function renderComparisonTable(runs) {
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
      return h(
        "tr",
        {},
        h("th", { scope: "row" }, row.label),
        values.map((value, index) =>
          h(
            "td",
            { class: best !== null && value === best && runs[index].backend !== "noop" ? "is-best" : null },
            formatOptional(value, row.format),
          ),
        ),
      );
    }),
  );
  return h(
    "section",
    { class: "report-section" },
    h("h3", { class: "section-title" }, "Comparison"),
    h("div", { class: "table-wrap" }, h("table", { class: "comparison-table" }, head, body)),
    h("p", { class: "muted small" }, "Highlighted cells are the best value among recording backends; noop is excluded."),
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

function selectDataset(name) {
  state.selected = name;
  history.replaceState(null, "", `#${encodeURIComponent(name)}`);
  renderOverview();
  renderNav();
  renderPanel();
}

function readHash() {
  const hash = location.hash.slice(1);
  return hash ? decodeURIComponent(hash) : null;
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
  return compareBackendNames(a.backend, b.backend);
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

function formatCompact(value) {
  return new Intl.NumberFormat("en-US", { notation: "compact", maximumFractionDigits: 2 }).format(value);
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
  const name = readHash();
  if (name && name !== state.selected && state.datasets.some((dataset) => dataset.name === name)) {
    selectDataset(name);
  }
});

ui.refresh.addEventListener("click", loadReport);

loadReport();

// endregion: --- Startup
