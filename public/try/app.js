// GeneGIS in the browser: the toolkit compiled to wasm, no server.
import init, { GeneGis } from "./pkg/genegis_wasm.js";

const $ = (id) => document.getElementById(id);
const EXAMPLES = [
  "区の人口密度",
  "駅から500m以内の避難所を数えて",
  "区ごとの店舗数",
  "浸水想定区域から1km以内の避難所",
  "この点から1km以内の人口は？",
];
const RAMP = ["#fde68a", "#fbbf24", "#f97316", "#dc2626", "#7f1d1d"];

let gis;
let map;
let mapReady = false;
let lastResult = null;
let clickedPoint = null;
let marker = null;
const shown = new Set();

function setBusy(busy, text) {
  for (const id of ["ask", "samples"]) $(id).disabled = busy;
  if (text !== undefined) $("status").textContent = text;
}

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

function download(filename, bytes, type) {
  const url = URL.createObjectURL(new Blob([bytes], { type }));
  const a = Object.assign(document.createElement("a"), { href: url, download: filename });
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function renderLayers() {
  if (!gis) return;
  const layers = JSON.parse(gis.layers());
  $("layers").innerHTML = layers
    .map((l) => `<li><span>${escapeHtml(l.name)}</span><small>${l.feature_count} 件 · ${escapeHtml(l.crs)}</small></li>`)
    .join("");
  for (const layer of layers) addToMap(layer.id, false);
}

function addToMap(id, highlight, colorField) {
  if (!mapReady) return;
  const source = `src-${id}`;
  if (shown.has(id) && !highlight) return;
  const data = JSON.parse(gis.geojson(id));
  if (map.getSource(source)) {
    map.getSource(source).setData(data);
  } else {
    map.addSource(source, { type: "geojson", data });
  }
  shown.add(id);
  for (const suffix of ["fill", "line", "circle"]) {
    if (map.getLayer(`${source}-${suffix}`)) map.removeLayer(`${source}-${suffix}`);
  }
  const color = highlight ? colorExpression(data, colorField) : "#64748b";
  map.addLayer({
    id: `${source}-fill`, type: "fill", source, filter: ["==", ["geometry-type"], "Polygon"],
    paint: { "fill-color": color, "fill-opacity": highlight ? 0.72 : 0.08 },
  });
  map.addLayer({
    id: `${source}-line`, type: "line", source,
    paint: { "line-color": highlight ? "#0b1222" : "#64748b", "line-width": highlight ? 1.2 : 0.7 },
  });
  map.addLayer({
    id: `${source}-circle`, type: "circle", source, filter: ["==", ["geometry-type"], "Point"],
    paint: {
      "circle-radius": highlight ? 6 : 3,
      "circle-color": highlight ? color : "#94a3b8",
      "circle-stroke-color": "#0b1222", "circle-stroke-width": 1,
    },
  });
}

// Features behind an aggregate answer (e.g. the shelters that were counted).
function showDisplay(data) {
  if (!mapReady) return;
  if (map.getSource("display")) {
    map.getSource("display").setData(data);
  } else {
    map.addSource("display", { type: "geojson", data });
    map.addLayer({
      id: "display-fill", type: "fill", source: "display", filter: ["==", ["geometry-type"], "Polygon"],
      paint: { "fill-color": RAMP[3], "fill-opacity": 0.6 },
    });
    map.addLayer({
      id: "display-circle", type: "circle", source: "display", filter: ["==", ["geometry-type"], "Point"],
      paint: { "circle-radius": 9, "circle-color": RAMP[1], "circle-stroke-color": "#0b1222", "circle-stroke-width": 2 },
    });
  }
}

function clearDisplay() {
  if (mapReady && map.getSource("display")) map.getSource("display").setData({ type: "FeatureCollection", features: [] });
}

function colorExpression(data, field) {
  const values = field
    ? data.features.map((f) => f.properties?.[field]).filter((v) => typeof v === "number").sort((a, b) => a - b)
    : [];
  if (values.length < 2 || values[0] === values[values.length - 1]) return RAMP[2];
  const breaks = [1, 2, 3, 4].map((i) => values[Math.floor((values.length - 1) * (i / 5))]);
  const steps = ["step", ["to-number", ["get", field], 0], RAMP[0]];
  breaks.forEach((b, i) => steps.push(b, RAMP[i + 1]));
  return steps;
}

function pickColorField(plan, rows) {
  const props = rows?.rows?.[0]?.properties ?? {};
  const last = plan?.steps?.[plan.steps.length - 1];
  const declared = last?.params?.field;
  if (declared && typeof props[declared] === "number") return declared;
  const key = Object.keys(props).find((k) => typeof props[k] === "number" && /density|count|sum|distance|area|length/.test(k));
  return key;
}

function renderTable(rows, colorField) {
  const records = rows?.rows ?? [];
  if (!records.length) {
    $("table").innerHTML = "";
    return;
  }
  const keys = Object.keys(records[0].properties ?? {}).filter((k) => !/^boundary_|census_year|_code$|_url$|license|retrieval/.test(k));
  const named = keys.filter((k) => /name|名/.test(k)).slice(0, 2);
  const numeric = keys.filter((k) => typeof records[0].properties[k] === "number" && !named.includes(k));
  const columns = [...new Set([...named, ...(colorField ? [colorField] : []), ...numeric, ...keys])].slice(0, 6);
  const cell = (v) => (typeof v === "number" ? v.toLocaleString("ja-JP", { maximumFractionDigits: 2 }) : escapeHtml(v ?? ""));
  $("table").innerHTML = `<table><thead><tr>${columns.map((c) => `<th>${escapeHtml(c)}</th>`).join("")}</tr></thead><tbody>${records
    .slice(0, 50)
    .map((r) => `<tr>${columns.map((c) => `<td>${cell(r.properties[c])}</td>`).join("")}</tr>`)
    .join("")}</tbody></table>`;
}

function renderResult(answer) {
  const { planned, result } = answer;
  const receipt = result.receipt;
  const checks = receipt.steps.flatMap((s) => s.checks);
  const passed = checks.filter((c) => c.passed).length;
  const steps = receipt.steps
    .map(
      (s) => `<div class="step"><code>${escapeHtml(s.op)}</code> → ${s.feature_count} 件 <small class="mono">${escapeHtml(s.crs)}</small>
        ${s.checks.map((c) => `<div class="check ${c.passed ? "pass" : "fail"}">${escapeHtml(c.id)} — ${escapeHtml(c.detail)}</div>`).join("")}</div>`,
    )
    .join("");
  const assumptions = [...(planned.plan.assumptions ?? []), ...(receipt.assumptions ?? [])];
  $("result").innerHTML = `<div class="card">
      <span class="badge ok">✓ 検証済み ${passed}/${checks.length}</span>
      <div style="margin-top:6px">${escapeHtml(planned.rationale?.join(" ") ?? "")}</div>
      ${assumptions.length ? `<div class="status">前提: ${assumptions.map(escapeHtml).join(" / ")}</div>` : ""}
      ${steps}
      <div class="mono" style="margin-top:8px">workflow ${escapeHtml(receipt.workflow_digest)}<br>result ${escapeHtml(receipt.result_digest)}</div>
      <div class="row">
        <button data-export="geojson">GeoJSON</button>
        <button data-export="csv">CSV</button>
        <button data-export="evidence">根拠 (JSON)</button>
      </div>
    </div>`;
  for (const button of $("result").querySelectorAll("[data-export]")) {
    button.onclick = () => {
      const format = button.dataset.export;
      if (format === "evidence") {
        download(`genegis-evidence-${result.output_id}.json`, gis.evidence(result.output_id), "application/json");
      } else {
        download(gis.exportName(result.output_id, format), gis.export(result.output_id, format), format === "csv" ? "text/csv" : "application/geo+json");
      }
    };
  }
  const colorField = pickColorField(planned.plan, result.rows);
  renderLayers();
  renderTable(result.rows, colorField);
  lastResult = { result, colorField };
  drawResult();
}

// Map side of a result; replayed once the map is ready, so analysis never
// waits for basemap tiles.
function drawResult() {
  if (!mapReady || !lastResult) return;
  const { result, colorField } = lastResult;
  clearDisplay();
  addToMap(result.output_id, true, colorField);
  if (result.display_geojson) showDisplay(JSON.parse(result.display_geojson));
  const bbox = result.layer.bbox_wgs84;
  if (bbox) map.fitBounds([[bbox[0], bbox[1]], [bbox[2], bbox[3]]], { padding: 40, maxZoom: 14 });
}

function renderError(question, error) {
  $("result").innerHTML = `<div class="card bad"><strong>答えを返しませんでした</strong>
    <div style="margin-top:6px">${escapeHtml(error?.message ?? error)}</div>
    <div class="status">計画の検証かステップのチェックに通らなかったため、未検証の数値は出しません。質問を言い換えるか、必要なデータを読み込んでください。</div></div>`;
  $("table").innerHTML = "";
}

async function importFiles(files) {
  setBusy(true, "読み込み中…");
  const messages = [];
  for (const file of files) {
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      const imported = JSON.parse(gis.import(file.name, bytes, "{}"));
      messages.push(`${file.name}: ${imported.layer.feature_count} 件 (${imported.layer.crs})`);
    } catch (error) {
      messages.push(`${file.name}: ${error.message ?? error}`);
    }
  }
  renderLayers();
  setBusy(false, messages.join(" / "));
}

function ask() {
  const question = $("question").value.trim();
  if (!question) return;
  setBusy(true, "計画・実行・検証中…");
  try {
    const context = clickedPoint ? JSON.stringify({ point: clickedPoint }) : "";
    renderResult(JSON.parse(gis.ask(question, context)));
    setBusy(false, "完了");
  } catch (error) {
    renderError(question, error);
    setBusy(false, "検証を通らなかったため停止しました");
  }
}

async function main() {
  map = new maplibregl.Map({
    container: "map",
    style: {
      version: 8,
      sources: {
        gsi: {
          type: "raster",
          tiles: ["https://cyberjapandata.gsi.go.jp/xyz/pale/{z}/{x}/{y}.png"],
          tileSize: 256,
          attribution: '<a href="https://maps.gsi.go.jp/development/ichiran.html">地理院タイル</a>',
        },
      },
      layers: [{ id: "gsi", type: "raster", source: "gsi", paint: { "raster-saturation": -0.6, "raster-brightness-max": 0.55 } }],
    },
    center: [136.92, 35.15],
    zoom: 10.3,
  });
  map.addControl(new maplibregl.NavigationControl(), "top-right");
  // A click sets the point that 「この点」 / "here" refers to.
  map.on("click", (e) => {
    clickedPoint = [e.lngLat.lng, e.lngLat.lat];
    if (!marker) marker = new maplibregl.Marker({ color: "#38bdf8" });
    marker.setLngLat(e.lngLat).addTo(map);
    $("status").textContent = `地点を選択: ${clickedPoint.map((v) => v.toFixed(4)).join(", ")}（「この点」で使われます）`;
  });
  // Draw as soon as the style is ready; basemap tiles may be slow or offline.
  const loaded = new Promise((resolve) => {
    map.once("style.load", resolve);
    map.once("load", resolve);
  });

  $("chips").innerHTML = EXAMPLES.map((q) => `<button>${escapeHtml(q)}</button>`).join("");
  for (const chip of $("chips").querySelectorAll("button")) {
    chip.onclick = () => {
      $("question").value = chip.textContent;
      ask();
    };
  }

  loaded.then(() => {
    mapReady = true;
    renderLayers();
    drawResult();
  });
  await init();
  gis = new GeneGis();
  const about = JSON.parse(gis.about());
  setBusy(false, `エンジン準備完了 (genegis-wasm ${about.version}) — サンプルを読み込むか、ファイルをドロップしてください`);

  $("samples").onclick = () => {
    setBusy(true, "サンプル読み込み中…");
    try {
      gis.loadSamples();
      renderLayers();
      setBusy(false, "サンプルを読み込みました（名古屋市の区界・人口、避難所、店舗、浸水想定区域、主要駅）");
    } catch (error) {
      setBusy(false, `サンプル読み込み失敗: ${error.message ?? error}`);
    }
  };
  $("ask").onclick = ask;
  $("question").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) ask();
  });
  $("file").onchange = (e) => importFiles([...e.target.files]);
  const drop = $("drop");
  drop.addEventListener("dragover", (e) => {
    e.preventDefault();
    drop.classList.add("over");
  });
  drop.addEventListener("dragleave", () => drop.classList.remove("over"));
  drop.addEventListener("drop", (e) => {
    e.preventDefault();
    drop.classList.remove("over");
    importFiles([...e.dataTransfer.files]);
  });

  const params = new URLSearchParams(location.search);
  if (params.has("demo")) {
    $("samples").click();
    $("question").value = params.get("q") || EXAMPLES[1];
    ask();
  }
}

main().catch((error) => {
  $("status").textContent = `エンジンの読み込みに失敗しました: ${error.message ?? error}`;
});
