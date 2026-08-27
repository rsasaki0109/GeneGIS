const promptEl = document.getElementById("prompt");
const runBtn = document.getElementById("run-btn");
const downloadPngBtn = document.getElementById("download-png-btn");
const gpuPreviewBtn = document.getElementById("gpu-preview-btn");
const sceneOpenBtn = document.getElementById("scene-open-btn");
const sceneCopcPathEl = document.getElementById("scene-copc-path");
const sceneBuildingsPathEl = document.getElementById("scene-buildings-path");
const sceneCrsEl = document.getElementById("scene-crs");
const statusEl = document.getElementById("status");
const mapFrame = document.getElementById("map-frame");
const temporalPlaybackEl = document.getElementById("temporal-playback");
const temporalPlayBtn = document.getElementById("temporal-play-btn");
const temporalSliderEl = document.getElementById("temporal-slider");
const temporalMetaEl = document.getElementById("temporal-meta");
const temporalValuesEl = document.getElementById("temporal-values");
const composerTemplateEl = document.getElementById("composer-template");
const composerCreateBtn = document.getElementById("composer-create-btn");
const composerUndoBtn = document.getElementById("composer-undo-btn");
const composerRedoBtn = document.getElementById("composer-redo-btn");
const composerRunBtn = document.getElementById("composer-run-btn");
const composerGoalEl = document.getElementById("composer-goal");
const composerGoalBtn = document.getElementById("composer-goal-btn");
const composerSourceNodeEl = document.getElementById("composer-source-node");
const composerTargetNodeEl = document.getElementById("composer-target-node");
const composerConnectBtn = document.getElementById("composer-connect-btn");
const composerDisconnectBtn = document.getElementById("composer-disconnect-btn");
const composerNewNodeIdEl = document.getElementById("composer-new-node-id");
const composerAddNodeBtn = document.getElementById("composer-add-node-btn");
const composerStatusEl = document.getElementById("composer-status");
const composerGraphEl = document.getElementById("composer-graph");
const geocodingModeEl = document.getElementById("geocoding-mode");
const geocodingProviderEl = document.getElementById("geocoding-provider");
const geocodingEndpointEl = document.getElementById("geocoding-endpoint");
const geocodingQueriesEl = document.getElementById("geocoding-queries");
const geocodingPrivacyEl = document.getElementById("geocoding-privacy");
const geocodingRunBtn = document.getElementById("geocoding-run-btn");
const geocodingResultEl = document.getElementById("geocoding-result");
const narrativeTitleEl = document.getElementById("narrative-title");
const narrativeFrameTitleEl = document.getElementById("narrative-frame-title");
const narrativeTextEl = document.getElementById("narrative-text");
const narrativeCenterEl = document.getElementById("narrative-center");
const narrativeZoomEl = document.getElementById("narrative-zoom");
const narrativeMediaUriEl = document.getElementById("narrative-media-uri");
const narrativeMediaDigestEl = document.getElementById("narrative-media-digest");
const narrativeMediaAltEl = document.getElementById("narrative-media-alt");
const narrativeComposeBtn = document.getElementById("narrative-compose-btn");
const narrativeResultEl = document.getElementById("narrative-result");
const resolutionEl = document.getElementById("resolution");
const summaryEl = document.getElementById("summary");
const datasetEl = document.getElementById("dataset");
const dashboardEl = document.getElementById("dashboard");
const stacCollectionEl = document.getElementById("stac-collection");
const stacItemsEl = document.getElementById("stac-items");
const stacUrlEl = document.getElementById("stac-url");
const stacFetchBtn = document.getElementById("stac-fetch-btn");
const stacImportBtn = document.getElementById("stac-import-btn");
const stacFetchResultEl = document.getElementById("stac-fetch-result");
const stacOverlayEl = document.getElementById("stac-overlay");
const endpointFormEl = document.getElementById("endpoint-form");
const endpointIdEl = document.getElementById("endpoint-id");
const endpointUrlEl = document.getElementById("endpoint-url");
const endpointListEl = document.getElementById("endpoint-list");
const federatedBboxEl = document.getElementById("federated-bbox");
const federatedSearchBtn = document.getElementById("federated-search-btn");
const federatedSummaryEl = document.getElementById("federated-summary");
const federatedResultsEl = document.getElementById("federated-results");
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
let temporalPlayback = null;
let temporalTimer = null;
let composerSessionId = null;
let composerTemplateId = null;
let composerDocument = null;
let narrativeResultDigest = null;
let lastVerifiedDashboard = null;

async function sha256Text(value) {
  const bytes = new TextEncoder().encode(value);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return `sha256:${Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
}

async function composerRequest(path, body) {
  const response = await fetch(path, {
    method: body === undefined ? "GET" : "POST",
    headers: body === undefined ? {} : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const payload = await response.json();
  if (!payload.ok) throw new Error(payload.error || `Composer request failed (${response.status})`);
  return payload;
}

function composerOption(value, label) {
  const option = document.createElement("option");
  option.value = value;
  option.textContent = label;
  return option;
}

function renderComposer(payload) {
  composerSessionId = payload.session_id;
  composerTemplateId = payload.template_id;
  composerDocument = payload.composer;
  const workflow = composerDocument.workflow;
  composerGoalEl.value = workflow.goal;
  composerSourceNodeEl.replaceChildren();
  composerTargetNodeEl.replaceChildren();
  composerGraphEl.replaceChildren();

  for (const step of workflow.steps) {
    const label = `${step.stable_id} · ${step.operation}`;
    composerSourceNodeEl.append(composerOption(step.stable_id, label));
    composerTargetNodeEl.append(composerOption(step.stable_id, label));
    const node = document.createElement("article");
    node.className = "composer-node";
    const title = document.createElement("strong");
    title.textContent = step.operation;
    const id = document.createElement("code");
    id.textContent = step.stable_id;
    const edges = document.createElement("span");
    edges.textContent = step.depends_on?.length
      ? `← ${step.depends_on.map((item) => item).join(", ")}`
      : "graph input";
    const ports = document.createElement("small");
    ports.textContent = `out: ${(step.outputs || []).map((item) => item.port).join(", ") || "—"}`;
    node.append(title, id, edges, ports);
    composerGraphEl.append(node);
  }
  const lastEvent = composerDocument.events.at(-1);
  const executable = !lastEvent || Boolean(lastEvent.executable_digest);
  composerStatusEl.textContent = `${workflow.steps.length} nodes · ${workflow.input_contracts.length} typed inputs · ${executable ? "execution contracts valid" : "draft has validation errors"}`;
  for (const button of [composerGoalBtn, composerConnectBtn, composerDisconnectBtn, composerAddNodeBtn]) {
    button.disabled = false;
  }
  composerUndoBtn.disabled = composerDocument.events.length === 0;
  composerRedoBtn.disabled = false;
  composerRunBtn.disabled = !executable;
}

async function loadComposerTemplates() {
  if (!composerTemplateEl || window.__TAURI__?.core?.invoke) return;
  try {
    const payload = await composerRequest("/api/composer/templates");
    composerTemplateEl.replaceChildren(...payload.templates.map((template) =>
      composerOption(template.id, `${template.title} (${template.node_count})`)));
    composerStatusEl.textContent = `${payload.templates.length} reviewed templates available.`;
  } catch (error) {
    composerStatusEl.textContent = `Composer unavailable: ${error.message || error}`;
  }
}

async function createComposer() {
  const payload = await composerRequest("/api/composer/sessions", {
    template_id: composerTemplateEl.value,
  });
  renderComposer(payload);
}

async function editComposer(command) {
  if (!composerSessionId) throw new Error("Create a composer draft first");
  const payload = await composerRequest(`/api/composer/sessions/${composerSessionId}/edit`, { command });
  renderComposer(payload);
}

function selectedComposerPort() {
  const source = composerDocument?.workflow.steps.find((step) => step.stable_id === composerSourceNodeEl.value);
  return source?.outputs?.[0]?.port || "result";
}

function stopTemporalPlayback() {
  if (temporalTimer !== null) {
    window.clearInterval(temporalTimer);
    temporalTimer = null;
  }
  temporalPlayBtn.textContent = "Play";
}

function renderTemporalEpoch(index) {
  if (!temporalPlayback) return;
  const epoch = temporalPlayback.epochs[index];
  if (!epoch) return;
  temporalSliderEl.value = String(index);
  const receipt = epoch.encoding;
  temporalMetaEl.textContent = `${epoch.datetime} · ${epoch.id} · ${receipt.tile_count} MVT tiles · ${receipt.encoded_bytes} bytes · budget ${receipt.budget_passed ? "PASS" : "FAIL"}`;
  temporalValuesEl.replaceChildren();
  const maximum = Math.max(...epoch.values.map((item) => Math.abs(item.value)), 0.0001);
  for (const item of [...epoch.values].sort((left, right) => right.value - left.value)) {
    const row = document.createElement("div");
    row.className = "temporal-value";
    const label = document.createElement("span");
    label.textContent = item.label;
    const bar = document.createElement("span");
    bar.className = "temporal-value-bar";
    bar.style.setProperty("--value-width", `${Math.max(2, (Math.abs(item.value) / maximum) * 100)}%`);
    const value = document.createElement("strong");
    value.textContent = `${item.value.toFixed(3)} ${epoch.value_unit}`;
    row.append(label, bar, value);
    temporalValuesEl.append(row);
  }
}

function renderTemporalPlayback(playback) {
  stopTemporalPlayback();
  const valid = playback?.schema_version === "0.1.0"
    && playback.epochs?.length >= 2
    && playback.epochs.every((epoch) => epoch.encoding?.budget_passed === true);
  if (!valid) {
    temporalPlayback = null;
    temporalPlaybackEl.hidden = true;
    return;
  }
  temporalPlayback = playback;
  temporalPlaybackEl.hidden = false;
  temporalSliderEl.max = String(playback.epochs.length - 1);
  renderTemporalEpoch(0);
}

temporalSliderEl.addEventListener("input", () => {
  stopTemporalPlayback();
  renderTemporalEpoch(Number(temporalSliderEl.value));
});

temporalPlayBtn.addEventListener("click", () => {
  if (!temporalPlayback) return;
  if (temporalTimer !== null) {
    stopTemporalPlayback();
    return;
  }
  temporalPlayBtn.textContent = "Pause";
  temporalTimer = window.setInterval(() => {
    const next = (Number(temporalSliderEl.value) + 1) % temporalPlayback.epochs.length;
    renderTemporalEpoch(next);
  }, 1200);
});

function appendDashboardBar(container, label, value, maximum) {
  const row = document.createElement("div");
  row.className = "dashboard-bar-row";
  const text = document.createElement("span");
  text.textContent = `${label} · ${value}`;
  const track = document.createElement("div");
  track.className = "dashboard-bar-track";
  const fill = document.createElement("div");
  fill.className = "dashboard-bar-fill";
  fill.style.width = `${maximum > 0 ? (value / maximum) * 100 : 0}%`;
  track.appendChild(fill);
  row.append(text, track);
  container.appendChild(row);
}

function renderDashboard(dashboard) {
  if (!dashboardEl) return;
  dashboardEl.replaceChildren();
  if (!dashboard?.widgets?.length) {
    lastVerifiedDashboard = null;
    const empty = document.createElement("p");
    empty.className = "dashboard-empty";
    empty.textContent = "No verified dashboard widgets.";
    dashboardEl.appendChild(empty);
    return;
  }
  lastVerifiedDashboard = dashboard;

  const binding = document.createElement("div");
  binding.className = "dashboard-binding";
  binding.textContent = `verified ${dashboard.dashboard_digest.slice(0, 18)}… · result ${dashboard.result_digest.slice(0, 18)}…`;
  dashboardEl.appendChild(binding);

  for (const widget of dashboard.widgets) {
    const card = document.createElement("article");
    card.className = `dashboard-widget dashboard-${widget.kind}`;
    const title = document.createElement("h3");
    title.textContent = widget.label;
    card.appendChild(title);
    if (widget.kind === "kpi") {
      const value = document.createElement("strong");
      value.textContent = `${Number(widget.value).toLocaleString()} ${widget.unit}`;
      card.appendChild(value);
    } else {
      const entries = widget.kind === "histogram"
        ? widget.bins.map((bin) => [bin.label, bin.count])
        : widget.categories.map((category) => [category.category, category.count]);
      const maximum = Math.max(0, ...entries.map(([, value]) => value));
      for (const [label, value] of entries) {
        appendDashboardBar(card, label, value, maximum);
      }
    }
    dashboardEl.appendChild(card);
  }
}

function verificationProfile(workflowId) {
  if (workflowId === "remote-cog-demo" || workflowId === "local-cog-demo") {
    return {
      label: "cog metadata",
      verifier: "cog_metadata_verify",
      status: (passed) => (passed ? "COG metadata verified" : "COG metadata failed"),
    };
  }

  if (workflowId === "nagoya-geoparquet") {
    return {
      label: "geoparquet features",
      verifier: "geoparquet_feature_verify",
      status: (passed) => (passed ? "GeoParquet verified" : "GeoParquet failed"),
    };
  }

  if (workflowId === "external-stac-demo") {
    return {
      label: "stac collection",
      verifier: "stac_collection_verify",
      status: (passed) => (passed ? "STAC collection verified" : "STAC collection failed"),
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

async function loadStacOverlay() {
  try {
    const response = await fetch("/api/stac/overlay");
    const payload = await response.json();
    if (!payload.ok) {
      throw new Error(payload.error || "Failed to load STAC overlay");
    }

    stacOverlayEl.innerHTML = "";
    const records = payload.records || [];
    if (!records.length) {
      stacOverlayEl.textContent = "No imported STAC items";
      return;
    }

    for (const record of records) {
      const card = document.createElement("article");
      card.className = "stac-item";
      card.textContent = `${record.id} · ${record.format}`;
      card.title = record.uri || record.id;
      stacOverlayEl.appendChild(card);
    }
  } catch (err) {
    console.error(err);
    stacOverlayEl.textContent = `Error: ${err.message || err}`;
  }
}

async function loadStacEndpoints() {
  try {
    const response = await fetch("/api/stac/endpoints");
    const payload = await response.json();
    if (!payload.ok) {
      throw new Error(payload.error || "Failed to load endpoints");
    }
    endpointListEl.innerHTML = "";
    const endpoints = payload.endpoints || [];
    if (!endpoints.length) {
      endpointListEl.textContent = "No endpoints configured";
      return;
    }
    for (const endpoint of endpoints) {
      const row = document.createElement("div");
      row.className = "stac-item endpoint-row";

      const selected = document.createElement("input");
      selected.type = "checkbox";
      selected.name = "federated-endpoint";
      selected.value = endpoint.id;
      selected.checked = true;
      selected.setAttribute("aria-label", `Search ${endpoint.id}`);

      const label = document.createElement("span");
      label.textContent = `${endpoint.title} · ${endpoint.url}`;
      label.title = `${endpoint.id} · ${endpoint.authentication?.type || "anonymous"}`;

      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "secondary";
      remove.textContent = "×";
      remove.setAttribute("aria-label", `Remove ${endpoint.id}`);
      remove.addEventListener("click", () => removeStacEndpoint(endpoint.id));

      row.append(selected, label, remove);
      endpointListEl.appendChild(row);
    }
  } catch (err) {
    console.error(err);
    endpointListEl.textContent = `Error: ${err.message || err}`;
  }
}

async function addStacEndpoint(event) {
  event.preventDefault();
  const id = endpointIdEl.value.trim();
  const url = endpointUrlEl.value.trim();
  if (!id || !url) {
    setStatus("Endpoint ID and URL are required");
    return;
  }
  try {
    const response = await fetch("/api/stac/endpoints", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ id, title: id, url, auth_kind: "anonymous" }),
    });
    const payload = await response.json();
    if (!payload.ok) {
      throw new Error(payload.error || "Failed to save endpoint");
    }
    await loadStacEndpoints();
    setStatus(`Endpoint saved: ${id}`);
  } catch (err) {
    console.error(err);
    setStatus(`Endpoint error: ${err.message || err}`);
  }
}

async function removeStacEndpoint(id) {
  try {
    const response = await fetch("/api/stac/endpoints/remove", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ id }),
    });
    const payload = await response.json();
    if (!payload.ok) {
      throw new Error(payload.error || "Failed to remove endpoint");
    }
    await loadStacEndpoints();
    setStatus(`Endpoint removed: ${id}`);
  } catch (err) {
    console.error(err);
    setStatus(`Endpoint error: ${err.message || err}`);
  }
}

function parseFederatedBbox() {
  const values = federatedBboxEl.value
    .split(",")
    .map((value) => Number(value.trim()));
  if (values.length !== 4 || values.some((value) => !Number.isFinite(value))) {
    throw new Error("bbox must be MINX,MINY,MAXX,MAXY");
  }
  if (values[0] > values[2] || values[1] > values[3]) {
    throw new Error("bbox minimums must not exceed maximums");
  }
  return values;
}

async function searchFederatedStac() {
  federatedSearchBtn.disabled = true;
  federatedResultsEl.innerHTML = "";
  try {
    const endpointIds = Array.from(
      endpointListEl.querySelectorAll('input[name="federated-endpoint"]:checked'),
    ).map((input) => input.value);
    if (!endpointIds.length) {
      throw new Error("Select at least one endpoint");
    }
    const searchRequest = {
      endpoint_ids: endpointIds,
      bbox: parseFederatedBbox(),
      limit: 25,
    };
    const response = await fetch("/api/stac/search", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(searchRequest),
    });
    const payload = await response.json();
    if (!payload.ok || !payload.result) {
      throw new Error(payload.error || "Federated search failed");
    }
    const result = payload.result;
    const binding = payload.binding;
    const succeeded = result.endpoints.filter((endpoint) => !endpoint.error).length;
    federatedSummaryEl.textContent = binding
      ? `${result.items.length} items · ${succeeded}/${result.endpoints.length} endpoints · bound ${binding.selected.asset_key}`
      : `${result.items.length} items · ${succeeded}/${result.endpoints.length} endpoints · no verified GeoParquet`;
    if (binding) {
      const decision = document.createElement("article");
      decision.className = "stac-item";
      const passed = binding.selected.verifications.filter((check) => check.passed).length;
      decision.textContent =
        `Selected ${binding.selected.asset_key} · ${passed}/${binding.selected.verifications.length} checks · score ${binding.selected.score}`;
      decision.title = binding.selection_reason;
      if (/^https?:\/\//.test(binding.selected.href)) {
        const execute = document.createElement("button");
        execute.type = "button";
        execute.className = "secondary";
        execute.textContent = "Range Read + verify";
        execute.addEventListener("click", async () => {
          execute.disabled = true;
          execute.textContent = "Executing…";
          try {
            const executionResponse = await fetch("/api/stac/execute", {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify(searchRequest),
            });
            const receipt = await executionResponse.json();
            if (!receipt.ok || !receipt.verification?.passed) {
              throw new Error(receipt.error || "Execution verification failed");
            }
            execute.textContent =
              `Verified · ${receipt.execution.range_requests} ranges · ${receipt.execution.dataset.features.length} features`;
            setStatus("Federated GeoParquet verified");
          } catch (error) {
            execute.textContent = `Failed · ${error.message || error}`;
            execute.disabled = false;
          }
        });
        decision.appendChild(execute);
      }
      federatedResultsEl.appendChild(decision);
    }
    for (const resultItem of result.items) {
      const card = document.createElement("article");
      card.className = "stac-item";
      const title = resultItem.item.properties?.title || resultItem.item.id;
      card.textContent = `${title} · sources: ${resultItem.source_endpoints.join(", ")}`;
      card.title = JSON.stringify(resultItem.item.assets || {});
      federatedResultsEl.appendChild(card);
    }
    setStatus(`Federated search: ${result.items.length} items`);
  } catch (err) {
    console.error(err);
    federatedSummaryEl.textContent = `Error: ${err.message || err}`;
    setStatus(`Federated search error: ${err.message || err}`);
  } finally {
    federatedSearchBtn.disabled = false;
  }
}

async function fetchExternalStac() {
  const url = stacUrlEl.value.trim();
  if (!url) {
    setStatus("STAC URL is empty");
    return;
  }

  setStatus("Fetching STAC…", true);
  try {
    const response = await fetch("/api/stac/fetch", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ url }),
    });
    const payload = await response.json();
    if (!payload.ok || !payload.collection) {
      throw new Error(payload.error || "STAC fetch failed");
    }
    stacFetchResultEl.textContent = JSON.stringify(payload.collection, null, 2);
    setStatus(`STAC fetched: ${payload.collection.id}`);
  } catch (err) {
    console.error(err);
    stacFetchResultEl.textContent = `Error: ${err.message || err}`;
    setStatus(`STAC fetch error: ${err.message || err}`);
  } finally {
    setStatus("Ready");
  }
}

async function importExternalStac() {
  const url = stacUrlEl.value.trim();
  if (!url) {
    setStatus("STAC URL is empty");
    return;
  }

  setStatus("Importing STAC item…", true);
  try {
    const response = await fetch("/api/stac/import", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ url }),
    });
    const payload = await response.json();
    if (!payload.ok || !payload.record) {
      throw new Error(payload.error || "STAC import failed");
    }
    stacFetchResultEl.textContent = JSON.stringify(payload.record, null, 2);
    await loadStacOverlay();
    await loadStacCollection();
    setStatus(`STAC imported: ${payload.record.id}`);
  } catch (err) {
    console.error(err);
    stacFetchResultEl.textContent = `Error: ${err.message || err}`;
    setStatus(`STAC import error: ${err.message || err}`);
  } finally {
    setStatus("Ready");
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
    const message = await window.__TAURI__.core.invoke("launch_gpu_preview", {
      workflowId: lastWorkflowId,
    });
    return { message, dashboard: null };
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
  return payload;
}

async function openGpuPreview() {
  setStatus("Launching GPU map…", true);
  try {
    const payload = await invokeGpuPreview();
    if (payload.dashboard) renderDashboard(payload.dashboard);
    setStatus(payload.message || "GPU preview launched");
  } catch (err) {
    console.error(err);
    setStatus(`Error: ${err.message || err}`);
  }
}

async function openScene3d() {
  const copcPath = sceneCopcPathEl?.value.trim();
  const buildingsPath = sceneBuildingsPathEl?.value.trim();
  const crs = sceneCrsEl?.value.trim();
  if (!copcPath || !buildingsPath || !crs) {
    setStatus("COPC path, LOD1 path, and projected CRS are required");
    return;
  }
  if (window.__TAURI__?.core?.invoke) {
    setStatus("Use the local web workbench for path-backed 3D scenes");
    return;
  }
  setStatus("Validating and launching 3D scene…", true);
  try {
    const response = await fetch("/api/gpu-preview", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        workflow_id: "scene3d-copc-lod1",
        copc_path: copcPath,
        buildings_path: buildingsPath,
        crs,
      }),
    });
    const payload = await response.json();
    if (!payload.ok) throw new Error(payload.error || "3D scene failed");
    renderDashboard(payload.dashboard);
    setStatus(payload.message || "3D scene launched");
  } catch (err) {
    console.error(err);
    setStatus(`Error: ${err.message || err}`);
  }
}

function renderAskResult(result) {
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
      ].filter(Boolean).join("\n")
    : "—";
  summaryEl.textContent = JSON.stringify(result.summary, null, 2);
  renderTemporalPlayback(result.summary?.temporal_playback);
  renderVerification(result.verification.checks);
  renderNotes(result.ambiguities);
  mapFrame.srcdoc = result.html;
  setPngExport(result.png_base64);
  setPipelineReady(true);
  narrativeResultDigest = result.execution_receipt?.verification_passed
    ? result.execution_receipt.result_digest
    : null;
  narrativeComposeBtn.disabled = !narrativeResultDigest;
  narrativeResultEl.textContent = narrativeResultDigest
    ? `Ready · verified result ${narrativeResultDigest}`
    : "Result is not verified for narrative composition.";
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
    renderAskResult(result);
    setStatus("Done");
    loadAgentTrace();
  } catch (err) {
    console.error(err);
    setStatus(`Error: ${err.message || err}`);
  }
}

composerCreateBtn?.addEventListener("click", async () => {
  try {
    composerStatusEl.textContent = "Creating typed workflow draft…";
    await createComposer();
  } catch (error) {
    composerStatusEl.textContent = `Create failed: ${error.message || error}`;
  }
});
composerGoalBtn?.addEventListener("click", async () => {
  try {
    await editComposer({ kind: "set_goal", goal: composerGoalEl.value });
  } catch (error) {
    composerStatusEl.textContent = `Edit failed: ${error.message || error}`;
  }
});
composerUndoBtn?.addEventListener("click", async () => {
  try { await editComposer({ kind: "undo" }); }
  catch (error) { composerStatusEl.textContent = `Undo failed: ${error.message || error}`; }
});
composerRedoBtn?.addEventListener("click", async () => {
  try { await editComposer({ kind: "redo" }); }
  catch (error) { composerStatusEl.textContent = `Redo failed: ${error.message || error}`; }
});
composerConnectBtn?.addEventListener("click", async () => {
  try {
    await editComposer({
      kind: "connect",
      source_node_id: composerSourceNodeEl.value,
      source_port: selectedComposerPort(),
      target_node_id: composerTargetNodeEl.value,
    });
  } catch (error) {
    composerStatusEl.textContent = `Connect failed: ${error.message || error}`;
  }
});
composerDisconnectBtn?.addEventListener("click", async () => {
  try {
    await editComposer({
      kind: "disconnect",
      source_node_id: composerSourceNodeEl.value,
      target_node_id: composerTargetNodeEl.value,
    });
  } catch (error) {
    composerStatusEl.textContent = `Disconnect failed: ${error.message || error}`;
  }
});
composerAddNodeBtn?.addEventListener("click", async () => {
  try {
    await editComposer({
      kind: "add_reviewed_node",
      template_id: composerTemplateId,
      source_node_id: composerSourceNodeEl.value,
      new_node_id: composerNewNodeIdEl.value.trim(),
    });
  } catch (error) {
    composerStatusEl.textContent = `Add node failed: ${error.message || error}`;
  }
});
composerRunBtn?.addEventListener("click", async () => {
  if (!composerSessionId) return;
  composerRunBtn.disabled = true;
  composerStatusEl.textContent = "Dispatching reviewed RunWorkflow…";
  try {
    const payload = await composerRequest(`/api/composer/sessions/${composerSessionId}/run`, {});
    renderAskResult(payload.result);
    composerStatusEl.textContent = `Executed command ${payload.command.id} · workflow digest matched`;
    setStatus("Composer workflow verified");
  } catch (error) {
    composerStatusEl.textContent = `Run blocked: ${error.message || error}`;
  } finally {
    composerRunBtn.disabled = false;
  }
});

geocodingRunBtn?.addEventListener("click", async () => {
  const texts = geocodingQueriesEl.value.split(/\r?\n/).map((value) => value.trim()).filter(Boolean);
  if (geocodingModeEl.value === "interactive" && texts.length !== 1) {
    geocodingResultEl.textContent = "Interactive mode requires exactly one query.";
    return;
  }
  const isHttp = geocodingProviderEl.value === "http_json";
  const provider = isHttp
    ? { kind: "http_json", provider_id: "workbench.http", version: "1", endpoint: geocodingEndpointEl.value.trim() }
    : { kind: "offline_nagoya" };
  geocodingRunBtn.disabled = true;
  geocodingResultEl.textContent = "Running admitted Command + Workflow…";
  try {
    const response = await fetch("/api/geocode", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        mode: geocodingModeEl.value,
        queries: texts.map((text, index) => ({ id: `q${index + 1}`, text })),
        language: "ja",
        max_candidates: 3,
        provider,
        privacy: geocodingPrivacyEl.value,
      }),
    });
    const payload = await response.json();
    if (!payload.ok) throw new Error(payload.error || `Geocoding failed (${response.status})`);
    const result = payload.result;
    geocodingResultEl.textContent = JSON.stringify({
      command_id: result.command_id,
      workflow_digest: result.workflow_digest,
      result_digest: result.result_digest,
      crs: result.receipt.crs,
      provider: result.receipt.provider_id,
      policy_digest: result.receipt.policy_digest,
      results: result.results,
    }, null, 2);
  } catch (error) {
    geocodingResultEl.textContent = `Blocked: ${error.message || error}`;
  } finally {
    geocodingRunBtn.disabled = false;
  }
});

narrativeComposeBtn?.addEventListener("click", async () => {
  if (!narrativeResultDigest) return;
  const center = narrativeCenterEl.value.split(",").map(Number);
  if (center.length !== 2 || center.some((value) => !Number.isFinite(value))) {
    narrativeResultEl.textContent = "Center must be lon,lat.";
    return;
  }
  const styleDigest = await sha256Text(JSON.stringify({
    layer: "verified-result",
    portrayal: "current-workbench-map",
  }));
  const media = [];
  if (narrativeMediaUriEl.value.trim()) {
    media.push({
      uri: narrativeMediaUriEl.value.trim(),
      content_digest: narrativeMediaDigestEl.value.trim(),
      media_type: "image/*",
      alt_text: narrativeMediaAltEl.value.trim(),
    });
  }
  const dashboard = lastVerifiedDashboard?.result_digest === narrativeResultDigest
    ? { dashboard_digest: lastVerifiedDashboard.dashboard_digest, result_digest: narrativeResultDigest }
    : null;
  narrativeComposeBtn.disabled = true;
  narrativeResultEl.textContent = "Sealing narrative through Command + Workflow…";
  try {
    const response = await fetch("/api/narratives/compose", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        title: narrativeTitleEl.value.trim(),
        result_digest: narrativeResultDigest,
        frames: [{
          id: "frame-1",
          title: narrativeFrameTitleEl.value.trim(),
          text: narrativeTextEl.value.trim(),
          map: {
            center,
            zoom: Number(narrativeZoomEl.value),
            bearing: 0,
            pitch: 0,
            temporal_cursor: null,
            layers: [{
              layer_id: "verified-result",
              visible: true,
              opacity: 1,
              result_digest: narrativeResultDigest,
              style_digest: styleDigest,
            }],
          },
          media,
          dashboard,
        }],
      }),
    });
    const payload = await response.json();
    if (!payload.ok) throw new Error(payload.error || `Narrative failed (${response.status})`);
    narrativeResultEl.textContent = JSON.stringify({
      command_id: payload.receipt.command_id,
      workflow_digest: payload.receipt.workflow_digest,
      view_digest: payload.receipt.view.view_digest,
      frame_count: payload.receipt.view.frames.length,
      screenshot_copies: 0,
    }, null, 2);
  } catch (error) {
    narrativeResultEl.textContent = `Blocked: ${error.message || error}`;
  } finally {
    narrativeComposeBtn.disabled = !narrativeResultDigest;
  }
});

runBtn.addEventListener("click", runAsk);
downloadPngBtn.addEventListener("click", downloadPng);
gpuPreviewBtn.addEventListener("click", openGpuPreview);
sceneOpenBtn?.addEventListener("click", openScene3d);
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
loadStacOverlay();
loadStacEndpoints();
loadComposerTemplates();
stacFetchBtn?.addEventListener("click", fetchExternalStac);
stacImportBtn?.addEventListener("click", importExternalStac);
endpointFormEl?.addEventListener("submit", addStacEndpoint);
federatedSearchBtn?.addEventListener("click", searchFederatedStac);
loadComments();
loadAgentTrace();
runAsk();
