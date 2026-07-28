"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

const DEMO_ID = "nagoya-density";
const DEFAULT_PROMPT = "名古屋市の人口密度を表示";
const ENGLISH_PROMPT = "Show population density in Nagoya";

type WorkflowStep = {
  id: string;
  operation: string;
  parameters: Record<string, unknown>;
};

type VerificationCheck = {
  name: string;
  passed: boolean;
  detail: string;
};

type Citation = {
  title: string;
  url?: string;
  license?: string;
};

type ProvenanceEntry = {
  id: string;
  timestamp: string;
  actor: string;
  action: string;
  target: string;
  workflow_id?: string;
  details: {
    command_id: string;
    source_uri: string;
    source_id: string;
    stac_item_id: string;
    crs: string;
    units: string;
    license: string;
    verifier: string;
    verification_passed: boolean;
  };
};

type DemoBundle = {
  schema_version: number;
  demo_id: string;
  generated_by: string;
  execution_mode: "verified_replay";
  prompt_aliases: string[];
  map_asset: string;
  command: {
    id: string;
    origin: string;
    timestamp: string;
    command: {
      type: "run_workflow";
      workflow_id: string;
    };
  };
  workflow: {
    id: string;
    goal: string;
    assumptions: string[];
    steps: WorkflowStep[];
    citations: Citation[];
    review_status: string;
  };
  provenance: {
    entries: ProvenanceEntry[];
  };
  dataset: {
    id: string;
    title: string;
    description: string;
    format: { kind: string; media_type: string };
    crs: string;
    uri: string;
    license: string;
  };
  verification: {
    crs: string;
    area_method: string;
    density_unit: string;
    checks: VerificationCheck[];
  };
  summary: {
    ward_count: number;
    top_density_ward?: {
      ward_name: string;
      density_per_km2: number;
    };
  };
  confidence: number;
};

type RunState = "idle" | "loading" | "running" | "complete" | "error";

function promptIsSupported(prompt: string) {
  const normalized = prompt.trim().toLowerCase();
  return (
    normalized === DEFAULT_PROMPT.toLowerCase() ||
    normalized === ENGLISH_PROMPT.toLowerCase() ||
    (normalized.includes("名古屋") && normalized.includes("人口密度")) ||
    (normalized.includes("nagoya") && normalized.includes("density"))
  );
}

function shortId(value: string) {
  return `${value.slice(0, 8)}…`;
}

export default function Playground() {
  const [prompt, setPrompt] = useState(DEFAULT_PROMPT);
  const [bundle, setBundle] = useState<DemoBundle | null>(null);
  const [runState, setRunState] = useState<RunState>("idle");
  const [activeStep, setActiveStep] = useState(-1);
  const [message, setMessage] = useState("Ready — no API key required");
  const [shareLabel, setShareLabel] = useState("Copy replay link");
  const hasReadUrl = useRef(false);

  const runDemo = useCallback(async (requestedPrompt = prompt, updateUrl = true) => {
    if (!promptIsSupported(requestedPrompt)) {
      setRunState("error");
      setMessage("Public alpha supports the Nagoya density sample. Choose a sample prompt.");
      return;
    }

    setPrompt(requestedPrompt);
    setRunState("loading");
    setActiveStep(-1);
    setMessage("Loading Rust-generated execution receipt…");

    try {
      const response = await fetch("/demo/nagoya-density.json");
      if (!response.ok) {
        throw new Error(`bundle request failed (${response.status})`);
      }
      const nextBundle = (await response.json()) as DemoBundle;
      if (
        nextBundle.command.command.workflow_id !== nextBundle.workflow.id ||
        nextBundle.provenance.entries[0]?.workflow_id !== nextBundle.workflow.id
      ) {
        throw new Error("execution receipt is internally inconsistent");
      }

      setBundle(nextBundle);
      setRunState("running");
      setMessage("Replaying verified Command + Workflow Graph…");

      for (let index = 0; index < nextBundle.workflow.steps.length; index += 1) {
        await new Promise((resolve) => window.setTimeout(resolve, 55));
        setActiveStep(index);
      }

      setRunState("complete");
      setMessage("Verified · 16 wards · sources attached");
      if (updateUrl) {
        const url = new URL(window.location.href);
        url.searchParams.set("demo", DEMO_ID);
        window.history.replaceState({}, "", url);
      }
    } catch (error) {
      setRunState("error");
      setMessage(error instanceof Error ? error.message : "Unable to load demo");
    }
  }, [prompt]);

  useEffect(() => {
    if (hasReadUrl.current) return;
    hasReadUrl.current = true;
    const params = new URLSearchParams(window.location.search);
    if (params.get("demo") === DEMO_ID) {
      void runDemo(DEFAULT_PROMPT, false);
    }
  }, [runDemo]);

  const allChecksPassed = useMemo(
    () => bundle?.verification.checks.every((check) => check.passed) ?? false,
    [bundle],
  );

  async function copyShareLink() {
    const url = new URL(window.location.href);
    url.searchParams.set("demo", DEMO_ID);
    try {
      await navigator.clipboard.writeText(url.toString());
      setShareLabel("Copied!");
    } catch {
      window.prompt("Copy this replay URL", url.toString());
      setShareLabel("Link ready");
    }
    window.setTimeout(() => setShareLabel("Copy replay link"), 1600);
  }

  function downloadBundle() {
    if (!bundle) return;
    const blob = new Blob([JSON.stringify(bundle, null, 2)], { type: "application/json" });
    const link = document.createElement("a");
    link.href = URL.createObjectURL(blob);
    link.download = "genegis-nagoya-density-receipt.json";
    link.click();
    URL.revokeObjectURL(link.href);
  }

  return (
    <main>
      <header className="topbar">
        <a className="wordmark" href="/" aria-label="GeneGIS Playground home">
          <span className="brand-mark" aria-hidden="true">G</span>
          <span>GeneGIS</span>
          <span className="alpha-tag">PUBLIC ALPHA</span>
        </a>
        <nav>
          <a href="#workflow">Workflow</a>
          <a href="#provenance">Provenance</a>
          <a href="https://github.com/rsasaki0109/GeneGIS" target="_blank" rel="noreferrer">
            GitHub ↗
          </a>
        </nav>
      </header>

      <section className="hero">
        <div className="hero-copy">
          <p className="eyebrow"><span /> VERIFIED SPATIAL COMPUTING</p>
          <h1>Ask for a map.<br /><em>Get the proof.</em></h1>
          <p className="hero-lede">
            GeneGIS turns intent into an inspectable workflow, executes it through a typed
            Command, and returns a verified map with every source attached.
          </p>
          <div className="hero-facts" aria-label="Playground facts">
            <span>NO INSTALL</span><span>NO API KEY</span><span>RUST CORE</span>
          </div>
        </div>

        <div className="prompt-card">
          <div className="prompt-card-head">
            <span>01 / INTENT</span>
            <span className={`live-dot ${runState}`}><i /> {runState === "complete" ? "VERIFIED" : "READY"}</span>
          </div>
          <label htmlFor="prompt">Describe the spatial result you need</label>
          <textarea
            id="prompt"
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
            rows={3}
            spellCheck={false}
          />
          <div className="sample-row">
            <span>TRY</span>
            <button type="button" onClick={() => setPrompt(DEFAULT_PROMPT)}>日本語</button>
            <button type="button" onClick={() => setPrompt(ENGLISH_PROMPT)}>English</button>
          </div>
          <button
            className="run-button"
            type="button"
            onClick={() => void runDemo()}
            disabled={runState === "loading" || runState === "running"}
          >
            <span>{runState === "running" ? "Executing workflow" : "Run verified demo"}</span>
            <b>{runState === "running" ? "···" : "→"}</b>
          </button>
          <p className={`run-status ${runState}`} role="status"><span />{message}</p>
        </div>
      </section>

      <section className={`result-shell ${bundle ? "visible" : ""}`} aria-live="polite">
        <div className="result-heading">
          <div>
            <p className="section-number">02 / RESULT</p>
            <h2>Population density,<br />Nagoya’s 16 wards</h2>
          </div>
          {bundle && (
            <div className="result-actions">
              <button type="button" onClick={copyShareLink}>{shareLabel}</button>
              <button type="button" onClick={downloadBundle}>Download receipt</button>
            </div>
          )}
        </div>

        <div className="result-grid">
          <article className="map-card">
            {bundle ? (
              <>
                <img src={bundle.map_asset} alt="Choropleth map of population density across Nagoya's 16 wards" />
                <div className="map-caption">
                  <span>EPSG:4326</span>
                  <span>{bundle.verification.density_unit}</span>
                  <span>16 FEATURES</span>
                </div>
              </>
            ) : (
              <div className="map-placeholder">
                <span>Run the sample to render a verified map</span>
              </div>
            )}
          </article>

          <aside className="metrics-card">
            <p className="card-kicker">VERIFICATION SUMMARY</p>
            <div className="primary-metric">
              <strong>{bundle ? `${bundle.summary.ward_count} wards` : "—"}</strong>
              <span>analyzed and rendered</span>
            </div>
            <div className="density-value">
              <strong>
                {bundle
                  ? `${bundle.verification.checks.filter((check) => check.passed).length} / ${bundle.verification.checks.length}`
                  : "—"}
              </strong>
              <span>verification checks passed</span>
            </div>
            <dl>
              <div><dt>CRS</dt><dd>{bundle?.verification.crs ?? "—"}</dd></div>
              <div><dt>Area method</dt><dd>{bundle?.verification.area_method ?? "—"}</dd></div>
              <div><dt>Dataset</dt><dd>{bundle?.dataset.format.kind ?? "—"}</dd></div>
              <div><dt>Confidence</dt><dd>{bundle ? `${Math.round(bundle.confidence * 100)}%` : "—"}</dd></div>
            </dl>
            <div className={`verification-seal ${allChecksPassed ? "passed" : ""}`}>
              <span>{allChecksPassed ? "✓" : "○"}</span>
              <div><strong>{allChecksPassed ? "All checks passed" : "Awaiting execution"}</strong>
                <small>{bundle?.verification.checks.length ?? 0} machine-readable checks</small></div>
            </div>
          </aside>
        </div>
      </section>

      <section className="workflow-section" id="workflow">
        <div className="section-heading">
          <div>
            <p className="section-number">03 / WORKFLOW GRAPH</p>
            <h2>Every operation is inspectable.</h2>
          </div>
          {bundle && (
            <div className="command-pill">
              <span>COMMAND</span>
              <code>RunWorkflow({shortId(bundle.workflow.id)})</code>
            </div>
          )}
        </div>
        <div className="workflow-track">
          {(bundle?.workflow.steps ?? []).map((step, index) => (
            <div className={`workflow-node ${index <= activeStep ? "done" : ""}`} key={step.id}>
              <span>{String(index + 1).padStart(2, "0")}</span>
              <strong>{step.operation.replace(/([a-z])([A-Z])/g, "$1 $2")}</strong>
              <i>{index <= activeStep ? "✓" : ""}</i>
            </div>
          ))}
          {!bundle && <p className="empty-note">The Rust-generated GeoWorkflow appears here after execution.</p>}
        </div>
      </section>

      <section className="proof-section" id="provenance">
        <div className="section-heading">
          <div>
            <p className="section-number">04 / PROOF</p>
            <h2>Sources, units, and decisions—attached.</h2>
          </div>
          <p className="proof-intro">
            A result without provenance is just a picture. GeneGIS ships the evidence with the map.
          </p>
        </div>

        <div className="proof-grid">
          <article>
            <p className="card-kicker">VALIDATION</p>
            <ul className="check-list">
              {(bundle?.verification.checks ?? []).map((check) => (
                <li key={check.name}><span>✓</span><div><strong>{check.name.replaceAll("_", " ")}</strong><small>{check.detail}</small></div></li>
              ))}
              {!bundle && <li className="empty-note">Run the demo to inspect checks.</li>}
            </ul>
          </article>

          <article>
            <p className="card-kicker">DATA SOURCES</p>
            <ul className="source-list">
              {(bundle?.workflow.citations ?? []).map((citation, index) => (
                <li key={`${citation.title}-${index}`}>
                  <span>{String(index + 1).padStart(2, "0")}</span>
                  <div><strong>{citation.title}</strong><small>{citation.license}</small></div>
                  {citation.url && <a href={citation.url} target="_blank" rel="noreferrer" aria-label={`Open ${citation.title}`}>↗</a>}
                </li>
              ))}
              {!bundle && <li className="empty-note">Source citations appear with the result.</li>}
            </ul>
          </article>

          <article className="receipt-card">
            <p className="card-kicker">EXECUTION RECEIPT</p>
            {bundle ? (
              <>
                <dl>
                  <div><dt>Command</dt><dd>{shortId(bundle.command.id)}</dd></div>
                  <div><dt>Workflow</dt><dd>{shortId(bundle.workflow.id)}</dd></div>
                  <div><dt>Origin</dt><dd>{bundle.command.origin}</dd></div>
                  <div><dt>Source</dt><dd>{bundle.dataset.id}</dd></div>
                  <div><dt>License</dt><dd>{bundle.dataset.license}</dd></div>
                  <div><dt>Mode</dt><dd>{bundle.execution_mode.replace("_", " ")}</dd></div>
                </dl>
                <p className="receipt-note">
                  Generated by the GeneGIS Rust pipeline. The public site replays the committed,
                  verified artifact—no model or API key required.
                </p>
              </>
            ) : <p className="empty-note">Command and provenance IDs appear after execution.</p>}
          </article>
        </div>
      </section>

      <footer>
        <div>
          <span className="brand-mark">G</span>
          <p><strong>GeneGIS</strong><br />GIS designed for agents and humans to collaborate.</p>
        </div>
        <a href="https://github.com/rsasaki0109/GeneGIS" target="_blank" rel="noreferrer">
          Read the code. Inspect the workflow. Star the idea. ↗
        </a>
      </footer>
    </main>
  );
}
