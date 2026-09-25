// General-purpose GIS view: import, layers, map, attribute table, natural-
// language analysis, and provenance. Talks to /api/gis/* on the workbench.

const $ = (id) => document.getElementById(id);
const SVG_NS = "http://www.w3.org/2000/svg";
const BASE_ZOOM = 10;
const TILE = 256;
const PALETTE = ["#e4572e", "#2e86ab", "#f3a712", "#29bf12", "#a846a0", "#00a6a6", "#d7263d", "#6c757d"];
const OP_TITLES = {};

const state = {
  layers: [], // StoredSummary list from the server
  geo: new Map(), // id -> {geojson, origin, paths}
  visible: new Map(), // id -> bool
  activeTable: null,
  table: { offset: 0, limit: 100, where: "", sortBy: null, desc: false, total: 0 },
  view: { lon: 136.9066, lat: 35.1815, zoom: 11 },
  clickPoint: null,
  highlight: null, // {layerId, featureId}
  fitted: false,
  pendingImport: null, // {file, bytes}
  llmReady: false,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function esc(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (key === "class") node.className = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else if (value !== undefined && value !== null && value !== false) node.setAttribute(key, value);
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined) continue;
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

function formatValue(value) {
  if (value === null || value === undefined) return "—";
  if (typeof value === "number") {
    // Group digits only from 10,000 so years and codes (2020, 23101) stay readable.
    if (Number.isInteger(value)) return Math.abs(value) >= 10000 ? value.toLocaleString("ja-JP") : String(value);
    return value.toLocaleString("ja-JP", { maximumFractionDigits: Math.abs(value) < 10 ? 3 : 1 });
  }
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

function short(digest) {
  if (!digest) return "";
  const hex = String(digest).replace("sha256:", "");
  return `sha256:${hex.slice(0, 12)}…`;
}

function setStatus(text, kind = "") {
  const node = $("gis-status");
  node.textContent = text;
  node.className = `status ${kind}`;
}

async function api(path, options = {}) {
  const response = await fetch(path, options);
  const contentType = response.headers.get("content-type") || "";
  if (!contentType.includes("json")) {
    if (!response.ok) throw new Error(`${response.status} ${response.statusText}`);
    return response;
  }
  const payload = await response.json();
  if (!payload.ok) {
    const error = new Error(payload.error || `HTTP ${response.status}`);
    error.payload = payload;
    throw error;
  }
  return payload.result;
}

const postJson = (path, body) =>
  api(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });

function layerColor(summary, index) {
  return summary.style?.color || PALETTE[index % PALETTE.length];
}

function layerById(id) {
  return state.layers.find((layer) => layer.id === id);
}

function kindLabel(kind) {
  return { point: "点", line: "線", polygon: "面", mixed: "混在", none: "表のみ" }[kind] || kind;
}

// ---------------------------------------------------------------------------
// View tabs
// ---------------------------------------------------------------------------

function showView(viewId) {
  for (const tab of document.querySelectorAll(".view-tab")) {
    const active = tab.dataset.view === viewId;
    tab.classList.toggle("active", active);
    const view = $(tab.dataset.view);
    if (view) view.hidden = !active;
  }
  try {
    localStorage.setItem("genegis.view", viewId);
  } catch {
    /* storage unavailable */
  }
  if (viewId === "gis-view") {
    if (!state.fitted && state.layers.length) {
      requestAnimationFrame(fitAll);
    }
    render();
  }
}

for (const tab of document.querySelectorAll(".view-tab")) {
  tab.addEventListener("click", () => showView(tab.dataset.view));
}

// ---------------------------------------------------------------------------
// Web Mercator
// ---------------------------------------------------------------------------

function worldXY(lon, lat, zoom) {
  const scale = TILE * 2 ** zoom;
  const clamped = Math.max(-85.05112878, Math.min(85.05112878, lat));
  const sin = Math.sin((clamped * Math.PI) / 180);
  return [((lon + 180) / 360) * scale, (0.5 - Math.log((1 + sin) / (1 - sin)) / (4 * Math.PI)) * scale];
}

function lonLat(x, y, zoom) {
  const scale = TILE * 2 ** zoom;
  const lon = (x / scale) * 360 - 180;
  const n = Math.PI - (2 * Math.PI * y) / scale;
  return [lon, (180 / Math.PI) * Math.atan(0.5 * (Math.exp(n) - Math.exp(-n)))];
}

function mapSize() {
  const rect = $("gis-map").getBoundingClientRect();
  return [Math.max(rect.width, 1), Math.max(rect.height, 1)];
}

function screenToLonLat(sx, sy) {
  const [w, h] = mapSize();
  const [cx, cy] = worldXY(state.view.lon, state.view.lat, state.view.zoom);
  return lonLat(cx + sx - w / 2, cy + sy - h / 2, state.view.zoom);
}

function metresPerPixel() {
  return (156543.03392 * Math.cos((state.view.lat * Math.PI) / 180)) / 2 ** state.view.zoom;
}

// ---------------------------------------------------------------------------
// Geometry → SVG paths in layer-local base-zoom coordinates
// ---------------------------------------------------------------------------

function ringPath(ring, origin) {
  let d = "";
  ring.forEach(([lon, lat], i) => {
    const [x, y] = worldXY(lon, lat, BASE_ZOOM);
    d += `${i ? "L" : "M"}${(x - origin[0]).toFixed(3)} ${(y - origin[1]).toFixed(3)}`;
  });
  return d;
}

function geometryParts(geometry, origin) {
  if (!geometry) return { d: "", points: [] };
  const { type, coordinates } = geometry;
  switch (type) {
    case "Polygon":
      return { d: coordinates.map((r) => ringPath(r, origin) + "Z").join(""), points: [], area: true };
    case "MultiPolygon":
      return { d: coordinates.flatMap((p) => p.map((r) => ringPath(r, origin) + "Z")).join(""), points: [], area: true };
    case "LineString":
      return { d: ringPath(coordinates, origin), points: [] };
    case "MultiLineString":
      return { d: coordinates.map((l) => ringPath(l, origin)).join(""), points: [] };
    case "Point":
      return { d: "", points: [coordinates] };
    case "MultiPoint":
      return { d: "", points: coordinates };
    case "GeometryCollection": {
      const parts = geometry.geometries.map((g) => geometryParts(g, origin));
      return { d: parts.map((p) => p.d).join(""), points: parts.flatMap((p) => p.points), area: parts.some((p) => p.area) };
    }
    default:
      return { d: "", points: [] };
  }
}

async function loadGeometry(id) {
  if (state.geo.has(id)) return state.geo.get(id);
  const response = await fetch(`/api/gis/layers/${encodeURIComponent(id)}/geojson`);
  if (!response.ok) throw new Error(`geometry ${id}: HTTP ${response.status}`);
  const geojson = await response.json();
  const summary = layerById(id);
  const bbox = summary?.bbox_wgs84;
  const origin = bbox ? worldXY(bbox[0], bbox[3], BASE_ZOOM) : [0, 0];
  const features = geojson.features.map((feature) => ({
    id: feature.properties.__id,
    properties: feature.properties,
    ...geometryParts(feature.geometry, origin),
  }));
  const entry = { origin, features };
  state.geo.set(id, entry);
  return entry;
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

let renderQueued = false;

function render() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(() => {
    renderQueued = false;
    drawMap();
  });
}

function drawTiles(group, w, h, cx, cy) {
  if (!$("gis-basemap").checked) return;
  const z = Math.max(2, Math.min(18, Math.round(state.view.zoom)));
  const scale = 2 ** (state.view.zoom - z);
  const size = TILE * scale;
  const [wx, wy] = [cx / scale, cy / scale];
  const minX = Math.floor((wx - w / 2 / scale) / TILE);
  const maxX = Math.floor((wx + w / 2 / scale) / TILE);
  const minY = Math.max(0, Math.floor((wy - h / 2 / scale) / TILE));
  const maxY = Math.min(2 ** z - 1, Math.floor((wy + h / 2 / scale) / TILE));
  for (let tx = minX; tx <= maxX; tx++) {
    for (let ty = minY; ty <= maxY; ty++) {
      const wrapped = ((tx % 2 ** z) + 2 ** z) % 2 ** z;
      const image = document.createElementNS(SVG_NS, "image");
      image.setAttribute("href", `https://cyberjapandata.gsi.go.jp/xyz/pale/${z}/${wrapped}/${ty}.png`);
      image.setAttribute("x", (tx * TILE * scale - cx + w / 2).toFixed(1));
      image.setAttribute("y", (ty * TILE * scale - cy + h / 2).toFixed(1));
      image.setAttribute("width", (size + 0.5).toFixed(1));
      image.setAttribute("height", (size + 0.5).toFixed(1));
      image.setAttribute("preserveAspectRatio", "none");
      group.append(image);
    }
  }
}

function classColor(summary, featureId) {
  const classification = summary.style?.classification;
  if (!classification) return null;
  const index = classification.assignments?.[featureId];
  if (index === null || index === undefined) return "#bbbbbb";
  return classification.classes[index]?.color || "#bbbbbb";
}

function drawMap() {
  const svg = $("gis-svg");
  if (!svg || $("gis-view").hidden) return;
  const [w, h] = mapSize();
  svg.setAttribute("viewBox", `0 0 ${w} ${h}`);
  svg.replaceChildren();
  const [cx, cy] = worldXY(state.view.lon, state.view.lat, state.view.zoom);
  const tiles = document.createElementNS(SVG_NS, "g");
  drawTiles(tiles, w, h, cx, cy);
  svg.append(tiles);

  const zoomScale = 2 ** (state.view.zoom - BASE_ZOOM);
  state.layers.forEach((summary, index) => {
    if (!state.visible.get(summary.id)) return;
    const entry = state.geo.get(summary.id);
    if (!entry) {
      loadGeometry(summary.id).then(render).catch((error) => setStatus(error.message, "gis-error"));
      return;
    }
    const color = layerColor(summary, index);
    const [ox, oy] = [entry.origin[0] * zoomScale - cx + w / 2, entry.origin[1] * zoomScale - cy + h / 2];
    const group = document.createElementNS(SVG_NS, "g");
    group.setAttribute("transform", `translate(${ox.toFixed(2)} ${oy.toFixed(2)}) scale(${zoomScale})`);
    const points = document.createElementNS(SVG_NS, "g");
    for (const feature of entry.features) {
      const fill = classColor(summary, feature.id) || color;
      const highlighted = state.highlight?.layerId === summary.id && state.highlight?.featureId === feature.id;
      if (feature.d) {
        const path = document.createElementNS(SVG_NS, "path");
        path.setAttribute("d", feature.d);
        path.setAttribute("vector-effect", "non-scaling-stroke");
        path.setAttribute("fill-rule", "evenodd");
        if (feature.area) {
          path.setAttribute("fill", fill);
          path.setAttribute("fill-opacity", summary.style?.classification ? "0.78" : "0.35");
          path.setAttribute("stroke", highlighted ? "#ffeb3b" : summary.style?.classification ? "#333" : color);
          path.setAttribute("stroke-width", highlighted ? "3" : "1");
        } else {
          path.setAttribute("fill", "none");
          path.setAttribute("stroke", highlighted ? "#ffeb3b" : fill);
          path.setAttribute("stroke-width", highlighted ? "4" : "2");
        }
        group.append(path);
      }
      for (const [lon, lat] of feature.points) {
        const [x, y] = worldXY(lon, lat, state.view.zoom);
        const circle = document.createElementNS(SVG_NS, "circle");
        circle.setAttribute("cx", (x - cx + w / 2).toFixed(1));
        circle.setAttribute("cy", (y - cy + h / 2).toFixed(1));
        circle.setAttribute("r", highlighted ? "7" : "4.5");
        circle.setAttribute("fill", fill);
        circle.setAttribute("stroke", highlighted ? "#ffeb3b" : "#fff");
        circle.setAttribute("stroke-width", highlighted ? "2.5" : "1.2");
        points.append(circle);
      }
    }
    svg.append(group, points);
  });

  if (state.clickPoint) {
    const [x, y] = worldXY(state.clickPoint[0], state.clickPoint[1], state.view.zoom);
    const marker = document.createElementNS(SVG_NS, "g");
    marker.setAttribute("transform", `translate(${(x - cx + w / 2).toFixed(1)} ${(y - cy + h / 2).toFixed(1)})`);
    marker.innerHTML = '<circle r="9" fill="none" stroke="#111" stroke-width="3"/><circle r="9" fill="none" stroke="#fff" stroke-width="1.5"/><circle r="2.5" fill="#111"/>';
    svg.append(marker);
  }

  drawLegend();
  drawAttribution();
}

function drawLegend() {
  const legend = $("gis-legend");
  const classified = state.layers.filter((l) => state.visible.get(l.id) && l.style?.classification);
  if (!classified.length) {
    legend.hidden = true;
    return;
  }
  legend.hidden = false;
  legend.replaceChildren(
    ...classified.map((layer) => {
      const c = layer.style.classification;
      return el(
        "div",
        {},
        el("strong", {}, `${layer.name}`),
        el("div", { class: "status" }, `${c.field}${c.unit ? ` (${c.unit})` : ""} · ${methodLabel(c.method)}`),
        ...c.classes.map((cls) =>
          el("div", { class: "gis-legend-row" }, el("span", { class: "gis-swatch", style: `background:${cls.color}` }), `${cls.label} (${cls.count})`),
        ),
      );
    }),
  );
}

function drawAttribution() {
  const sources = new Set();
  for (const layer of state.layers) {
    if (state.visible.get(layer.id) && layer.provenance?.attribution) sources.add(layer.provenance.attribution);
  }
  const node = $("gis-attribution");
  node.replaceChildren();
  if ($("gis-basemap").checked) {
    node.append(el("a", { href: "https://maps.gsi.go.jp/development/ichiran.html", target: "_blank", rel: "noopener" }, "地理院タイル"));
  }
  if (sources.size) node.append(` | ${[...sources].join(" | ")}`);
}

function methodLabel(method) {
  return { equal_interval: "等間隔", quantile: "等量", natural_breaks: "自然分類 (Jenks)", categorical: "カテゴリ" }[method] || method;
}

// ---------------------------------------------------------------------------
// Map interaction
// ---------------------------------------------------------------------------

function zoomAt(sx, sy, delta) {
  const [w, h] = mapSize();
  const before = screenToLonLat(sx, sy);
  state.view.zoom = Math.max(2, Math.min(19, state.view.zoom + delta));
  const [bx, by] = worldXY(before[0], before[1], state.view.zoom);
  [state.view.lon, state.view.lat] = lonLat(bx - (sx - w / 2), by - (sy - h / 2), state.view.zoom);
  render();
}

function fitBounds(bbox) {
  if (!bbox) return;
  const [w, h] = mapSize();
  const [minLon, minLat, maxLon, maxLat] = bbox;
  let zoom = 18;
  for (; zoom > 2; zoom -= 0.25) {
    const [x0, y0] = worldXY(minLon, maxLat, zoom);
    const [x1, y1] = worldXY(maxLon, minLat, zoom);
    if (x1 - x0 < w * 0.9 && y1 - y0 < h * 0.9) break;
  }
  state.view = { lon: (minLon + maxLon) / 2, lat: (minLat + maxLat) / 2, zoom };
  render();
}

function fitAll() {
  if ($("gis-view").hidden) return;
  state.fitted = true;
  const boxes = state.layers.filter((l) => state.visible.get(l.id) && l.bbox_wgs84).map((l) => l.bbox_wgs84);
  if (!boxes.length) return;
  fitBounds([
    Math.min(...boxes.map((b) => b[0])),
    Math.min(...boxes.map((b) => b[1])),
    Math.max(...boxes.map((b) => b[2])),
    Math.max(...boxes.map((b) => b[3])),
  ]);
}

function setupMapInteraction() {
  const svg = $("gis-svg");
  const map = $("gis-map");
  let drag = null;
  svg.addEventListener("pointerdown", (event) => {
    drag = { x: event.clientX, y: event.clientY, moved: false, start: { ...state.view } };
    svg.setPointerCapture(event.pointerId);
  });
  svg.addEventListener("pointermove", (event) => {
    const rect = svg.getBoundingClientRect();
    const [lon, lat] = screenToLonLat(event.clientX - rect.left, event.clientY - rect.top);
    $("gis-coords").textContent = `${lon.toFixed(5)}, ${lat.toFixed(5)} (EPSG:4326)`;
    if (!drag) return;
    const dx = event.clientX - drag.x;
    const dy = event.clientY - drag.y;
    if (Math.abs(dx) + Math.abs(dy) > 3) drag.moved = true;
    if (!drag.moved) return;
    svg.classList.add("dragging");
    $("gis-popup").hidden = true;
    const [cx, cy] = worldXY(drag.start.lon, drag.start.lat, drag.start.zoom);
    [state.view.lon, state.view.lat] = lonLat(cx - dx, cy - dy, drag.start.zoom);
    render();
  });
  svg.addEventListener("pointerup", (event) => {
    svg.classList.remove("dragging");
    const wasClick = drag && !drag.moved;
    drag = null;
    if (wasClick) {
      const rect = svg.getBoundingClientRect();
      pickAt(event.clientX - rect.left, event.clientY - rect.top);
    }
  });
  svg.addEventListener(
    "wheel",
    (event) => {
      event.preventDefault();
      const rect = svg.getBoundingClientRect();
      zoomAt(event.clientX - rect.left, event.clientY - rect.top, event.deltaY < 0 ? 0.5 : -0.5);
    },
    { passive: false },
  );
  map.addEventListener("keydown", (event) => {
    const [w, h] = mapSize();
    if (event.key === "+") zoomAt(w / 2, h / 2, 1);
    if (event.key === "-") zoomAt(w / 2, h / 2, -1);
  });
  $("gis-zoom-in").addEventListener("click", () => {
    const [w, h] = mapSize();
    zoomAt(w / 2, h / 2, 1);
  });
  $("gis-zoom-out").addEventListener("click", () => {
    const [w, h] = mapSize();
    zoomAt(w / 2, h / 2, -1);
  });
  $("gis-zoom-all").addEventListener("click", fitAll);
  $("gis-basemap").addEventListener("change", render);
  new ResizeObserver(render).observe(map);
}

async function pickAt(sx, sy) {
  const [lon, lat] = screenToLonLat(sx, sy);
  state.clickPoint = [lon, lat];
  render();
  const popup = $("gis-popup");
  const visible = state.layers.filter((l) => state.visible.get(l.id)).map((l) => l.id);
  const header = el(
    "div",
    {},
    el("strong", {}, "選択地点"),
    ` ${lon.toFixed(5)}, ${lat.toFixed(5)}`,
    el("div", { class: "status" }, "「この点から…」の質問で使えます"),
  );
  popup.replaceChildren(header);
  placePopup(popup, sx, sy);
  if (!visible.length) return;
  try {
    const result = await postJson("/api/gis/pick", { lon, lat, tolerance_m: Math.max(5, metresPerPixel() * 8), layers: visible });
    for (const hit of result.hits.slice(0, 6)) {
      const layer = layerById(hit.layer_id);
      const keys = layer ? orderedFields(layer).map((f) => f.name) : Object.keys(hit.feature.properties);
      const rows = keys
        .filter((key) => key !== "__id" && key in hit.feature.properties)
        .slice(0, 12)
        .map((key) => [key, hit.feature.properties[key]])
        .map(([key, value]) => el("tr", {}, el("td", {}, key), el("td", {}, `${formatValue(value)}${hit.units?.[key] ? ` ${hit.units[key]}` : ""}`)));
      popup.append(el("div", {}, el("strong", {}, hit.layer_name), hit.feature.distance_m > 0 ? el("span", { class: "status" }, ` (${hit.feature.distance_m.toFixed(0)} m)`) : null, el("table", {}, rows)));
    }
    if (!result.hits.length) popup.append(el("div", { class: "status" }, "この位置に地物はありません"));
  } catch (error) {
    popup.append(el("div", { class: "gis-error" }, error.message));
  }
}

function placePopup(popup, sx, sy) {
  const [w, h] = mapSize();
  popup.hidden = false;
  popup.style.left = `${Math.min(sx + 12, w - 330)}px`;
  popup.style.top = `${Math.min(sy + 12, h - 120)}px`;
}

// ---------------------------------------------------------------------------
// Layers
// ---------------------------------------------------------------------------

async function refreshLayers({ fitNew = false } = {}) {
  const previous = new Set(state.layers.map((l) => l.id));
  const result = await api("/api/gis/layers");
  state.layers = result.layers;
  state.llmReady = result.llm_ready;
  const added = [];
  for (const layer of state.layers) {
    if (!state.visible.has(layer.id)) state.visible.set(layer.id, layer.receipt?.kind !== "intermediate");
    if (!previous.has(layer.id)) added.push(layer);
  }
  for (const id of [...state.geo.keys()]) {
    if (!layerById(id)) state.geo.delete(id);
  }
  renderLayerList();
  if (fitNew && added.length) {
    const boxes = added.filter((l) => l.bbox_wgs84).map((l) => l.bbox_wgs84);
    if (boxes.length) {
      fitBounds([
        Math.min(...boxes.map((b) => b[0])),
        Math.min(...boxes.map((b) => b[1])),
        Math.max(...boxes.map((b) => b[2])),
        Math.max(...boxes.map((b) => b[3])),
      ]);
    }
  }
  render();
  return added;
}

function renderLayerList() {
  const container = $("gis-layers");
  if (!state.layers.length) {
    container.replaceChildren(el("p", { class: "gis-empty" }, "まだレイヤがありません。"));
    return;
  }
  container.replaceChildren(
    ...[...state.layers].reverse().map((layer) => {
      const index = state.layers.indexOf(layer);
      const color = layerColor(layer, index);
      const numericFields = layer.fields.filter((f) => f.type === "integer" || f.type === "float");
      const card = el("div", { class: `gis-layer${state.activeTable === layer.id ? " active" : ""}` });
      const visible = el("input", {
        type: "checkbox",
        "aria-label": `${layer.name} を表示`,
        onchange: (event) => {
          state.visible.set(layer.id, event.target.checked);
          render();
        },
      });
      visible.checked = !!state.visible.get(layer.id);
      const name = el("span", { class: "gis-layer-name", title: "クリックで来歴を表示" }, layer.name);
      name.addEventListener("click", () => showLayerProvenance(layer));
      const crsBadge = layer.crs_needs_confirmation
        ? el("span", { class: "gis-badge warn", title: "座標範囲から推定した CRS です" }, `CRS推定 ${layer.crs}`)
        : el("span", { class: "gis-badge" }, layer.crs);
      const meta = el(
        "div",
        { class: "gis-layer-meta" },
        `${kindLabel(layer.geometry_kind)} · ${layer.feature_count.toLocaleString("ja-JP")} 件 · `,
        crsBadge,
        layer.receipt?.kind === "analysis" ? el("span", { class: "gis-badge ok" }, " 検証済み分析") : null,
        layer.invalid_feature_ids?.length
          ? el("span", { class: "gis-badge warn", title: "自己交差などの不正なポリゴンです。分析では make_valid で修復されます" }, ` 不正な形状 ${layer.invalid_feature_ids.length}件`)
          : null,
      );
      const panel = el("div", { class: "gis-layer-panel", hidden: true });
      const toggle = (builder) => () => {
        if (!panel.hidden && panel.dataset.kind === builder.name) {
          panel.hidden = true;
          return;
        }
        panel.dataset.kind = builder.name;
        panel.replaceChildren(...builder(layer, numericFields));
        panel.hidden = false;
      };
      const actions = el(
        "div",
        { class: "gis-layer-actions" },
        el("button", { type: "button", class: "secondary", onclick: () => openTable(layer.id) }, "表"),
        el("button", { type: "button", class: "secondary", onclick: () => fitBounds(layer.bbox_wgs84) }, "ズーム"),
        el("button", { type: "button", class: "secondary", onclick: toggle(classifyPanel) }, "色分け"),
        el("button", { type: "button", class: "secondary", onclick: toggle(exportPanel) }, "書き出し"),
        layer.crs_needs_confirmation ? el("button", { type: "button", class: "secondary", onclick: toggle(crsPanel) }, "CRS確認") : null,
        el("button", { type: "button", class: "secondary", onclick: () => removeLayer(layer) }, "削除"),
      );
      card.append(el("div", { class: "gis-layer-head" }, visible, el("span", { class: "gis-swatch", style: `background:${color}` }), name), meta, actions, panel);
      return card;
    }),
  );
}

function classifyPanel(layer) {
  const fieldSelect = el("select", { "aria-label": "色分けする列" }, ...layer.fields.map((f) => el("option", { value: f.name }, `${f.name}${f.unit ? ` (${f.unit})` : ""}`)));
  const current = layer.style?.classification;
  if (current) fieldSelect.value = current.field;
  const method = el(
    "select",
    { "aria-label": "分類方法" },
    el("option", { value: "natural_breaks" }, "自然分類 (Jenks)"),
    el("option", { value: "quantile" }, "等量"),
    el("option", { value: "equal_interval" }, "等間隔"),
    el("option", { value: "categorical" }, "カテゴリ"),
  );
  if (current) method.value = current.method;
  const classes = el("input", { type: "number", min: "2", max: "9", value: current?.classes?.length && current.method !== "categorical" ? current.classes.length : "5", "aria-label": "階級数" });
  const apply = el("button", {
    type: "button",
    onclick: async () => {
      try {
        await postJson(`/api/gis/layers/${layer.id}/classify`, { field: fieldSelect.value, method: method.value, classes: Number(classes.value) });
        await refreshLayers();
      } catch (error) {
        setStatus(error.message, "gis-error");
      }
    },
  }, "適用");
  const clear = el("button", {
    type: "button",
    class: "secondary",
    onclick: async () => {
      const style = { ...(layer.style || {}) };
      delete style.classification;
      await postJson(`/api/gis/layers/${layer.id}/style`, style);
      await refreshLayers();
    },
  }, "解除");
  return [el("div", { class: "gis-row" }, fieldSelect), el("div", { class: "gis-row" }, method, classes), el("div", { class: "gis-row" }, apply, clear)];
}

function exportPanel(layer) {
  const base = `/api/gis/layers/${encodeURIComponent(layer.id)}/export`;
  const title = encodeURIComponent(layer.name);
  return [
    el(
      "div",
      {},
      el("a", { href: `${base}?format=geojson`, download: "" }, "GeoJSON"),
      el("a", { href: `${base}?format=csv`, download: "" }, "CSV"),
      el("a", { href: `${base}?format=geopackage`, download: "" }, "GeoPackage"),
      el("a", { href: `${base}?format=geoparquet`, download: "" }, "GeoParquet"),
      el("a", { href: `${base}?format=pdf&title=${title}`, target: "_blank", rel: "noopener" }, "PDF 地図"),
    ),
    el("div", { class: "status" }, "PDF には凡例・縮尺・方位・CRS・出典・ダイジェストが入ります。CSV は型情報を持たないため、型を保つには GeoPackage / GeoParquet を使ってください。"),
  ];
}

function crsPanel(layer) {
  const select = crsSelect(layer.crs);
  return [
    el("div", { class: "status" }, `座標範囲から ${layer.crs} と推定しました。正しい CRS を確定してください（座標値は変わりません）。`),
    el("div", { class: "gis-row" }, select),
    el("div", { class: "gis-row" }, el("button", {
      type: "button",
      onclick: async () => {
        try {
          setStatus("CRS を指定しています…");
          const result = await postJson(`/api/gis/layers/${layer.id}/crs`, { crs: select.value });
          state.geo.delete(layer.id);
          await refreshLayers();
          showRun(result, null);
          setStatus("CRS を確定しました", "");
        } catch (error) {
          setStatus(error.message, "gis-error");
        }
      },
    }, "確定")),
  ];
}

let crsOptionsCache = null;

function crsSelect(selected) {
  const options = crsOptionsCache || [
    { id: "EPSG:4326", name: "WGS 84（経緯度）" },
    { id: "EPSG:6668", name: "JGD2011（経緯度）" },
    ...Array.from({ length: 19 }, (_, i) => ({ id: `EPSG:${6669 + i}`, name: `JGD2011 平面直角座標系 ${["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X", "XI", "XII", "XIII", "XIV", "XV", "XVI", "XVII", "XVIII", "XIX"][i]} 系` })),
    { id: "EPSG:3857", name: "Web メルカトル" },
  ];
  const select = el("select", { "aria-label": "座標参照系" }, ...options.map((o) => el("option", { value: o.id }, `${o.id} — ${o.name}`)));
  if (selected) select.value = selected;
  return select;
}

async function removeLayer(layer) {
  try {
    await api(`/api/gis/layers/${encodeURIComponent(layer.id)}`, { method: "DELETE" });
    state.visible.delete(layer.id);
    if (state.activeTable === layer.id) {
      state.activeTable = null;
      $("gis-table").replaceChildren();
    }
    await refreshLayers();
  } catch (error) {
    setStatus(error.message, "gis-error");
  }
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

async function importFile(file, bytes, crs = null) {
  const params = new URLSearchParams({ filename: file.name });
  if (crs) params.set("crs", crs);
  setStatus(`${file.name} を読み込み中…`);
  try {
    const result = await api(`/api/gis/import?${params}`, { method: "POST", body: bytes, headers: { "content-type": "application/octet-stream" } });
    $("gis-crs-prompt").hidden = true;
    state.pendingImport = null;
    await refreshLayers({ fitNew: true });
    showImport(result);
    setStatus(`${file.name} を読み込みました`);
  } catch (error) {
    if (error.payload?.needs_crs) {
      crsOptionsCache = error.payload.crs_options;
      state.pendingImport = { file, bytes };
      $("gis-crs-message").textContent = `${file.name}: ${error.message}`;
      const select = crsSelect("EPSG:6675");
      select.id = "gis-crs-select";
      $("gis-crs-select").replaceWith(select);
      $("gis-crs-prompt").hidden = false;
      setStatus("CRS を選んでください", "gis-error");
    } else {
      setStatus(`${file.name}: ${error.message}`, "gis-error");
    }
  }
}

async function importFiles(files) {
  for (const file of files) {
    const bytes = await file.arrayBuffer();
    await importFile(file, bytes);
  }
}

function setupImport() {
  const drop = $("gis-drop");
  $("gis-file").addEventListener("change", (event) => {
    importFiles([...event.target.files]);
    event.target.value = "";
  });
  for (const type of ["dragenter", "dragover"]) {
    drop.addEventListener(type, (event) => {
      event.preventDefault();
      drop.classList.add("dragging");
    });
  }
  for (const type of ["dragleave", "drop"]) {
    drop.addEventListener(type, (event) => {
      event.preventDefault();
      drop.classList.remove("dragging");
    });
  }
  drop.addEventListener("drop", (event) => importFiles([...event.dataTransfer.files]));
  $("gis-crs-retry").addEventListener("click", () => {
    if (state.pendingImport) importFile(state.pendingImport.file, state.pendingImport.bytes, $("gis-crs-select").value);
  });
  $("gis-crs-cancel").addEventListener("click", () => {
    state.pendingImport = null;
    $("gis-crs-prompt").hidden = true;
    setStatus("Ready");
  });
  $("gis-samples-btn").addEventListener("click", async () => {
    setStatus("サンプルを読み込み中…");
    try {
      await postJson("/api/gis/samples", {});
      await refreshLayers({ fitNew: true });
      setStatus("サンプルを読み込みました。例: 「駅から500m以内の避難所を数えて」");
    } catch (error) {
      setStatus(error.message, "gis-error");
    }
  });
  $("gis-place-btn").addEventListener("click", async () => {
    const query = $("gis-place-query").value.trim();
    if (!query) return;
    setStatus(`「${query}」を検索中…`);
    try {
      const result = await postJson("/api/gis/place", { query, provider: $("gis-place-provider").value });
      await refreshLayers({ fitNew: true });
      showPlace(result);
      setStatus(`「${query}」を追加しました`);
    } catch (error) {
      setStatus(error.message, "gis-error");
    }
  });
}

// ---------------------------------------------------------------------------
// Attribute table
// ---------------------------------------------------------------------------

function openTable(id) {
  state.activeTable = id;
  state.table = { offset: 0, limit: 100, where: "", sortBy: null, desc: false, total: 0 };
  $("gis-where").value = "";
  renderLayerList();
  loadTable();
}

async function loadTable() {
  const id = state.activeTable;
  const layer = layerById(id);
  if (!layer) return;
  $("gis-table-title").textContent = layer.name;
  const params = new URLSearchParams({ offset: state.table.offset, limit: state.table.limit });
  if (state.table.where) params.set("where", state.table.where);
  if (state.table.sortBy) {
    params.set("sort_by", state.table.sortBy);
    params.set("descending", state.table.desc);
  }
  try {
    const page = await api(`/api/gis/layers/${encodeURIComponent(id)}/table?${params}`);
    state.table.total = page.total_matched;
    renderTable(layer, page.rows);
    const end = Math.min(state.table.offset + state.table.limit, page.total_matched);
    $("gis-table-count").textContent = `${page.total_matched.toLocaleString("ja-JP")} / ${page.total_rows.toLocaleString("ja-JP")} 件（${page.total_matched ? state.table.offset + 1 : 0}–${end}）`;
  } catch (error) {
    $("gis-table-count").textContent = error.message;
  }
}

/** Measured/derived columns (with units) and names first, source metadata last. */
function orderedFields(layer) {
  const rank = (field) => {
    if (field.unit) return 0;
    if (/(^|_)(name|名称|名前)$|^name|名$/i.test(field.name)) return 1;
    if (/(url|source|license|retrieval|basis|boundary_)/i.test(field.name)) return 3;
    return 2;
  };
  return [...layer.fields].sort((a, b) => rank(a) - rank(b));
}

function renderTable(layer, rows) {
  const fields = orderedFields(layer);
  const head = el(
    "tr",
    {},
    el("th", {}, "#"),
    ...fields.map((field) => {
      const arrow = state.table.sortBy === field.name ? (state.table.desc ? " ▼" : " ▲") : "";
      return el("th", {
        title: `${field.type}${field.unit ? ` · ${field.unit}` : ""}`,
        onclick: () => {
          state.table.desc = state.table.sortBy === field.name ? !state.table.desc : false;
          state.table.sortBy = field.name;
          loadTable();
        },
      }, `${field.name}${field.unit ? ` [${field.unit}]` : ""}${arrow}`);
    }),
  );
  const body = rows.map((row) =>
    el(
      "tr",
      {
        class: state.highlight?.layerId === layer.id && state.highlight?.featureId === row.id ? "selected" : "",
        onclick: () => selectFeature(layer, row.id),
      },
      el("td", { class: "num" }, row.id),
      ...fields.map((field) => {
        const value = row.properties[field.name];
        return el("td", { class: typeof value === "number" ? "num" : "" }, formatValue(value));
      }),
    ),
  );
  $("gis-table").replaceChildren(el("thead", {}, head), el("tbody", {}, body));
}

async function selectFeature(layer, featureId) {
  state.highlight = { layerId: layer.id, featureId };
  state.visible.set(layer.id, true);
  const entry = await loadGeometry(layer.id);
  const feature = entry.features.find((f) => f.id === featureId);
  const geometryBox = feature ? featureBox(feature, entry.origin) : null;
  if (geometryBox) fitBounds(geometryBox);
  loadTable();
  render();
}

function featureBox(feature, origin) {
  const coords = [...feature.points];
  const regex = /[ML](-?[\d.]+) (-?[\d.]+)/g;
  let match;
  while ((match = regex.exec(feature.d))) {
    coords.push(lonLat(Number(match[1]) + origin[0], Number(match[2]) + origin[1], BASE_ZOOM));
  }
  if (!coords.length) return null;
  const lons = coords.map((c) => c[0]);
  const lats = coords.map((c) => c[1]);
  const pad = 0.002;
  return [Math.min(...lons) - pad, Math.min(...lats) - pad, Math.max(...lons) + pad, Math.max(...lats) + pad];
}

function setupTable() {
  const apply = () => {
    state.table.where = $("gis-where").value.trim();
    state.table.offset = 0;
    loadTable();
  };
  $("gis-where-btn").addEventListener("click", apply);
  $("gis-where").addEventListener("keydown", (event) => {
    if (event.key === "Enter") apply();
  });
  $("gis-prev").addEventListener("click", () => {
    state.table.offset = Math.max(0, state.table.offset - state.table.limit);
    loadTable();
  });
  $("gis-next").addEventListener("click", () => {
    if (state.table.offset + state.table.limit < state.table.total) {
      state.table.offset += state.table.limit;
      loadTable();
    }
  });
}

// ---------------------------------------------------------------------------
// Asking, plans, and provenance
// ---------------------------------------------------------------------------

function planContext() {
  return { point: state.clickPoint, selected_layer: state.activeTable };
}

function setupAsk() {
  const run = async (execute) => {
    const prompt = $("gis-prompt").value.trim();
    if (!prompt) return;
    const mode = $("gis-planner-mode").value;
    $("gis-popup").hidden = true;
    $("gis-ask-btn").disabled = true;
    $("gis-plan-btn").disabled = true;
    setStatus(execute ? "計画して実行しています…" : "計画しています…");
    try {
      if (execute) {
        const result = await postJson("/api/gis/ask", { prompt, context: planContext(), mode, keep_intermediate: $("gis-keep-intermediate").checked });
        await refreshLayers();
        state.visible.set(result.output_id, true);
        showRun(result, result.planner);
        const output = layerById(result.output_id);
        if (output?.bbox_wgs84) fitBounds(output.bbox_wgs84);
        openTable(result.output_id);
        setStatus("実行して検証しました", "");
      } else {
        const planned = await postJson("/api/gis/plan", { prompt, context: planContext(), mode });
        showPlan(planned);
        setStatus("計画を作成しました（未実行）");
      }
    } catch (error) {
      showError(prompt, error);
      setStatus(error.message, "gis-error");
    } finally {
      $("gis-ask-btn").disabled = false;
      $("gis-plan-btn").disabled = false;
    }
  };
  $("gis-ask-btn").addEventListener("click", () => run(true));
  $("gis-plan-btn").addEventListener("click", () => run(false));
  $("gis-prompt").addEventListener("keydown", (event) => {
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) run(true);
  });
}

function stepCard(step, receipt) {
  const params = step.params && Object.keys(step.params).length ? JSON.stringify(step.params) : "";
  const inputs = Object.entries(step.inputs || {})
    .map(([role, ref]) => `${role}=${layerById(ref)?.name || ref}`)
    .join(", ");
  const card = el(
    "div",
    { class: "gis-step" },
    el("div", { class: "gis-step-head" }, el("strong", {}, `${step.id} · ${OP_TITLES[step.op] || step.op}`), receipt ? el("span", { class: "status" }, `${receipt.feature_count} 件 · ${receipt.crs}`) : null),
    inputs ? el("div", {}, el("code", {}, inputs)) : null,
    params ? el("div", {}, el("code", {}, params)) : null,
  );
  if (receipt) {
    for (const check of receipt.checks) {
      card.append(el("div", { class: `gis-check-item ${check.passed ? "pass" : "fail"}` }, `${check.id}: ${check.detail}`));
    }
    for (const note of receipt.notes) card.append(el("div", { class: "status" }, note));
  }
  return card;
}

function plannerBlock(planner) {
  if (!planner) return [];
  return [
    el("h4", {}, "計画"),
    el("div", {}, `プランナー: ${planner.backend === "llm" ? "LLM" : "ルール"} · 確信度 ${(planner.confidence * 100).toFixed(0)}%`),
    ...planner.rationale.map((r) => el("div", {}, `・${r}`)),
    ...(planner.plan.assumptions || []).map((a) => el("div", { class: "status" }, `前提: ${a}`)),
    ...(planner.ambiguities || []).map((a) => el("div", { class: "status" }, `要確認: ${a}`)),
    ...(planner.rejected_attempts || []).map((a) => el("div", { class: "gis-error" }, `却下された計画: ${a}`)),
  ];
}

function answerLine(result) {
  const row = result.preview?.rows?.[0];
  const output = layerById(result.output_id);
  if (!row || !output) return null;
  if (result.preview.total_rows !== 1) {
    return el("div", { class: "gis-answer" }, `結果: ${result.preview.total_rows.toLocaleString("ja-JP")} 件の地物`);
  }
  const parts = output.fields
    .filter((f) => ["count", "population", "count_within"].includes(f.name) || f.name.startsWith("aw_") || f.name.startsWith("sum_"))
    .map((f) => `${f.name} = ${formatValue(row.properties[f.name])}${f.unit ? ` ${f.unit}` : ""}`);
  return parts.length ? el("div", { class: "gis-answer" }, parts.join(" · ")) : null;
}

function showRun(result, planner) {
  const receipt = result.receipt;
  const byId = new Map(receipt.steps.map((s) => [s.id, s]));
  const plan = result.workflow ? stepsFromWorkflow(result.workflow) : result.plan?.steps || [];
  const allPassed = receipt.steps.every((s) => s.checks.every((c) => c.passed));
  $("gis-result").replaceChildren(
    el("div", {}, el("strong", {}, receipt.goal), " ", el("span", { class: `gis-badge ${allPassed ? "ok" : "bad"}` }, allPassed ? "全検証パス" : "検証失敗")),
    answerLine(result),
    ...plannerBlock(planner),
    el("h4", {}, "Workflow（実行したグラフ）"),
    ...plan.map((step) => stepCard(step, byId.get(step.id))),
    el("h4", {}, "結果"),
    el("div", {}, `${receipt.output.name} · ${kindLabel(receipt.output.geometry_kind)} · ${receipt.output.feature_count} 件 · ${receipt.output.crs}`),
    ...receipt.output.fields.filter((f) => f.unit).map((f) => el("div", { class: "status" }, `${f.name}: ${f.unit}`)),
    el("h4", {}, "来歴"),
    el("div", { class: "gis-digest" }, `command ${receipt.command_id}`),
    el("div", { class: "gis-digest" }, `workflow ${receipt.workflow_digest}`),
    el("div", { class: "gis-digest" }, `result ${receipt.result_digest}`),
    el("div", { class: "gis-digest" }, `layer ${receipt.output.digest}`),
    receipt.output.provenance?.attribution ? el("div", {}, `出典: ${receipt.output.provenance.attribution}`) : null,
    receipt.output.provenance?.license ? el("div", {}, `ライセンス: ${receipt.output.provenance.license}`) : null,
  );
}

function stepsFromWorkflow(workflow) {
  return workflow.steps.map((step) => ({
    id: step.stable_id,
    op: step.operation.replace(/^toolkit\./, ""),
    params: step.parameters?.params,
    inputs: Object.fromEntries(Object.entries(step.parameters?.inputs || {}).map(([role, source]) => [role, source.layer || source.step])),
  }));
}

function showPlan(planned) {
  $("gis-result").replaceChildren(
    el("div", {}, el("strong", {}, planned.plan.goal), " ", el("span", { class: "gis-badge warn" }, "未実行")),
    ...plannerBlock(planned),
    el("h4", {}, "計画されたステップ"),
    ...planned.plan.steps.map((step) => stepCard(step, null)),
    el("div", { class: "status" }, "「質問して実行」で Command + Workflow を通して実行し、各ステップの独立検証を行います。"),
  );
}

function showError(prompt, error) {
  $("gis-result").replaceChildren(
    el("div", {}, el("strong", {}, prompt)),
    el("div", { class: "gis-error" }, error.message),
    el("div", { class: "status" }, "検証に失敗した結果はレイヤとして保存されません（fail-closed）。"),
  );
}

function showImport(result) {
  const r = result.receipt;
  $("gis-result").replaceChildren(
    el("div", {}, el("strong", {}, `${r.layer.name} を取り込みました`)),
    el("div", {}, `${r.report.format} · ${r.layer.feature_count} 件 · ${r.layer.crs} (${r.layer.crs_name})`),
    r.layer.crs_needs_confirmation ? el("div", { class: "gis-callout" }, "CRS は座標範囲から推定しました。レイヤの「CRS確認」から確定してください。") : null,
    r.report.encoding ? el("div", {}, `文字コード: ${r.report.encoding}`) : null,
    ...r.report.notes.map((n) => el("div", { class: "status" }, n)),
    r.report.skipped.length ? el("h4", {}, `読み込めなかった行 (${r.report.skipped.length})`) : null,
    ...r.report.skipped.slice(0, 20).map((s) => el("div", { class: "gis-error" }, `#${s.record}: ${s.reason}`)),
    el("h4", {}, "列"),
    ...r.layer.fields.map((f) => el("div", {}, `${f.name} · ${f.type}${f.unit ? ` · ${f.unit}` : ""}`)),
    el("h4", {}, "来歴"),
    el("div", { class: "gis-digest" }, `source ${r.report.source_sha256}`),
    el("div", { class: "gis-digest" }, `workflow ${r.workflow_digest}`),
    el("div", { class: "gis-digest" }, `layer ${r.result_digest}`),
  );
}

function showPlace(result) {
  const r = result.receipt;
  $("gis-result").replaceChildren(
    el("div", {}, el("strong", {}, `${r.layer.name} を追加しました`)),
    el("div", {}, `${kindLabel(r.layer.geometry_kind)} · ${r.layer.crs} · ${r.layer.provenance.attribution} (${r.layer.provenance.license})`),
    ...r.layer.provenance.notes.map((n) => el("div", { class: "status" }, n)),
    el("h4", {}, "候補"),
    ...r.candidates.map((c, i) => el("div", {}, `${i === r.chosen ? "▶ " : "　"}${c.name}（${c.kind}${c.has_boundary ? "・境界あり" : ""}）`)),
    el("h4", {}, "来歴"),
    el("div", { class: "gis-digest" }, r.source_uri),
    el("div", { class: "gis-digest" }, `response ${r.response_sha256}`),
    el("div", { class: "gis-digest" }, `workflow ${r.workflow_digest}`),
  );
}

function showLayerProvenance(layer) {
  const p = layer.provenance || {};
  const receipt = layer.receipt || {};
  if (receipt.kind === "analysis" && receipt.run) {
    showRun({ receipt: receipt.run, plan: receipt.plan, output_id: layer.id }, null);
    return;
  }
  $("gis-result").replaceChildren(
    el("div", {}, el("strong", {}, layer.name)),
    el("div", {}, `${kindLabel(layer.geometry_kind)} · ${layer.feature_count} 件 · ${layer.crs} (${layer.crs_name}) · CRS: ${layer.crs_status}`),
    el("h4", {}, "出典"),
    el("div", { class: "gis-digest" }, p.source_uri || "—"),
    p.source_sha256 ? el("div", { class: "gis-digest" }, `source ${p.source_sha256}`) : null,
    p.attribution ? el("div", {}, `出典表示: ${p.attribution}`) : null,
    p.license ? el("div", {}, `ライセンス: ${p.license}`) : null,
    ...(p.notes || []).map((n) => el("div", { class: "status" }, n)),
    el("h4", {}, "識別子"),
    el("div", { class: "gis-digest" }, `layer ${layer.digest}`),
    p.workflow_digest ? el("div", { class: "gis-digest" }, `workflow ${p.workflow_digest}`) : null,
    el("h4", {}, "列"),
    ...layer.fields.map((f) => el("div", {}, `${f.name} · ${f.type}${f.unit ? ` · ${f.unit}` : ""}`)),
  );
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

async function boot() {
  if (!$("gis-view")) return;
  setupMapInteraction();
  setupImport();
  setupTable();
  setupAsk();
  try {
    for (const op of await api("/api/gis/operations")) OP_TITLES[op.name] = op.title;
    const added = await refreshLayers();
    if (state.layers.length) fitAll();
    if (!added.length && !state.layers.length) setStatus("データを読み込むか、サンプルを試してください");
    if (state.llmReady) $("gis-planner-mode").title = "LLM プランナーが設定されています";
  } catch (error) {
    setStatus(`GIS API に接続できません: ${error.message}`, "gis-error");
  }
  let saved = null;
  try {
    saved = localStorage.getItem("genegis.view");
  } catch {
    /* storage unavailable */
  }
  if (new URLSearchParams(location.search).get("view") === "gis" || saved === "gis-view") showView("gis-view");
}

boot();
