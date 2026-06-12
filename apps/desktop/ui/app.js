const promptEl = document.getElementById("prompt");
const runBtn = document.getElementById("run-btn");
const downloadPngBtn = document.getElementById("download-png-btn");
const gpuPreviewBtn = document.getElementById("gpu-preview-btn");
const statusEl = document.getElementById("status");
const mapFrame = document.getElementById("map-frame");
const resolutionEl = document.getElementById("resolution");
const summaryEl = document.getElementById("summary");
const datasetEl = document.getElementById("dataset");
const pluginsEl = document.getElementById("plugins");
const commentsEl = document.getElementById("comments");
const agentMetaEl = document.getElementById("agent-meta");
const agentStepsEl = document.getElementById("agent-steps");
const agentPlanBtn = document.getElementById("agent-plan-btn");
const agentExecuteBtn = document.getElementById("agent-execute-btn");
const commentFormEl = document.getElementById("comment-form");
const commentAuthorEl = document.getElementById("comment-author");
const commentBodyEl = document.getElementById("comment-body");
const commentSyncBtn = document.getElementById("comment-sync-btn");
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

function renderPlugins(plugins, pluginRoot) {
  pluginsEl.innerHTML = "";

  if (!plugins.length) {
    const empty = document.createElement("p");
    empty.className = "plugin-empty";
    empty.textContent = pluginRoot
      ? `No plugins found in ${pluginRoot}`
      : "No plugins discovered";
    pluginsEl.appendChild(empty);
    return;
  }

  for (const plugin of plugins) {
    const card = document.createElement("article");
    card.className = "plugin-item";

    const title = document.createElement("strong");
    title.textContent = plugin.name || plugin.id;
    card.appendChild(title);

    const meta = document.createElement("div");
    meta.className = "plugin-meta";
    meta.textContent = `${plugin.id} · v${plugin.version}`;
    card.appendChild(meta);

    if (plugin.description) {
      const description = document.createElement("p");
      description.className = "plugin-description";
      description.textContent = plugin.description;
      card.appendChild(description);
    }

    const caps = document.createElement("div");
    caps.className = "plugin-caps";
    const effective = plugin.effective_capabilities || plugin.capabilities || [];
    caps.textContent = effective.length
      ? `caps: ${effective.join(", ")}`
      : "caps: —";
    card.appendChild(caps);

    pluginsEl.appendChild(card);
  }
}

async function invokePlugins() {
  if (window.__TAURI__?.core?.invoke) {
    return window.__TAURI__.core.invoke("list_plugins");
  }

  const response = await fetch("/api/plugins");
  const payload = await response.json();
  if (!payload.ok) {
    throw new Error(payload.error || "Failed to load plugins");
  }
  return payload;
}

async function loadPlugins() {
  try {
    const payload = await invokePlugins();
    renderPlugins(payload.plugins || [], payload.plugin_root || "");
  } catch (err) {
    console.error(err);
    pluginsEl.textContent = `Error: ${err.message || err}`;
  }
}

function renderAgentRun(run) {
  agentStepsEl.innerHTML = "";
  if (!run) {
    agentMetaEl.textContent = "No agent run yet — try: genegis agent run";
    agentStepsEl.textContent = "Run the north-star prompt to populate trace.";
    return;
  }

  agentMetaEl.textContent = [
    `run: ${run.id.slice(0, 8)}…`,
    run.workflow_id ? `workflow: ${run.workflow_id}` : "plan-only",
    run.verification_passed ? "verification: passed" : "verification: pending/failed",
    run.verify_attempts ? `attempts: ${run.verify_attempts}` : "",
  ]
    .filter(Boolean)
    .join(" · ");

  for (const step of run.steps || []) {
    const card = document.createElement("article");
    card.className = `agent-item ${step.tool_calls?.every((call) => call.ok) ? "ok" : "bad"}`;

    const role = document.createElement("div");
    role.className = "agent-role";
    role.textContent = `${step.role} · ${step.agent}`;
    card.appendChild(role);

    const detail = document.createElement("div");
    detail.className = "agent-detail";
    detail.textContent = step.detail;
    card.appendChild(detail);

    const tools = document.createElement("div");
    tools.className = "agent-tools";
    const toolNames = (step.tool_calls || []).map((call) => call.tool).join(", ");
    tools.textContent = toolNames ? `tools: ${toolNames}` : "tools: —";
    card.appendChild(tools);

    agentStepsEl.appendChild(card);
  }
}

async function loadAgentTrace() {
  try {
    const response = await fetch("/api/agent/runs/latest");
    const payload = await response.json();
    renderAgentRun(payload.run);
  } catch (err) {
    console.error(err);
    agentMetaEl.textContent = `Error: ${err.message || err}`;
    agentStepsEl.textContent = "";
  }
}

async function invokeAgentPlan(prompt) {
  const response = await fetch("/api/agent/plan", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ prompt }),
  });
  return response.json();
}

async function invokeAgentExecute() {
  const response = await fetch("/api/agent/execute", { method: "POST" });
  return response.json();
}

function renderCollabSync(sync) {
  if (!sync) {
    collabSyncEl.textContent = "Collab sync unavailable";
    collabSyncEl.className = "collab-sync warn";
    return;
  }

  const status = sync.synced ? "synced" : "offline";
  const detail = sync.error ? ` · ${sync.error}` : "";
  collabSyncEl.textContent = `${status} · source: ${sync.source} · ${sync.server_url}${detail}`;
  collabSyncEl.className = sync.synced ? "collab-sync ok" : "collab-sync warn";
}

function renderComments(comments) {
  commentsEl.innerHTML = "";
  if (!comments.length) {
    commentsEl.textContent = "No comments yet";
    return;
  }

  for (const comment of comments) {
    const card = document.createElement("article");
    card.className = "comment-item";

    const header = document.createElement("div");
    header.className = "comment-meta";
    header.textContent = `${comment.author} · ${comment.body.slice(0, 40)}${
      comment.body.length > 40 ? "…" : ""
    }`;
    card.appendChild(header);

    const body = document.createElement("p");
    body.className = "comment-body";
    body.textContent = comment.body;
    card.appendChild(body);

    if (comment.map_anchor) {
      const anchor = document.createElement("div");
      anchor.className = "comment-anchor";
      anchor.textContent = `map: ${comment.map_anchor[0].toFixed(3)}, ${comment.map_anchor[1].toFixed(3)}`;
      card.appendChild(anchor);
    }

    if (comment.agent_run_id) {
      const agentLink = document.createElement("div");
      agentLink.className = "comment-agent";
      const step = comment.agent_step_id ? ` · step ${comment.agent_step_id.slice(0, 8)}…` : "";
      agentLink.textContent = `agent run: ${comment.agent_run_id.slice(0, 8)}…${step}`;
      card.appendChild(agentLink);
    }

    commentsEl.appendChild(card);
  }
}

async function invokeCollab() {
  if (window.__TAURI__?.core?.invoke) {
    return window.__TAURI__.core.invoke("collab_snapshot");
  }

  const response = await fetch("/api/collab");
  return response.json();
}

async function invokeCollabSync() {
  if (window.__TAURI__?.core?.invoke) {
    return invokeCollab();
  }

  const response = await fetch("/api/collab/sync", { method: "POST" });
  return response.json();
}

async function invokeAddComment(author, body) {
  if (window.__TAURI__?.core?.invoke) {
    return window.__TAURI__.core.invoke("collab_add_comment", { author, body });
  }

  const response = await fetch("/api/collab/comment", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ author, body }),
  });
  return response.json();
}

async function loadComments() {
  try {
    const payload = await invokeCollab();
    renderCollabSync(payload.sync);
    renderComments(payload.comments || []);
  } catch (err) {
    console.error(err);
    collabSyncEl.textContent = `Error: ${err.message || err}`;
    collabSyncEl.className = "collab-sync warn";
    commentsEl.textContent = `Error: ${err.message || err}`;
  }
}

async function syncComments() {
  commentSyncBtn.disabled = true;
  collabSyncEl.textContent = "Syncing…";
  try {
    const payload = await invokeCollabSync();
    renderCollabSync(payload.sync);
    renderComments(payload.comments || []);
  } catch (err) {
    console.error(err);
    collabSyncEl.textContent = `Sync error: ${err.message || err}`;
    collabSyncEl.className = "collab-sync warn";
  } finally {
    commentSyncBtn.disabled = false;
  }
}

async function submitComment(event) {
  event.preventDefault();
  const author = commentAuthorEl.value.trim();
  const body = commentBodyEl.value.trim();
  if (!author || !body) {
    return;
  }

  commentFormEl.querySelector("button[type='submit']").disabled = true;
  try {
    const payload = await invokeAddComment(author, body);
    if (!payload.ok) {
      throw new Error(payload.sync?.error || payload.summary?.error || "Failed to add comment");
    }
    renderCollabSync(payload.sync);
    renderComments(payload.comments || []);
    commentBodyEl.value = "";
  } catch (err) {
    console.error(err);
    collabSyncEl.textContent = `Error: ${err.message || err}`;
    collabSyncEl.className = "collab-sync warn";
  } finally {
    commentFormEl.querySelector("button[type='submit']").disabled = false;
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
    loadAgentTrace();
  } catch (err) {
    console.error(err);
    setStatus(`Error: ${err.message || err}`);
  }
}

runBtn.addEventListener("click", runAsk);
downloadPngBtn.addEventListener("click", downloadPng);
gpuPreviewBtn.addEventListener("click", openGpuPreview);
commentFormEl.addEventListener("submit", submitComment);
commentSyncBtn.addEventListener("click", syncComments);

agentPlanBtn?.addEventListener("click", async () => {
  const prompt = promptEl.value.trim();
  if (!prompt) {
    setStatus("Enter a prompt first");
    return;
  }
  agentPlanBtn.disabled = true;
  setStatus("Planning…");
  try {
    const payload = await invokeAgentPlan(prompt);
    if (!payload.ok) {
      throw new Error(payload.error || "Agent plan failed");
    }
    renderAgentRun(payload.run);
    await loadComments();
    setStatus("Plan saved — approve to execute");
  } catch (err) {
    console.error(err);
    setStatus(`Plan error: ${err.message || err}`);
  } finally {
    agentPlanBtn.disabled = false;
  }
});

agentExecuteBtn?.addEventListener("click", async () => {
  agentExecuteBtn.disabled = true;
  setStatus("Executing approved plan…");
  try {
    const payload = await invokeAgentExecute();
    if (!payload.ok) {
      throw new Error(payload.error || "Agent execute failed");
    }
    renderAgentRun(payload.run);
    await loadComments();
    setStatus("Agent run verified");
  } catch (err) {
    console.error(err);
    setStatus(`Execute error: ${err.message || err}`);
  } finally {
    agentExecuteBtn.disabled = false;
  }
});

loadPlugins();
loadComments();
loadAgentTrace();
runAsk();
