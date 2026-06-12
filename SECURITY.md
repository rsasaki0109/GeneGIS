# Security Policy

## Supported versions

GeneGIS is in Phase 0 (pre-release). Security fixes land on `main`.

## Reporting a vulnerability

Please report security issues privately to the maintainers — do not open public issues for exploitable vulnerabilities.

Include:

- Description and impact
- Steps to reproduce
- Affected version/commit

## Plugin sandbox

GeneGIS plugins run in capability-sandboxed environments (WASM default). Report sandbox escapes with high priority.

## Sensitive data

Do not commit IMEI, serial numbers, private keys, credentials, or private geospatial datasets.
