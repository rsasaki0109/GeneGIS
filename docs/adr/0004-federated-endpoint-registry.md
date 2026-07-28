# ADR 0004: Federated STAC Endpoint Registry

- Status: Accepted
- Date: 2026-07-28

## Context

Federated STAC discovery needs named endpoints shared by CLI and Workbench. Endpoint
changes must follow GeneGIS Command + Workflow Graph rules and retain provenance.
Some STAC APIs also require credentials, but project files must not contain secrets.

## Decision

Persist the registry at `.genegis/catalog/endpoints.json`, overridable with
`GENEGIS_STAC_ENDPOINT_REGISTRY`.

Every register, remove, and search operation records:

- a `CommandEnvelope` with origin and timestamp;
- the corresponding `GeoWorkflow`;
- a provenance entry linked to the workflow;
- WGS84 CRS and degree units for spatial searches;
- endpoint identity and success/failure counts.

Authentication configuration stores only an environment-variable reference:

- `anonymous`;
- `bearer_env`;
- `header_env`.

The secret value is resolved only when an HTTP request is sent and is never added
to the registry, workflow, command, provenance, or search result.

## Consequences

- CLI and Workbench use the same portable registry and audit model.
- A missing authentication environment variable fails only that endpoint; other
  federated endpoints can still return results.
- Registry schema migrations must increment `schema_version`.
- Secret rotation does not require modifying GeneGIS project files.
