const promptEl = document.getElementById("prompt");
const runBtn = document.getElementById("run-btn");
const downloadPngBtn = document.getElementById("download-png-btn");
const gpuPreviewBtn = document.getElementById("gpu-preview-btn");
const statusEl = document.getElementById("status");
const mapFrame = document.getElementById("map-frame");
const resolutionEl = document.getElementById("resolution");
const summaryEl = document.getElementById("summary");
const datasetEl = document.getElementById("dataset");
const stacCollectionEl = document.getElementById("stac-collection");
const stacItemsEl = document.getElementById("stac-items");
const pluginsEl = document.getElementById("plugins");
const commentsEl = document.getElementById("comments");
const agentMetaEl = document.getElementById("agent-meta");
const agentStepsEl = document.getElementById("agent-steps");
const agentPlanBtn = document.getElementById("agent-plan-btn");
const agentExecuteBtn = document.getElementById("agent-execute-btn");
const agentRetryBtn = document.getElementById("agent-retry-btn");
const agentHistoryEl = document.getElementById("agent-history");
const provenanceEl = document.getElementById("provenance");

let activeProvenanceFilter = null;
const commentFormEl = document.getElementById("comment-form");
const commentAuthorEl = document.getElementById("comment-author");
const commentBodyEl = document.getElementById("comment-body");
const commentSyncBtn = document.getElementById("comment-sync-btn");
const verificationEl = document.getElementById("verification");
const notesEl = document.getElementById("notes");

let lastPngBase64 = null;
let lastWorkflowId = "nagoya-density";

function verificationProfile(workflowId) {
  if (workflowId === "remote-cog-demo" || workflowId === "local-cog-demo") {
    return {
      label: "cog metadata",
      verifier: "cog_metadata_verify",
      status: (passed) => (passed ? "COG metadata verified" : "COG metadata failed"),
    };
  }

  return {
    label: "duckdb",
    verifier: "duckdb_verify",
    status: (passed) => (passed ? "DuckDB verified" : "DuckDB failed"),
  };
}

function verificationLine(workflowId, passed) {
  const profile = verificationProfile(workflowId);
  return `${profile.label}: ${passed ? "passed" : "failed"}`;
}

function agentVerificationLine(run) {
  if (run.plan_only) {
    return "verification: plan-only";
  }
  const profile = verificationProfile(run.workflow_id);
  return run.verification_passed
    ? `verification: ${profile.status(true)}`
    : `verification: ${profile.status(false)}`;
}

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

async function loadStacCollection() {
  try {
    const response = await fetch("/api/stac/collection");
    const payload = await response.json();
    if (!payload.ok || !payload.collection) {
      throw new Error(payload.error || "Failed to load STAC collection");
    }

    const collection = payload.collection;
    stacCollectionEl.textContent = [
      `id: ${collection.id}`,
      `title: ${collection.title}`,
      `items: ${collection.item_count ?? "—"}`,
      `license: ${collection.license ?? "—"}`,
    ].join("\n");

    stacItemsEl.innerHTML = "";
    const itemIds = collection.item_ids || [];
    if (!itemIds.length) {
      stacItemsEl.textContent = "No STAC items in collection";
      return;
    }

    for (const itemId of itemIds) {
      const card = document.createElement("article");
      card.className = "stac-item";
      card.textContent = itemId;
      card.addEventListener("click", async () => {
        try {
          const itemResponse = await fetch(`/api/stac/items/${encodeURIComponent(itemId)}`);
          const itemPayload = await itemResponse.json();
          if (!itemPayload.ok || !itemPayload.item) {
            throw new Error(itemPayload.error || "Failed to load STAC item");
          }
          summaryEl.textContent = JSON.stringify(itemPayload.item, null, 2);
          setStatus(`STAC item loaded: ${itemId}`);
        } catch (err) {
          console.error(err);
          setStatus(`STAC item error: ${err.message || err}`);
        }
      });
      stacItemsEl.appendChild(card);
    }
  } catch (err) {
    console.error(err);
    stacCollectionEl.textContent = `Error: ${err.message || err}`;
    stacItemsEl.textContent = "";
  }
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
    if (agentRetryBtn) {
      agentRetryBtn.hidden = true;
    }
    return;
  }

  if (agentRetryBtn) {
    agentRetryBtn.hidden = !(run.verification_passed === false && run.plan_only === false);
  }

  agentMetaEl.textContent = [
    `run: ${run.id.slice(0, 8)}…`,
    run.workflow_id ? `workflow: ${run.workflow_id}` : "plan-only",
    agentVerificationLine(run),
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
    let latestPayload;
    let historyPayload;

    if (window.__TAURI__?.core?.invoke) {
      latestPayload = await window.__TAURI__.core.invoke("agent_runs_latest");
      historyPayload = await window.__TAURI__.core.invoke("agent_runs_list");
    } else {
      [latestPayload, historyPayload] = await Promise.all([
        fetch("/api/agent/runs/latest").then((response) => response.json()),
        loadAgentHistory(),
      ]);
    }

    renderAgentRun(latestPayload.run);
    if (historyPayload.ok) {
      renderAgentHistory(historyPayload.runs || []);
    } else {
      agentHistoryEl.textContent = historyPayload.error || "History unavailable";
    }
  } catch (err) {
    console.error(err);
    agentMetaEl.textContent = `Error: ${err.message || err}`;
    agentStepsEl.textContent = "";
    agentHistoryEl.textContent = "";
  }
}

async function invokeAgentPlan(prompt) {
  if (window.__TAURI__?.core?.invoke) {
    return window.__TAURI__.core.invoke("agent_plan", { prompt });
  }

  const response = await fetch("/api/agent/plan", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ prompt }),
  });
  return response.json();
}

async function invokeAgentExecute() {
  if (window.__TAURI__?.core?.invoke) {
    return window.__TAURI__.core.invoke("agent_execute");
  }

  const response = await fetch("/api/agent/execute", { method: "POST" });
  return response.json();
}

async function invokeAgentRetry() {
  if (window.__TAURI__?.core?.invoke) {
    return window.__TAURI__.core.invoke("agent_retry");
  }

  const response = await fetch("/api/agent/retry", { method: "POST" });
  return response.json();
}

async function loadAgentHistory() {
  const response = await fetch("/api/agent/runs");
  return response.json();
}

async function loadAgentRunById(id) {
  if (window.__TAURI__?.core?.invoke) {
    return window.__TAURI__.core.invoke("agent_run_get", { id });
  }

  const response = await fetch(`/api/agent/runs/${id}`);
  return response.json();
}

function renderAgentHistory(runs) {
  agentHistoryEl.innerHTML = "";
  if (!runs?.length) {
    agentHistoryEl.textContent = "No agent runs yet";
    return;
  }

  for (const run of runs.slice(0, 8)) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = `agent-history-item ${run.verification_passed ? "ok" : "bad"}`;
    button.textContent = `${run.id.slice(0, 8)}… · ${run.workflow_id || "plan-only"} · ${
      run.verification_passed
        ? verificationProfile(run.workflow_id).label
        : run.plan_only
          ? "plan"
          : "failed"
    }`;
    button.addEventListener("click", async () => {
      try {
        const payload = await loadAgentRunById(run.id);
        if (!payload.ok || !payload.run) {
          throw new Error(payload.error || "Run not found");
        }
        renderAgentRun(payload.run);
        activeProvenanceFilter = run.id;
        renderProvenance(window.__lastProvenanceEntries || [], activeProvenanceFilter);
      } catch (err) {
        console.error(err);
        agentMetaEl.textContent = `Error: ${err.message || err}`;
      }
    });
    agentHistoryEl.appendChild(button);
  }
}

function renderProvenance(entries, filterRunId = activeProvenanceFilter) {
  window.__lastProvenanceEntries = entries || [];
  provenanceEl.innerHTML = "";
  const filtered = filterRunId
    ? (entries || []).filter((entry) => entry.agent_run_id === filterRunId)
    : entries || [];

  if (!filtered?.length) {
    provenanceEl.textContent = filterRunId
      ? "No provenance entries for selected agent run"
      : "No provenance entries yet";
    return;
  }

  if (filterRunId) {
    const clear = document.createElement("button");
    clear.type = "button";
    clear.className = "secondary provenance-clear";
    clear.textContent = "Show all provenance";
    clear.addEventListener("click", () => {
      activeProvenanceFilter = null;
      renderProvenance(window.__lastProvenanceEntries || [], null);
    });
    provenanceEl.appendChild(clear);
  }

  for (const entry of filtered.slice().reverse().slice(0, 10)) {
    const card = document.createElement("article");
    card.className = "provenance-item";
    const header = document.createElement("div");
    header.className = "provenance-meta";
    header.textContent = `${entry.action} · ${entry.target} · ${entry.actor}`;
    card.appendChild(header);
    if (entry.agent_run_id) {
      const link = document.createElement("div");
      link.className = "provenance-agent";
      link.textContent = `agent run: ${entry.agent_run_id.slice(0, 8)}…`;
      card.appendChild(link);
    }
    provenanceEl.appendChild(card);
  }
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
    renderProvenance(payload.provenance || []);
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
    renderProvenance(payload.provenance || []);
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
    renderProvenance(payload.provenance || []);
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
    return window.__TAURI__.core.invoke("launch_gpu_preview", {
      workflowId: lastWorkflowId,
    });
  }

  const response = await fetch("/api/gpu-preview", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ workflow_id: lastWorkflowId }),
  });
  const payload = await response.json();
  if (!payload.ok) {
    throw new Error(payload.error || "GPU preview failed");
  }
  return payload.message || "GPU preview launched";
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
    lastWorkflowId = result.workflow_id;

    resolutionEl.textContent = [
      `workflow: ${result.workflow_id}`,
      `confidence: ${(result.confidence * 100).toFixed(0)}%`,
      `steps: ${result.workflow_steps}`,
      verificationLine(result.workflow_id, result.duckdb_verified),
    ].join("\n");

    datasetEl.textContent = result.dataset
      ? [
          `id: ${result.dataset.id}`,
          `title: ${result.dataset.title}`,
          `format: ${result.dataset.format?.kind ?? "—"}`,
          `crs: ${result.dataset.crs}`,
          `uri: ${result.dataset.uri}`,
          `license: ${result.dataset.license}`,
          result.stac_item ? `stac: ${result.stac_item.id}` : "",
        ]
            .filter(Boolean)
            .join("\n")
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
    const profile = verificationProfile(payload.run?.workflow_id);
    setStatus(payload.run?.verification_passed ? profile.status(true) : profile.status(false));
  } catch (err) {
    console.error(err);
    setStatus(`Execute error: ${err.message || err}`);
  } finally {
    agentExecuteBtn.disabled = false;
  }
});

agentRetryBtn?.addEventListener("click", async () => {
  agentRetryBtn.disabled = true;
  setStatus("Retrying agent verify…");
  try {
    const payload = await invokeAgentRetry();
    if (!payload.ok || !payload.run) {
      throw new Error(payload.error || "Agent retry failed");
    }
    renderAgentRun(payload.run);
    await loadComments();
    await loadAgentTrace();
    setStatus(
      payload.run.verification_passed
        ? verificationProfile(payload.run.workflow_id).status(true)
        : verificationProfile(payload.run.workflow_id).status(false),
    );
  } catch (err) {
    console.error(err);
    setStatus(`Retry error: ${err.message || err}`);
  } finally {
    agentRetryBtn.disabled = false;
  }
});

loadPlugins();
loadStacCollection();
loadComments();
loadAgentTrace();
runAsk();
