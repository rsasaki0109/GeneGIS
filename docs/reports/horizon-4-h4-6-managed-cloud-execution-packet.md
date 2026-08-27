# H4.6 Managed-Cloud Execution Packet

If the managed-cloud waiver is reopened, this packet validates that deployment
class. It must run on the declared managed-cloud Windows GPU worker; a local
workstation run is not admissible as managed-cloud evidence.

## Preconditions

- The checkout and `Cargo.lock` digest match the managed-cloud performance
  profile.
- The worker has a hardware WebGPU adapter and the pinned Pixi/GDAL environment.
- A credential-free HTTP(S) URL identifies the immutable Nagoya benchmark
  object in the managed object store. The server must expose `Content-Length`,
  `Accept-Ranges: bytes`, and at least one of `ETag` or `Last-Modified`.
- Any authentication is established outside the URL and is not written to the
  receipt. The current collector intentionally admits only directly reachable,
  exact-host allowlisted objects.

## Execute

From the repository root, first collect twenty GPU samples from one release
executable:

```powershell
$env:WGPU_BACKEND = "dx12"
./scripts/collect-gpu-scene-sample-set.ps1 `
  -SampleCount 20 `
  -OutputDirectory docs/reports/managed-cloud-gpu-samples `
  -SampleSetPath docs/reports/managed-cloud-gpu-sample-set.json
```

Then collect bounded HTTP Range evidence and evaluate the complete matrix:

```powershell
./scripts/run-managed-cloud-performance-acceptance.ps1 `
  -SourceUrl "https://OBJECT-HOST/PINNED-NAGOYA-OBJECT" `
  -GpuSampleSetPath docs/reports/managed-cloud-gpu-sample-set.json
```

Both scripts refuse to overwrite prior evidence. Partial or failed runs remain
available for audit but are never promoted to a passing matrix.

## Admission checks

The range receipt is accepted only when all of the following recompute from the
persisted JSON:

- profile, dataset, build, OS, and CPU identities match the GPU sample set;
- the source has immutable HTTP metadata and advertises byte ranges;
- four distributed requests return exact HTTP 206 bodies and `Content-Range`;
- every response length and SHA-256 digest matches its claimed range;
- no whole-object fallback occurred;
- the managed-cloud matrix has no pending dimensions or regressions, including
  the 1000 ms nearest-rank p95 first-frame threshold;
- the persisted matrix exactly matches an independent recomputation from the
  pinned profile and its recorded measurements.

The accepted outputs are
`docs/reports/horizon-4-h4-6-managed-cloud-range-receipt.json` and
`docs/reports/horizon-4-h4-6-managed-cloud-receipt.json`.

## Current roadmap decision

The project owner waived this deployment-class execution on 2026-08-27. The
waiver is recorded in `docs/reports/horizon-4-h4-6-managed-cloud-waiver.json`;
it does not claim that managed-cloud hardware, HTTP Range behavior, or
performance thresholds were validated. This packet remains available if the
decision is reopened later.
