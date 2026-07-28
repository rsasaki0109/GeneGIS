# ADR 0006: Verified Federated STAC Asset Binding

- **Status:** Accepted
- **Date:** 2026-07-28

## Context

Phase 10 could search federated STAC endpoints and independently read remote
GeoParquet, but there was no auditable contract connecting discovery to
execution. A planner could not explain why one asset was selected, and the
execution receipt did not retain STAC identity, endpoint URLs, CRS, units,
license, or source coverage.

## Decision

GeneGIS flattens discovered STAC item assets into deterministic candidates and
verifies media type, data role, search-area coverage, CRS, units, and license
before binding. Compatible candidates receive a stable score; ties are resolved
by STAC item key and asset key. The receipt includes all candidates, verification
evidence, selection reason, source endpoint URLs, STAC identity, and retrieval
time.

Binding is represented by `Command::BindStacAsset` and the
`federated_asset_execution_template` GeoWorkflow. Remote GeoParquet execution
uses HTTP Range requests and returns schema, CRS, source, request-count, and
retrieval evidence in the same API receipt.

## Consequences

- Planner decisions are explainable and reproducible without model-specific
  ranking behavior.
- Assets missing required metadata are visible as rejected candidates rather
  than silently accepted.
- Search, binding, and execution preserve the same source identity.
- Phase 10 remote requests use a fail-closed policy: explicit host allowlist,
  no redirects, 8 MiB response limit, 15-second timeout, and no URL credentials.
- Loopback remains enabled for deterministic local and CI fixtures.

## Verification

The integration test at
`C:\Users\rsasa\Workspace\GeneGIS\crates\genegis-catalog\tests\federated_asset_e2e.rs`
runs two HTTP STAC endpoints, deduplicates their result, binds the verified
GeoParquet asset through Command + GeoWorkflow, reads row group 0 using HTTP
Range requests, and verifies schema, CRS, source URI, and provenance fields.
