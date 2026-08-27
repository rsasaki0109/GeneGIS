# Phase 14: Web GIS Platform Parity

**Status:** Complete (M0–M5; 2026-08-27).

**Goal:** Reach user-facing capability parity with the reference commercial
web-GIS platform — 3D, live dashboards, OGC service layers, time slider,
publishing, and a desktop-GIS bridge — while keeping every result
provenance-bound and verified.

Reference capability analysis and milestone table:
[`docs/adr/0016-web-gis-platform-parity-track.md`](../adr/0016-web-gis-platform-parity-track.md).

## Milestones

| ID | Deliverable | Exit evidence | Status |
| --- | --- | --- | --- |
| `M0` | 3D district showcase GIF | README GIF: point-cloud terrain + LOD1 buildings + roads + POI dashboard for a Japanese suburban district; frames regenerable via one script | Complete (2026-08-26) |
| `M1` | 3D viewer (WebGPU) | Orbit camera over COPC points + extruded buildings with real heights in the workbench; first-frame/FPS budgets under the shared I/O receipt model | Complete (2026-08-27) |
| `M2` | Live dashboards | Chart widgets driven by verified workflow results (building-height histogram, POI category breakdown, KPI counters); digest-bound to the executed workflow | Complete (2026-08-26) |
| `M3` | WMS/WFS clients | GetCapabilities/GetMap and GetFeature through adapter manifest with I/O receipts; positive + negative admission tests | Complete (2026-08-26) |
| `M4` | Time slider + layer streaming | Epoch playback widget over temporal layers; per-layer tile encoding budget evidence | Complete (2026-08-26) |
| `M5` | Publishing + desktop bridge | Share-link/embeddable viewer export; plugin pushing desktop-GIS layers into a GeneGIS project capsule | Complete (2026-08-26) |

## M0 first slice (complete)

Recreate the reference 3D district demo end to end as a deterministic,
offline-safe Japanese suburban fixture:

1. **Terrain** — seeded point-cloud grid in a local metric CRS.
2. **Buildings** — seeded LOD1 footprints with generated metric heights,
   extruded in renderer space.
3. **Roads** — deterministic local road grid.
4. **POI** — six deterministic categorized points.
5. **Dashboard strip** — height-band histogram, POI category counts, total
   building count rendered into each frame.
6. **GIF** — orbit-camera frame sequence assembled by
   `scripts/build-district3d-gif.sh` into `docs/assets/district3d.gif`.

Constraints:

- No competitor names anywhere in the repository or generated assets.
- All data sources recorded as citations/provenance in the result capsule.
- Frames must be regenerable offline from pinned fixtures (CI-safe), with an
  optional real-download path behind `GENEGIS_*_PATH` overrides like the
  existing real-data showcase.

The optional real-data override remains non-gating follow-up work. Any future
override must identify every source, CRS, unit, and checksum without changing
the accepted offline fixture path.

## M1 implementation status

The native renderer now has a versioned `Scene3d` admission contract and a
depth-tested WebGPU path for COPC points and LOD1 building extrusion. A scene
is rejected unless it declares a known projected CRS, metre horizontal and
vertical units, immutable point/building source snapshots, and a resolvable
height source for every building. Left-drag orbits the camera and the mouse
wheel changes orbit radius.

The workbench `POST /api/gpu-preview` route accepts
`workflow_id=scene3d-copc-lod1`, local COPC and measured-height building paths,
and an EPSG identifier. It executes a validated `RunWorkflow` command with two
input snapshots before opening the native scene. Direct-render shortcuts are
not accepted for this path.

`Scene3dBenchmark` maps directly into `IoReceipt.gpu`; the shared budget now
requires first frame at or below 2 seconds and steady-state rendering at or
above 30 FPS. The committed acceptance scene now contains 40,949 deterministic
COPC 1.0 points and three LOD1 buildings in EPSG:6675 metres. Its pinned Pixi /
PDAL 2.9.3 pipeline reproduces the exact COPC digest on consecutive runs.

`scripts/run-gpu-scene-acceptance.ps1` executes fixture admission, rendering,
budget evaluation, and receipt sealing through the existing Scene3D Command +
Workflow Graph. An outer process timeout prevents a blocked driver call from
creating a receipt. Backend selection now honors `WGPU_BACKEND`, and native
completion callbacks are driven by bounded `Device::poll` calls. Twenty
optimized DX12 runs on one digest-bound release executable and an NVIDIA
GeForce GTX 1660 Ti were independently reverified after JSON persistence. The
nearest-rank p95 first frame is 1.1180932 seconds and the minimum steady-state
rate is 555.3010 FPS over 120 measured frames per run. The canonical receipt is
[`docs/reports/phase-14-m1-gpu-hardware-receipt.json`](../reports/phase-14-m1-gpu-hardware-receipt.json),
and the immutable sample index is
[`docs/reports/phase-14-m1-gpu-sample-set.json`](../reports/phase-14-m1-gpu-sample-set.json).

## M2 live dashboards (complete)

`LiveDashboard` is generated only after the scene result digest is recomputed
and matched. Its binding includes the executed workflow digest, scene result
digest, and source snapshots. The dashboard has three KPI widgets, a fixed
metric building-height histogram, and a deterministic POI-category breakdown.
Offline verification recomputes the source binding, every widget value, and
the dashboard digest; source, widget, or result tampering fails closed.

The browser workbench exposes a local COPC + LOD1 launcher and renders the
verified widgets next to the map. Exit evidence is recorded in
[`docs/reports/phase-14-m2-live-dashboard.json`](../reports/phase-14-m2-live-dashboard.json).

## M3 OGC service clients (complete)

The reviewed `org.genegis.ogc-web-service` adapter admits only read-only
network operations for WMS 1.3.0 and WFS 2.0.0. Typed requests construct
encoded `GetCapabilities`, `GetMap`, and `GetFeature` URLs; the remote host
allowlist, timeout, redirect, and response-size policies are applied before
response parsing. Service exception documents, unexpected content types,
invalid image signatures, invalid GeoJSON, unknown CRS, and malformed extents
fail closed.

Every successful response carries the manifest admission report, source
snapshot and observed checksum, CRS and coordinate unit where applicable,
response digest, byte/item counts, request selection, and timing in the shared
`IoReceipt`. The workbench `POST /api/ogc/execute` route dispatches a
`RunWorkflow` command through the four-step admission, request, validation,
and receipt graph; it does not call the transport as an untracked UI shortcut.

Exit evidence is recorded in
[`docs/reports/phase-14-m3-ogc-service-clients.json`](../reports/phase-14-m3-ogc-service-clients.json).

## M4 time slider and layer streaming (complete)

Verified NDVI time-series results now include a `TemporalPlayback` document
bound to the workflow digest, result digest, CRS/unit contract, and command
source snapshots. Each strictly ordered epoch is encoded as actual MVT payloads
at the first-slice stream zoom. Its receipt records tile count, total and
largest encoded bytes, encoding time, tile-set digest, and the exact limits
used to make the pass/fail decision.

The workbench shows the temporal control only when at least two epochs are
present and every layer passed its encoding budget. Manual scrubbing and
play/pause cycle through the verified ward values without generating an
untracked result in the browser. A changed value, epoch order, budget result,
or binding digest fails offline verification.

Exit evidence is recorded in
[`docs/reports/phase-14-m4-temporal-playback.json`](../reports/phase-14-m4-temporal-playback.json).

## M5 portable publishing and desktop bridge (complete)

Portable publishing begins from an offline-verified result capsule. The
exporter emits a full share view, minimal embed view, allowlisted
`publication.json`, and a downloadable source capsule. The publication binds
the result, workflow, verification graph, policy, source capsule, attribution,
limitations, CRS/units, and explicit redaction policy into one semantic
digest. Public HTML embeds the verified PNG and excludes local paths,
credentials, prompts, reviewer identity, comments, and arbitrary provenance.

The provider-neutral desktop plugin core hashes only explicitly selected layer
exports and sends a strict request to the local workbench. GeneGIS validates
the declared open-format payload, known CRS and derived coordinate unit,
license, extent/time interval, and checksum. It then writes byte-exact assets,
input snapshots, a `RunWorkflow` command, and the reviewed four-step import
graph into a bridge capsule. It never imports a host project file or executes
desktop expressions.

The contract and redaction decision are recorded in
[`docs/adr/0017-no-code-composition-portable-publishing-and-desktop-bridge.md`](../adr/0017-no-code-composition-portable-publishing-and-desktop-bridge.md).
Exit evidence is recorded in
[`docs/reports/phase-14-m5-portable-publishing-desktop-bridge.json`](../reports/phase-14-m5-portable-publishing-desktop-bridge.json).

## Out of scope

- Multi-tenant SaaS, billing, per-seat licensing
- Vendor visual identities/trademarks
- Replacing the Workflow Graph execution path with direct-render shortcuts

## North star (unchanged)

「名古屋市の人口密度を表示」 keeps passing offline via rule planner + DuckDB
verification in CI.

## Long-term relationship

Phase 14 M1–M4 form Horizon 1 of the
[long-term product roadmap](long-term-product-roadmap.md). M5 begins the
portable publishing and desktop bridge work in Horizon 2.
