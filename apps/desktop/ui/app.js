const promptEl = document.getElementById("prompt");
const runBtn = document.getElementById("run-btn");
const downloadPngBtn = document.getElementById("download-png-btn");
const gpuPreviewBtn = document.getElementById("gpu-preview-btn");
const statusEl = document.getElementById("status");
const mapFrame = document.getElementById("map-frame");
const resolutionEl = document.getElementById("resolution");
const summaryEl = document.getElementById("summary");
const datasetEl = document.getElementById("dataset");
const verificationEl = document.getElementById("verification");
const notesEl = document.getElementById("notes");

let lastPngBase64 = null;

function setStatus(text, busy = false) {
  statusEl.textContent = text;
  runBtn.disabled = busy;
  downloadPngBtn.disabled = busy || !lastPngBase64;
  gpuPreviewBtn.disabled = busy || !pipelineReady;
}

let pipelineReady = false;

function setPipelineReady(ready) {
  pipelineReady = ready;
  gpuPreviewBtn.disabled = !pipelineReady;
}

function setPngExport(pngBase64) {
  lastPngBase64 = pngBase64 || null;
  downloadPngBtn.disabled = !lastPngBase64;
}

function downloadPng() {
  if (!lastPngBase64) {
    return;
  }

  const link = document.createElement("a");
  link.href = `data:image/png;base64,${lastPngBase64}`;
  link.download = "nagoya-density.png";
  link.click();
}

function renderVerification(checks) {
  verificationEl.innerHTML = "";
  for (const check of checks) {
    const li = document.createElement("li");
    li.className = check.passed ? "ok" : "bad";
    li.textContent = `${check.passed ? "✓" : "✗"} ${check.name}: ${check.detail}`;
    verificationEl.appendChild(li);
  }
}

function renderNotes(notes) {
  notesEl.innerHTML = "";
  for (const note of notes) {
    const li = document.createElement("li");
    li.className = "note";
    li.textContent = note;
    notesEl.appendChild(li);
  }
}

async function invokeAsk(prompt) {
  if (window.__TAURI__?.core?.invoke) {
    return window.__TAURI__.core.invoke("run_ask", { prompt });
  }

  const response = await fetch("/api/ask", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ prompt }),
  });
  const payload = await response.json();
  if (!payload.ok || !payload.result) {
    throw new Error(payload.error || "Request failed");
  }
  return payload.result;
}

async function invokeGpuPreview() {
  if (window.__TAURI__?.core?.invoke) {
    return window.__TAURI__.core.invoke("launch_gpu_preview");
  }

  const response = await fetch("/api/gpu-preview", { method: "POST" });
  const payload = await response.json();
  if (!payload.ok) {
    throw new Error(payload.error || "GPU preview failed");
  }
  return payload.message || "WebGPU choropleth preview launched";
}

async function openGpuPreview() {
  setStatus("Launching GPU map…", true);
  try {
    const message = await invokeGpuPreview();
    setStatus(message);
  } catch (err) {
    console.error(err);
    setStatus(`Error: ${err.message || err}`);
  }
}

async function runAsk() {
  const prompt = promptEl.value.trim();
  if (!prompt) {
    setStatus("Prompt is empty");
    return;
  }

  setStatus("Running pipeline…", true);
  setPngExport(null);
  setPipelineReady(false);
  try {
    const result = await invokeAsk(prompt);

    resolutionEl.textContent = [
      `workflow: ${result.workflow_id}`,
      `confidence: ${(result.confidence * 100).toFixed(0)}%`,
      `steps: ${result.workflow_steps}`,
      `duckdb: ${result.duckdb_verified ? "passed" : "failed"}`,
    ].join("\n");

    datasetEl.textContent = result.dataset
      ? [
          `id: ${result.dataset.id}`,
          `title: ${result.dataset.title}`,
          `format: ${result.dataset.format?.kind ?? "—"}`,
          `crs: ${result.dataset.crs}`,
          `uri: ${result.dataset.uri}`,
          `license: ${result.dataset.license}`,
        ].join("\n")
      : "—";

    summaryEl.textContent = JSON.stringify(result.summary, null, 2);
    renderVerification(result.verification.checks);
    renderNotes(result.ambiguities);
    mapFrame.srcdoc = result.html;
    setPngExport(result.png_base64);
    setPipelineReady(true);
    setStatus("Done");
  } catch (err) {
    console.error(err);
    setStatus(`Error: ${err.message || err}`);
  }
}

runBtn.addEventListener("click", runAsk);
downloadPngBtn.addEventListener("click", downloadPng);
gpuPreviewBtn.addEventListener("click", openGpuPreview);
runAsk();
