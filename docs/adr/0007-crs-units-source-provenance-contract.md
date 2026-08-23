# ADR 0007: CRS, Units, and Source/Provenance Contract

- Status: Accepted
- Date: 2026-08-22

## Context

GeneGIS workflows previously carried CRS and units as unrelated strings. The
Nagoya population-density demo declared `EPSG:4326`, but calculated area by
scaling degree coordinates at the feature centroid. That made the result
ambiguous to downstream operations and allowed an unknown CRS or unit to reach
execution without a validation boundary. Source URI and license were present
in some receipts, but were not part of the analysis verification result.

The platform's north-star workflow must be reproducible across the CLI,
workbench, and AI execution paths. A result therefore needs enough spatial and
source identity to explain what was measured, in which units, and from which
asset.

## Decision

1. Every spatial input is normalized through `genegis-crs::Crs`. The canonical
   identifier is `AUTHORITY:CODE` (for example, `EPSG:4326`). A syntactically
   valid but unknown EPSG code is retained for diagnostics and rejected by
   `Crs::require_known()` before an operation runs.
2. Coordinate-axis units are derived from the CRS definition and are carried
   separately from value units. Geographic coordinates use `degrees`; projected
   coordinates use `metres`. Derived values must declare their own unit (the
   Nagoya result uses `km²` for area and `persons/km²` for density).
3. Results and execution receipts carry source identity as a structured
   `SourceMetadata` value: dataset id when available, exact URI/path, license,
   optional SHA-256 checksum, source version, and optional RFC 3339 retrieval
   time. Checksum verification is explicit (`verified`, `declared`, `unknown`,
   or `mismatch`); `expected_checksum` records a catalog/provider declaration
   and `observed_checksum` records bytes actually read. The compatibility
   `checksum` field is the observed value when available and otherwise the
   declaration. An external URI without a declared checksum is never treated
   as verified. Credentials and other secrets are never included.
4. Area operations fail closed when CRS semantics are unknown. WGS 84 and
   JGD2011 geographic rings use WGS 84 ellipsoidal surface integration;
   projected metre rings use the projected shoelace formula. The former
   centroid-latitude degree scaling remains available only as a compatibility
   API and is not used by the north-star workflow.
5. `VerificationReport`, workflow inputs, and the append-only provenance entry
   repeat the same CRS, coordinate units, derived units, area method, and source
   snapshot so a serialized result is self-describing without consulting an
   in-memory catalog.
6. The canonical Nagoya snapshot uses the [名古屋市 令和2年国勢調査確定値 page](https://www.city.nagoya.jp/shisei/toukei/1003703/1003773/1003809/1034253/1003818.html)
   and its [official Excel table](https://www.city.nagoya.jp/_res/projects/default_project/_page_/001/003/818/toukeihyo.xlsx)
   for population, not a provisional e-Stat value. The immutable source
   manifest and independent area oracle are bundled under
   `/home/sasaki/workspace/GeneGIS/examples/nagoya-population-density/data/`.
   Boundary adapters preserve every polygon part and retain valid interior
   rings so area calculations subtract holes.
   The canonical boundary fixture digest is
   `sha256:d0f8958813fe28e9428169ca7c638a0ea3b3ed7ae526750156d3f94e1308d30e`;
   the population asset digest is
   `sha256:bd19086c0e859d397c2b3cb8e945fcda850fd3907a404e3f9756f74b154e8c6c`.

## Consequences

- A planner or plugin cannot silently calculate an area from degrees as if they
  were metres; unsupported or malformed CRS identifiers produce a validation
  error.
- Replaying the same input keeps the calculation method, unit contract, source
  version, and content checksum stable. Retrieval time is an adapter-supplied
  event and is not generated unconditionally during execution, so it does not
  perturb an input/workflow digest.
- Existing JSON consumers may continue reading the legacy `crs`, `units`, and
  `density_unit` fields. New consumers should prefer the structured metadata
  fields and treat missing legacy metadata as invalid for spatial calculations.
- Broad EPSG/PROJ database coverage remains follow-up work; the MVP registry
  intentionally covers WGS 84, JGD2011, Nagoya's local projected CRS, and the
  common Web Mercator/UTM definitions needed by bundled raster examples.
  Polygon-hole subtraction is part of the canonical vector model and is
  covered by geometry regression tests.

## Verification

The CRS crate tests parsing, unknown-code rejection, coordinate-domain checks,
source metadata serialization, local SHA-256 verification, and preservation of
expected/observed values on mismatch. The geometry and vector tests cover
multipart polygons, hole subtraction, missing/unsupported geometry, and the
immutable Nagoya oracle. The Nagoya analysis and ask pipeline regression tests
assert the ellipsoidal method, coordinate units, official source URLs/license,
source version, checksum status, population total delta, per-ward area error,
density-oracle check, and receipt propagation.
