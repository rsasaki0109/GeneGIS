# ADR 0017: No-code composition, portable publishing, and desktop bridge

- Status: Accepted
- Date: 2026-08-26
- Owners: GeneGIS core, workbench, capsule, and plugin SDK maintainers

## Context

Phase 14 M5 begins the second roadmap horizon. GeneGIS already produces a
verified HTML/PNG result and an open, content-addressed result capsule. It does
not yet define which fields may enter a public view, how an embed remains bound
to offline evidence, or how a desktop GIS can transfer selected layers without
bypassing Command + Workflow Graph.

The same contracts must later support no-code workflow composition. A UI may
make graph construction approachable, but it cannot invent an execution path
that the CLI, agent, verifier, or SDK cannot replay.

## Decision

### 1. One graph contract for code, agents, and no-code UI

A no-code canvas edits a versioned `GeoWorkflow`; it does not execute UI
callbacks. Ports expose the existing CRS, coordinate unit, measure, temporal,
coverage, source, and provenance contracts. Connecting incompatible ports
fails before execution. Running a composed graph always dispatches a
`RunWorkflow` command carrying its stable digest and input snapshots.

The first no-code slice may expose only reviewed templates. Arbitrary SQL,
shell, Python, provider expressions, and undeclared network calls remain out of
scope.

### 2. Publishing starts from a verified capsule

A publication exporter accepts an offline-verifiable result capsule, never a
browser render alone. It verifies the capsule and emits:

- a self-contained `index.html` review page;
- a minimal `embed.html` viewer;
- `publication.json`, binding both views to the result, workflow, verification
  graph, policy, and capsule digests;
- the downloadable source capsule and its notices.

The generated share reference is content-addressed. Hosting may prepend an
origin later, but the stable suffix is the publication digest. Hosting state is
not part of the analytical result digest.

### 3. Public metadata is allowlisted, not copied and cleaned afterward

The public manifest may include title, description, attribution, licenses,
declared limitations, verification/trust state, CRS/units, content digests, and
portable artifact paths. It excludes local filesystem paths, source
credentials, environment values, auth headers, reviewer identities, comments,
unreleased prompts, and arbitrary provenance details.

The exporter fails closed if attribution, license, result digest, workflow
digest, or verification state is absent. A policy records every included and
redacted field class. Redaction policy identity is included in the publication
digest.

### 4. The desktop bridge transfers declared assets into a bridge capsule

The bridge is a thin, provider-neutral SDK client. A desktop host supplies only
explicitly selected layers with:

- stable layer id and display name;
- semantic kind and open format;
- known CRS and derived coordinate unit;
- license/attribution;
- exact source bytes and SHA-256 snapshot;
- optional extent and temporal interval.

GeneGIS validates and copies the bytes into a content-addressed bridge capsule.
The capsule contains a `RunWorkflow` command, stable import workflow digest,
input snapshots, layer declarations, and asset inventory. Opening the bridge
capsule is therefore an inspectable import proposal, not an implicit mutation
or direct-render shortcut.

The initial bridge accepts GeoJSON, GeoParquet, COG, COPC, and PMTiles. Host
project files, styles with executable expressions, database credentials, and
plugin binaries are not transferred.

### 5. Workbench endpoints are local export coordinators

The workbench may expose local endpoints for publication and bridge-capsule
creation. Output names are slugged and rooted under `.genegis`; callers cannot
choose an arbitrary destination. Remote publishing, tenant access control,
revocation, billing, and hosted share-link administration are separate roadmap
work.

## Verification gates

Portable publishing is complete only when tests prove:

1. a valid capsule produces byte-stable public metadata and both viewers;
2. the public manifest contains attribution, limitations, verification state,
   and digest-bound capsule download;
3. local paths and forbidden metadata do not appear in exported public files;
4. capsule or publication tampering fails verification.

The desktop bridge is complete only when tests prove:

1. at least one selected vector layer is copied byte-exactly into a bridge
   capsule;
2. CRS, units, license, source snapshot, Command, and Workflow Graph are all
   present and mutually consistent;
3. unknown CRS/unit, unsupported format, path escape, missing license, checksum
   mismatch, or changed asset bytes fail closed.

## Consequences

- Public views remain small and easy to host while their full evidence stays
  downloadable and independently verifiable.
- No-code composition expands product reach without creating a second runtime.
- Desktop interoperability is an SDK boundary, not a desktop UI imitation.
- A bridge capsule is portable and reviewable, but intentionally requires an
  explicit later import action.
