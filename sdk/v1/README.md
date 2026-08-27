# GeneGIS SDK v1

SDK v1 fixes one language-neutral plugin contract. Rust is the reference
implementation; TypeScript and Python bindings model the same manifest and
capability vocabulary; the WIT world is the sandboxed WASM call boundary.

All plugin operations still enter GeneGIS as Commands and reviewed Workflow
Graph nodes. SDK code cannot mutate project state directly.

## Contract artifacts

- `schema/plugin-manifest.schema.json` — authoritative JSON interchange schema
- `typescript/index.ts` — TypeScript types and fail-closed validator
- `python/genegis_sdk_v1.py` — Python dataclasses and validator
- `wasm/genegis-plugin-v1.wit` — WASM component exports
- `conformance/valid-plugin.json` — shared positive corpus item

Registry publication additionally requires an Ed25519 signature from a trusted
key. Resolution re-verifies the signature and artifact SHA-256 and rejects any
release carrying a signed revocation. Billing and marketplace economics are not
part of SDK v1.
