# ADR 0010: GeoContract v0 Semantic Type System

- **Status:** Accepted
- **Date:** 2026-08-23
- **Decision owners:** GeneGIS core team

## Context

The original `WorkflowInputContract` recorded a name, CRS, coordinate unit,
value-unit string, and source snapshot. That boundary catches important spatial
errors but cannot express whether two values refer to the same period,
population universe, geographic coverage, join cardinality, aggregation basis,
or quality tolerance.

Those omissions are material for the north-star density workflow. A valid
`persons / km²` calculation can still be wrong when the population year differs
from the boundary definition, one ward is absent, join keys are duplicated, or
the numerator means a different population universe.

## Decision

Introduce the dependency-light `genegis-contract` crate and versioned
`GeoContract` document. It depends on `genegis-crs`; workflow, analysis, core,
and adapter crates may depend on it, but it does not depend on workflow or an
execution engine.

GeoContract v0 schema version is `0.1.0` and covers six domains:

1. spatial: geometry kind, CRS, axis order/unit, extent, and resolution;
2. measure: kind, unit, numerator/denominator, aggregation, and universe;
3. temporal: reference period, granularity, and observation time;
4. coverage: scope, feature count, join keys, uniqueness, and null policy;
5. source: immutable snapshot, authority, license/version through the snapshot,
   and freshness requirement;
6. quality: uncertainty and metric-specific tolerances in parts per million.

The authoritative artifacts are:

- Rust types and validator:
  `/home/sasaki/workspace/GeneGIS/crates/genegis-contract/src/lib.rs`
- JSON Schema:
  `/home/sasaki/workspace/GeneGIS/crates/genegis-contract/schema/geo-contract-v0.schema.json`
- directional compatibility truth table:
  `/home/sasaki/workspace/GeneGIS/crates/genegis-contract/COMPATIBILITY.md`

## Validation and compatibility

Release-bound validation is fail-closed. When its domain is present, an unknown
CRS, axis order, coordinate unit, measure kind, aggregation basis, temporal
granularity, key uniqueness, or null policy is invalid. Invalid extents,
resolutions, density terms, duplicate join keys, and duplicate quality metrics
are also rejected.

Compatibility is directional:

```text
provided.compatibility_with(required)
```

A missing requirement is unconstrained. Equal known semantics are compatible;
different known semantics are incompatible; missing/unknown provided semantics
are indeterminate. Overall ordering is:

```text
incompatible > indeterminate > compatible
```

Indeterminate is never a verified match. A provided quality tolerance is
compatible only when it is equal to or stricter than the required maximum.

## Workflow migration

`WorkflowInputContract.geo_contract` and
`WorkflowStep.output_contracts` attach the new contract to graph values. The
legacy CRS/unit/source fields remain serde-compatible during migration. If both
representations exist, workflow validation rejects disagreement. Runtime source
resolution updates both representations, while retrieval timestamps remain
events excluded from stable workflow identity.

The Nagoya template carries full boundary/population input contracts and a
density output contract with a 2020 reference period, 16-ward coverage, explicit
`persons/km2` terms, and a 5,000 ppm verification tolerance.

## Versioning

Schema versions are explicit strings. Unknown versions fail validation instead
of being coerced. Backward-compatible readers may retain legacy workflow fields,
but a semantic change to compatibility or field meaning requires a new
GeoContract schema version and migration/conformance fixtures.

`VerificationPolicy` has its own schema version. Contract validity and trust
level are related but separate: GeoContract describes meaning; policy decides
which valid/compatible meanings and evidence are sufficient for release.

## Consequences

- Workflow digests change when contract meaning changes.
- Existing serialized workflows remain readable through serde defaults.
- Adapters gain a common semantic boundary rather than free-form metadata.
- More explicit metadata is required before a result can be verified.
- Exploratory handling of unknowns must be implemented by policy and visibly
  lower trust; it cannot weaken the release validator.
