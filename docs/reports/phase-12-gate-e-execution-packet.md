# Phase 12 Gate E execution packet

This packet turns the preregistered Trust UX study into three bounded actions.
It does not change the corpus, hidden oracle, thresholds, or admission rules.
Raw reviewer sessions are written under `gate-e-study/`, which is Git-ignored.

## 1. Prepare the pinned runner

From the repository root on the study machine:

```powershell
& "C:\Users\rsasa\AppData\Local\pixi\bin\pixi.exe" install --manifest-path tools/pdal/pixi.toml
.\scripts\prepare-gate-e-study.ps1
```

Preparation builds the optimized release CLI with the pinned Pixi GDAL 3.12.3 environment and
writes `gate-e-study/study-manifest.json` containing the corpus, Cargo.lock,
release runner, protocol, OS, architecture, and terminal identities. Each file
identity is a SHA-256 digest. Do not edit that manifest after recruiting
reviewers; preparation refuses to replace it after the first human session.

## 2. Run observed human sessions

Recruit at least three people who have not seen the hidden oracle. Use anonymous
codes only. A facilitator must physically or synchronously observe each session
without coaching diagnoses.

```powershell
.\scripts\run-gate-e-session.ps1 -ReviewerCode HUMAN_01 -FacilitatorCode FAC_01
.\scripts\run-gate-e-session.ps1 -ReviewerCode HUMAN_02 -FacilitatorCode FAC_01
.\scripts\run-gate-e-session.ps1 -ReviewerCode HUMAN_03 -FacilitatorCode FAC_01
```

Preserve aborted sessions. Never reuse a reviewer code or overwrite a session;
a rerun receives a new code. Record accommodations, training, interruptions,
and facilitator observations in a separate note without names or contact data.

## 3. Aggregate without exclusions

After all attempts, aggregate every `session-*.json`, including aborts:

```powershell
.\scripts\aggregate-gate-e-study.ps1
```

The resulting `gate-e-study/aggregate.json` is a sealed study receipt. It binds
the exact study manifest, Cargo.lock, release runner, protocol, and admitted or
excluded session digests. Its nested `aggregate` is admissible only if it
reports at least three unique human reviewers, 12 tasks per admitted reviewer,
correctness at least 0.90, median diagnosis time at most 120 seconds, median
decisive-card opening ordinal at most 2, and `aggregate.passed: true`.
Automated, duplicate, partial, changed, or tampered sessions remain excluded or
rejected and cannot pass the gate. Aggregation exits with code 2 when the
thresholds are not met, while still preserving the sealed failed receipt.

Without an explicit roadmap waiver, only after an admissible aggregate exists may it be copied to
`docs/reports/phase-12-trust-ux-human.json` and Gate E/P6-2 be changed from
`not_measured`. No synthetic or model-generated session is a substitute.

## Current roadmap decision

The project owner waived this human-participant gate on 2026-08-27. The waiver
is recorded in `docs/reports/phase-12-gate-e-waiver.json`; it does not claim that
the study ran or passed. This packet remains available if the decision is
reopened later.
