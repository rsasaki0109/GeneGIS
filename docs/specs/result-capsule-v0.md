# GeneGIS Result Capsule v0

Version `0.1.0` is an open directory profile for proof-carrying spatial
analysis. Paths use `/` separators and are relative to the capsule root.

```text
capsule.json
artifacts/map.html
artifacts/map.png
metadata/analysis.json
metadata/command.json
metadata/receipt.json
metadata/verification-graph.json
metadata/verification-policy.json
metadata/workflow.json
reports/trust.html
```

The receipt manifest role is the stable identifier `execution-receipt`.
Consumers should use roles and manifest paths rather than assuming filenames.

Every manifest entry records:

- `path`: safe normalized relative path;
- `role`: semantic subject role;
- `media_type`: media type of exact bytes;
- `sha256`: lowercase `sha256:<64 hex>` identity;
- `bytes`: exact byte length.

`capsule.json` is not self-hashed. Publisher authenticity therefore comes from
an expected capsule/result digest or a later attestation, while the manifest
establishes internal consistency. An external VerificationPolicy should be
supplied for release verification so a capsule cannot weaken its own rules.

Offline verification command:

```text
genegis capsule verify CAPSULE --policy CAPSULE/metadata/verification-policy.json
```

The verifier performs no HTTP request and never invokes a planner or LLM.

`reports/trust.html` is regenerated deterministically from the policy-derived
trust assessment plus result, workflow, policy, verification-graph, and map
artifact digests. The offline verifier rejects a report whose claim differs
from those subjects even when an attacker refreshes its manifest entry.
