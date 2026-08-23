# Phase 12 map-first Trust UX protocol

This protocol measures RFC 0004 Gate E. It does not permit simulated reviewers,
automated key injection, model answers, or developer self-scoring to count as
human evidence. The fixed corpus and hidden oracles are bound by digest in
`/home/sasaki/workspace/GeneGIS/docs/reports/phase-12-trust-ux-preregistration.json`.

## Preparation

Build the exact integrated revision before recruiting reviewers:

```console
cargo build -p genegis-cli
```

Choose three or more people who have not opened
`/home/sasaki/workspace/GeneGIS/crates/genegis-testkit/src/trust_ux.rs` or the
preregistration oracle. Assign each person a pseudonymous code containing only
letters, digits, `-`, or `_`. Do not use names, email addresses, or employee
identifiers. Assign a different pseudonymous facilitator code.

The facilitator records runner hardware, terminal dimensions, accessibility
accommodations, training given, and any interruption in a separate study note.
Do not coach diagnoses. A short practice task not present in the fixed corpus is
allowed, but its timing is not recorded.

## Session

Run once per reviewer, changing only the reviewer code and output path:

```console
target/debug/genegis bench trust-ux \
  --human \
  --reviewer-code HUMAN_01 \
  --facilitator-code FAC_01 \
  --output /home/sasaki/workspace/GeneGIS/docs/reports/phase-12-trust-ux-HUMAN_01.json
```

Each task remains hidden and untimed until the reviewer presses Space or Enter.
The next screen starts with a map and highlighted spatial subject. Keys `1`,
`2`, and `3` open Source, Contract/workflow, or I/O/artifact evidence cards.
Each opening counts as an interaction. The ordinal of the first decisive-card
opening is recorded separately. Press `a` to show diagnoses, then arrows or
`j`/`k` and Enter to submit. Pressing `q` records an abort and writes the partial
session; it never silently discards the attempt.

Without `--human`, the runner deliberately writes `session_kind: automated`.
Such sessions are useful for smoke testing but the aggregator can never count
them toward the reviewer or task thresholds. The facilitator uses `--human`
only while physically or synchronously observing the named pseudonymous
reviewer; software cannot infer human presence from terminal input alone.

The facilitator must preserve every output, including aborts. A rerun uses a
new reviewer code and the original aborted report remains in the study bundle.

## Aggregate

After all attempts, include every session file, including automated smoke runs
and aborts:

```console
target/debug/genegis bench trust-ux-aggregate \
  --input /absolute/path/session-01.json \
  --input /absolute/path/session-02.json \
  --input /absolute/path/session-03.json \
  --output /home/sasaki/workspace/GeneGIS/docs/reports/phase-12-trust-ux-human.json
```

The aggregator verifies report and corpus digests, fixed task order, embedded
oracles, pseudonyms, completeness, decisive evidence navigation, and unique
reviewers. Automated sessions, partial sessions, duplicate reviewers, changed
corpora, missing decisive evidence, and tampered results do not enter metrics.

Gate E passes only with at least three admitted human reviewers and twelve
completed tasks per reviewer, aggregate correctness at least 90%, median task
diagnosis at most 120 seconds, and median map-to-decisive-evidence interactions
at most two. Until a real report meets every threshold,
`/home/sasaki/workspace/GeneGIS/docs/reports/phase-12-acceptance.json` must remain
`not_measured` for Gate E.
