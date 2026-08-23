# Phase 11 reviewer timing protocol

The Gate-B metric requires a human reviewer to identify every seeded root cause
without raw JSON, with a median duration of at most 120 seconds. Automated key
injection is not valid evidence for this metric.

**Execution status (2026-08-23):** explicitly skipped by the user. An attempted
session was aborted before any answer and wrote no timing report. Gate B is
therefore `not_evaluated`, not passed; the command below remains available if
the reviewer-speed claim is needed later.

Run from `/home/sasaki/workspace/GeneGIS` in an interactive terminal:

```text
target/debug/genegis bench review --reviewer REVIEWER_ID \
  --output /home/sasaki/workspace/GeneGIS/docs/reports/phase-11-review-timing.json
```

The fixed six-task corpus covers source drift, join total, ward coverage,
area tolerance, density tolerance, and render/result divergence. Before each
task, the failure remains hidden and the timer remains stopped until the
reviewer presses Space or Enter. The task then shows the same structured
failure code, subject, and detail exposed by the Trust Debugger and asks for
the first Workflow node to inspect. Arrow keys or `j`/`k` select; Enter
submits. The output records every answer, per-task wall time, median,
runner/corpus identity, and a computed pass only when all answers are correct
and the median is at most 120 seconds.

Reviewers should not inspect the task source or acceptance report beforehand.
The report is append-only evidence: reruns use a separate filename and are not
silently averaged with earlier attempts.
