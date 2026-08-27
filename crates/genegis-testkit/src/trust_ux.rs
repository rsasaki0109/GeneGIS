//! Preregistered map-first human Trust UX study corpus and evidence model.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Stable Phase 12 corpus version.
pub const TRUST_UX_CORPUS_VERSION: &str = "phase-12-map-first-trust-v1";

/// Preregistered digest of the complete v1 corpus, including hidden oracles.
pub const TRUST_UX_CORPUS_DIGEST: &str =
    "sha256:cde790805a1e6dc4f1200d92b5aa95804e94e7604667ab7219af32e5133f88ca";

/// Pinned execution context prepared before any human reviewer is recruited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustUxStudyManifest {
    /// Manifest schema version.
    pub schema_version: String,
    /// UTC preparation timestamp.
    pub prepared_at: String,
    /// Must remain `prepared_human_sessions_pending` until aggregation.
    pub status: String,
    /// Fixed corpus version.
    pub corpus_version: String,
    /// Fixed corpus digest.
    pub corpus_digest: String,
    /// Cargo.lock digest used to build the runner.
    pub build_lock_digest: String,
    /// Exact CLI executable digest.
    pub runner_digest: String,
    /// Exact preregistered protocol digest.
    pub protocol_digest: String,
    /// Study host operating-system identity.
    pub os: String,
    /// Study host architecture.
    pub architecture: String,
    /// Pinned terminal width.
    pub terminal_columns: u32,
    /// Pinned terminal height.
    pub terminal_rows: u32,
    /// Repository-relative protocol path.
    pub protocol: String,
    /// Minimum unique human reviewers.
    pub minimum_unique_human_reviewers: usize,
    /// Required tasks for every admitted reviewer.
    pub task_count_per_reviewer: usize,
    /// Raw sessions must remain outside version control.
    pub raw_sessions_git_ignored: bool,
}

/// Whether a timing session was performed by a person or automation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustUxSessionKind {
    /// A person completed the session under the documented protocol.
    Human,
    /// A smoke test, scripted key injection, or other automated execution.
    Automated,
}

/// One reviewer-facing evidence card, initially hidden behind the map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustUxEvidenceCard {
    /// Stable card identity.
    pub card_id: String,
    /// Short UI title.
    pub title: String,
    /// Human-readable evidence without raw JSON.
    pub detail: String,
}

/// One possible diagnosis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustUxAnswerChoice {
    /// Stable answer identity.
    pub answer_id: String,
    /// Human-readable diagnosis.
    pub label: String,
}

/// One preregistered map-first diagnosis task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustUxTask {
    /// Stable task identity.
    pub task_id: String,
    /// Required RFC failure category.
    pub category: String,
    /// Map title shown before any evidence card.
    pub map_title: String,
    /// Textual map overlay; this is presentation, not raw evidence JSON.
    pub map_lines: Vec<String>,
    /// Cards reachable from the map in one interaction each.
    pub evidence_cards: Vec<TrustUxEvidenceCard>,
    /// Diagnosis choices.
    pub answer_choices: Vec<TrustUxAnswerChoice>,
    /// Oracle answer, hidden by the interactive runner.
    pub expected_answer_id: String,
    /// Card containing the decisive evidence, hidden by the runner.
    pub decisive_card_id: String,
}

/// One response recorded by the interactive runner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustUxTaskResult {
    /// Stable task identity.
    pub task_id: String,
    /// Submitted diagnosis, absent when aborted.
    pub answer_id: Option<String>,
    /// Whether the diagnosis matched the preregistered oracle.
    pub correct: bool,
    /// Wall time after task reveal.
    pub elapsed_seconds: f64,
    /// Evidence-card openings from the initial map.
    pub interaction_count: u32,
    /// Ordered card identities opened by the reviewer; repeated openings remain visible.
    pub opened_card_ids: Vec<String>,
    /// Opening count at which decisive evidence was first reached.
    pub interactions_to_decisive_evidence: Option<u32>,
    /// Whether the reviewer aborted this task/session.
    pub aborted: bool,
}

/// One immutable reviewer session report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustUxSessionReport {
    /// Report schema version.
    pub schema_version: String,
    /// Human or automated session label.
    pub session_kind: TrustUxSessionKind,
    /// Preregistered anonymized reviewer code; never a name or email address.
    pub reviewer_id: String,
    /// Pseudonymous facilitator code, required for human sessions.
    pub facilitator_id: Option<String>,
    /// Runner binary/build identity.
    pub runner_identity: String,
    /// Digest of the exact prepared study manifest; mandatory for human sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub study_manifest_digest: Option<String>,
    /// Fixed corpus version.
    pub corpus_version: String,
    /// Digest of the complete fixed corpus including hidden oracles.
    pub corpus_digest: String,
    /// RFC3339 start time.
    pub started_at: String,
    /// RFC3339 finish/abort time.
    pub finished_at: String,
    /// Results in preregistered task order, including an aborted final task.
    pub results: Vec<TrustUxTaskResult>,
    /// Digest over every preceding report field.
    pub report_digest: String,
}

/// Aggregate human-study acceptance report. Automated sessions are reported but excluded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustUxAggregateReport {
    /// Report schema version.
    pub schema_version: String,
    /// Corpus version shared by admitted human sessions.
    pub corpus_version: String,
    /// Corpus digest shared by admitted human sessions.
    pub corpus_digest: String,
    /// Total supplied sessions.
    pub supplied_sessions: usize,
    /// Valid, complete, unique human reviewers used in metrics.
    pub admitted_human_reviewers: usize,
    /// Automated sessions excluded from metrics.
    pub excluded_automated_sessions: usize,
    /// Invalid, incomplete, duplicate, or mixed-corpus sessions.
    pub rejected_sessions: usize,
    /// Total admitted task answers.
    pub admitted_tasks: usize,
    /// Correct admitted task answers.
    pub correct_tasks: usize,
    /// Correct answers divided by admitted answers.
    pub correctness: f64,
    /// Median diagnosis time over admitted answers.
    pub median_diagnosis_seconds: Option<f64>,
    /// Median map-to-decisive-evidence interaction count.
    pub median_interactions_to_decisive_evidence: Option<f64>,
    /// Total aborts in supplied session evidence.
    pub aborts: usize,
    /// Human reviewer count gate.
    pub reviewers_gate: bool,
    /// Twelve-task-per-reviewer gate.
    pub task_gate: bool,
    /// Aggregate correctness gate.
    pub correctness_gate: bool,
    /// Diagnosis-time gate.
    pub time_gate: bool,
    /// Interaction gate.
    pub interaction_gate: bool,
    /// True only when every Gate E requirement passes.
    pub passed: bool,
}

/// Manifest-bound, sealed Gate E acceptance receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustUxStudyAggregateReceipt {
    /// Receipt schema version.
    pub schema_version: String,
    /// Digest of the exact study manifest bytes.
    pub study_manifest_digest: String,
    /// Build identity copied from the validated manifest.
    pub build_lock_digest: String,
    /// Runner identity copied from the validated manifest.
    pub runner_digest: String,
    /// Protocol identity copied from the validated manifest.
    pub protocol_digest: String,
    /// Sorted sealed session identities, including excluded automation and rejected sessions.
    pub session_report_digests: Vec<String>,
    /// Recomputed threshold result.
    pub aggregate: TrustUxAggregateReport,
    /// Digest over every preceding receipt field.
    pub receipt_digest: String,
}

/// Return the fixed twelve-task Phase 12 corpus.
pub fn trust_ux_task_corpus() -> Vec<TrustUxTask> {
    vec![
        task("source-drift", "source_drift", "Boundary snapshot changed", "Naka ward", "Source", "Executed digest differs from the approved MLIT snapshot.", "source_changed", "The source snapshot changed", "Workflow logic is slow", "Map colors need tuning"),
        task("edit-conflict", "edit_conflict", "Concurrent boundary edit", "Atsuta ward", "Edit", "Revision 42 was based on revision 40; revision 41 changed the same ring.", "stale_edit", "A stale concurrent edit must be resolved", "The CRS is geographic", "The adapter is unavailable"),
        task("crs-unit", "crs_unit_error", "Implausible density", "Minato ward", "CRS / units", "Area was calculated from degree coordinates while the contract requires square kilometres.", "unit_mismatch", "CRS and area units are incompatible", "Population is stale", "The renderer dropped a feature"),
        task("adapter-denial", "adapter_denial", "Processing step denied", "All wards", "Adapter", "QGIS operation requested native-code and network capabilities absent from its admitted manifest.", "capability_denied", "The adapter capability policy denied execution", "The cloud object lacks ranges", "The legend is incomplete"),
        task("cloud-fallback", "cloud_fallback", "Remote raster read warning", "East viewport", "Cloud I/O", "The server returned a whole-object 200 response instead of the requested byte range.", "whole_object_fallback", "Cloud I/O fell back to a whole-object transfer", "The source has an open dispute", "The edit revision is stale"),
        task("changed-result", "changed_result", "Verified result changed", "Moriyama ward", "Result diff", "Density changed while source, CRS, and policy stayed fixed; the workflow digest differs.", "workflow_changed", "The workflow/result identity changed", "The source license is missing", "The GPU is integrated"),
        task("uncertainty", "uncertainty", "Estimate shown as exact", "Midori ward", "Uncertainty", "Population is an estimate with ±3.2% scope, but the displayed value omits the interval.", "uncertainty_hidden", "Required uncertainty is hidden", "The source digest changed", "A topology repair failed"),
        task("open-dispute", "open_dispute", "Source challenge unresolved", "Nakagawa ward", "Source Assurance", "A publisher correction challenge remains open and policy requires resolved disputes.", "dispute_open", "An unresolved source dispute blocks trust", "The PMTiles tile is absent", "The style font changed"),
        task("topology-damage", "topology_damage", "Gap after split", "Showa ward", "Topology", "The split output loses 124.6 m² compared with the pre-edit polygon.", "topology_invalid", "The edit violates topology conservation", "The database is read-only", "The source is old"),
        task("schema-violation", "schema_violation", "Join key rejected", "Chikusa ward", "Contract", "ward_code is null although the input GeoContract requires a unique non-null key.", "schema_invalid", "The data violates its schema contract", "The raster overview is missing", "The map is zoomed out"),
        task("coverage-gap", "coverage_gap", "Ward missing from output", "Tenpaku ward", "Coverage", "The official 16-key oracle finds 15 unique wards in the result.", "coverage_incomplete", "The result has incomplete ward coverage", "The adapter build drifted", "Uncertainty is too large"),
        task("artifact-divergence", "artifact_divergence", "Map disagrees with result", "Kita ward", "Artifact", "The verified table contains 16 wards, but the exported map contains 15 feature identifiers.", "artifact_mismatch", "The rendered artifact diverges from the result", "The source has no ETag", "The area tolerance is strict"),
    ]
}

#[allow(clippy::too_many_arguments)]
fn task(
    task_id: &str,
    category: &str,
    map_title: &str,
    hotspot: &str,
    decisive_title: &str,
    decisive_detail: &str,
    expected_answer_id: &str,
    expected_label: &str,
    distractor_a: &str,
    distractor_b: &str,
) -> TrustUxTask {
    let mut answer_choices = vec![
        TrustUxAnswerChoice {
            answer_id: expected_answer_id.into(),
            label: expected_label.into(),
        },
        TrustUxAnswerChoice {
            answer_id: format!("{task_id}_distractor_a"),
            label: distractor_a.into(),
        },
        TrustUxAnswerChoice {
            answer_id: format!("{task_id}_distractor_b"),
            label: distractor_b.into(),
        },
    ];
    let rotation = task_id.bytes().map(usize::from).sum::<usize>() % answer_choices.len();
    answer_choices.rotate_left(rotation);
    TrustUxTask {
        task_id: task_id.into(),
        category: category.into(),
        map_title: map_title.into(),
        map_lines: vec![
            "      ╭───────╮  ╭─────╮".into(),
            "  ╭───╯       ╰──╯     │".into(),
            format!("  │   ● {hotspot:<14}│"),
            "  ╰────╮    ╭───────────╯".into(),
            "       ╰────╯".into(),
        ],
        evidence_cards: vec![
            TrustUxEvidenceCard {
                card_id: "source".into(),
                title: "Source".into(),
                detail: if decisive_title == "Source" || decisive_title == "Source Assurance" {
                    decisive_detail.into()
                } else {
                    "Snapshot identity and publisher checks pass.".into()
                },
            },
            TrustUxEvidenceCard {
                card_id: "contract".into(),
                title: "Contract / workflow".into(),
                detail: if matches!(
                    decisive_title,
                    "Edit"
                        | "CRS / units"
                        | "Adapter"
                        | "Result diff"
                        | "Topology"
                        | "Contract"
                        | "Coverage"
                ) {
                    decisive_detail.into()
                } else {
                    "CRS, units, schema, and workflow identity pass.".into()
                },
            },
            TrustUxEvidenceCard {
                card_id: "artifact".into(),
                title: "I/O / artifact".into(),
                detail: if matches!(decisive_title, "Cloud I/O" | "Uncertainty" | "Artifact") {
                    decisive_detail.into()
                } else {
                    "Selected I/O and rendered artifact checks pass.".into()
                },
            },
        ],
        answer_choices,
        expected_answer_id: expected_answer_id.into(),
        decisive_card_id: match decisive_title {
            "Source" | "Source Assurance" => "source",
            "Cloud I/O" | "Uncertainty" | "Artifact" => "artifact",
            _ => "contract",
        }
        .into(),
    }
}

/// Compute the stable digest of the complete preregistered corpus.
pub fn trust_ux_corpus_digest() -> String {
    let bytes = serde_json::to_vec(&trust_ux_task_corpus()).expect("Trust UX corpus JSON");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Seal a session by computing its report digest.
pub fn seal_trust_ux_session(mut report: TrustUxSessionReport) -> TrustUxSessionReport {
    report.report_digest.clear();
    report.report_digest = session_digest(&report);
    report
}

/// Validate report integrity, corpus binding, anonymity, ordering, and result oracles.
pub fn validate_trust_ux_session(report: &TrustUxSessionReport) -> Result<(), String> {
    let mut unsigned = report.clone();
    unsigned.report_digest.clear();
    if report.report_digest != session_digest(&unsigned) {
        return Err("session report digest mismatch".into());
    }
    if report.schema_version != "0.1.0" {
        return Err("unsupported session report schema version".into());
    }
    if trust_ux_corpus_digest() != TRUST_UX_CORPUS_DIGEST {
        return Err("compiled corpus differs from its preregistered digest".into());
    }
    if report.corpus_version != TRUST_UX_CORPUS_VERSION
        || report.corpus_digest != TRUST_UX_CORPUS_DIGEST
    {
        return Err("session corpus identity mismatch".into());
    }
    validate_pseudonym("reviewer", &report.reviewer_id)?;
    if report.session_kind == TrustUxSessionKind::Human {
        let facilitator = report
            .facilitator_id
            .as_deref()
            .ok_or("human session requires a facilitator pseudonym")?;
        validate_pseudonym("facilitator", facilitator)?;
        if facilitator == report.reviewer_id {
            return Err("reviewer and facilitator pseudonyms must differ".into());
        }
        if !report
            .study_manifest_digest
            .as_deref()
            .is_some_and(valid_digest)
        {
            return Err("human session requires a valid study manifest digest".into());
        }
    }
    if report.runner_identity.trim().is_empty()
        || report.started_at.trim().is_empty()
        || report.finished_at.trim().is_empty()
    {
        return Err("runner and session timestamps are required".into());
    }
    let tasks = trust_ux_task_corpus();
    if report.results.len() > tasks.len() {
        return Err("session contains more results than preregistered tasks".into());
    }
    for (index, result) in report.results.iter().enumerate() {
        let task = &tasks[index];
        if result.task_id != task.task_id {
            return Err(format!("task order mismatch at index {index}"));
        }
        if !result.elapsed_seconds.is_finite() || result.elapsed_seconds < 0.0 {
            return Err(format!("invalid elapsed time for {}", result.task_id));
        }
        let expected_correct = result.answer_id.as_deref() == Some(&task.expected_answer_id);
        if let Some(answer_id) = &result.answer_id {
            if !task
                .answer_choices
                .iter()
                .any(|choice| &choice.answer_id == answer_id)
            {
                return Err(format!("unknown answer for {}", result.task_id));
            }
        }
        let known_cards = task
            .evidence_cards
            .iter()
            .map(|card| card.card_id.as_str())
            .collect::<BTreeSet<_>>();
        if result.interaction_count as usize != result.opened_card_ids.len()
            || result
                .opened_card_ids
                .iter()
                .any(|card| !known_cards.contains(card.as_str()))
        {
            return Err(format!("interaction log mismatch for {}", result.task_id));
        }
        let derived_decisive = result
            .opened_card_ids
            .iter()
            .position(|card| card == &task.decisive_card_id)
            .map(|index| index as u32 + 1);
        if result.correct != expected_correct
            || (result.aborted && result.answer_id.is_some())
            || (!result.aborted && result.answer_id.is_none())
            || (result.aborted && index + 1 != report.results.len())
        {
            return Err(format!("result oracle mismatch for {}", result.task_id));
        }
        if result.interactions_to_decisive_evidence != derived_decisive {
            return Err(format!(
                "invalid decisive interaction for {}",
                result.task_id
            ));
        }
    }
    Ok(())
}

/// Aggregate sessions using the exact RFC 0004 Gate E thresholds.
pub fn aggregate_trust_ux_sessions(sessions: &[TrustUxSessionReport]) -> TrustUxAggregateReport {
    aggregate_trust_ux_sessions_bound(sessions, None)
}

fn aggregate_trust_ux_sessions_bound(
    sessions: &[TrustUxSessionReport],
    expected_study_manifest_digest: Option<&str>,
) -> TrustUxAggregateReport {
    let corpus_digest = trust_ux_corpus_digest();
    let tasks = trust_ux_task_corpus();
    let mut reviewer_ids = BTreeSet::new();
    let mut admitted = Vec::new();
    let mut excluded_automated_sessions = 0;
    let mut rejected_sessions = 0;
    let aborts = sessions
        .iter()
        .flat_map(|session| &session.results)
        .filter(|result| result.aborted)
        .count();
    for session in sessions {
        if session.session_kind == TrustUxSessionKind::Automated {
            excluded_automated_sessions += 1;
            continue;
        }
        let complete = validate_trust_ux_session(session).is_ok()
            && expected_study_manifest_digest
                .is_none_or(|expected| session.study_manifest_digest.as_deref() == Some(expected))
            && session.results.len() == tasks.len()
            && session.results.iter().all(|result| {
                !result.aborted
                    && result.answer_id.is_some()
                    && result.interactions_to_decisive_evidence.is_some()
            });
        if !complete || !reviewer_ids.insert(session.reviewer_id.clone()) {
            rejected_sessions += 1;
            continue;
        }
        admitted.push(session);
    }
    let results = admitted
        .iter()
        .flat_map(|session| session.results.iter())
        .collect::<Vec<_>>();
    let correct_tasks = results.iter().filter(|result| result.correct).count();
    let correctness = ratio(correct_tasks, results.len());
    let median_diagnosis_seconds = median(
        results
            .iter()
            .map(|result| result.elapsed_seconds)
            .collect(),
    );
    let median_interactions_to_decisive_evidence = median(
        results
            .iter()
            .filter_map(|result| result.interactions_to_decisive_evidence.map(f64::from))
            .collect(),
    );
    let reviewers_gate = admitted.len() >= 3;
    let task_gate = admitted.len() >= 3 && results.len() == admitted.len() * tasks.len();
    let correctness_gate = correctness >= 0.9;
    let time_gate = median_diagnosis_seconds.is_some_and(|value| value <= 120.0);
    let interaction_gate =
        median_interactions_to_decisive_evidence.is_some_and(|value| value <= 2.0);
    TrustUxAggregateReport {
        schema_version: "0.1.0".into(),
        corpus_version: TRUST_UX_CORPUS_VERSION.into(),
        corpus_digest,
        supplied_sessions: sessions.len(),
        admitted_human_reviewers: admitted.len(),
        excluded_automated_sessions,
        rejected_sessions,
        admitted_tasks: results.len(),
        correct_tasks,
        correctness,
        median_diagnosis_seconds,
        median_interactions_to_decisive_evidence,
        aborts,
        reviewers_gate,
        task_gate,
        correctness_gate,
        time_gate,
        interaction_gate,
        passed: reviewers_gate && task_gate && correctness_gate && time_gate && interaction_gate,
    }
}

/// Validate the immutable study context before a human session or aggregation.
pub fn validate_trust_ux_study_manifest(manifest: &TrustUxStudyManifest) -> Result<(), String> {
    if manifest.schema_version != "0.1.0"
        || manifest.status != "prepared_human_sessions_pending"
        || manifest.corpus_version != TRUST_UX_CORPUS_VERSION
        || manifest.corpus_digest != TRUST_UX_CORPUS_DIGEST
        || !valid_digest(&manifest.build_lock_digest)
        || !valid_digest(&manifest.runner_digest)
        || !valid_digest(&manifest.protocol_digest)
        || manifest.prepared_at.trim().is_empty()
        || manifest.os.trim().is_empty()
        || manifest.architecture.trim().is_empty()
        || manifest.terminal_columns == 0
        || manifest.terminal_rows == 0
        || manifest.protocol != "docs/reports/phase-12-trust-ux-protocol.md"
        || manifest.minimum_unique_human_reviewers != 3
        || manifest.task_count_per_reviewer != trust_ux_task_corpus().len()
        || !manifest.raw_sessions_git_ignored
    {
        return Err("Trust UX study manifest identity or preregistration drifted".into());
    }
    Ok(())
}

/// Compute a SHA-256 identity for exact evidence bytes.
pub fn trust_ux_evidence_digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Aggregate sessions against one exact study manifest and seal the result.
pub fn aggregate_trust_ux_study(
    manifest: &TrustUxStudyManifest,
    manifest_bytes: &[u8],
    sessions: &[TrustUxSessionReport],
) -> Result<TrustUxStudyAggregateReceipt, String> {
    validate_trust_ux_study_manifest(manifest)?;
    let study_manifest_digest = trust_ux_evidence_digest(manifest_bytes);
    let aggregate = aggregate_trust_ux_sessions_bound(sessions, Some(&study_manifest_digest));
    let mut session_report_digests = sessions
        .iter()
        .map(|session| session.report_digest.clone())
        .collect::<Vec<_>>();
    if session_report_digests
        .iter()
        .any(|digest| !valid_digest(digest))
    {
        return Err("study contains an invalid session report digest".into());
    }
    session_report_digests.sort();
    let mut receipt = TrustUxStudyAggregateReceipt {
        schema_version: "1.0.0".into(),
        study_manifest_digest,
        build_lock_digest: manifest.build_lock_digest.clone(),
        runner_digest: manifest.runner_digest.clone(),
        protocol_digest: manifest.protocol_digest.clone(),
        session_report_digests,
        aggregate,
        receipt_digest: String::new(),
    };
    receipt.receipt_digest = study_receipt_digest(&receipt)?;
    Ok(receipt)
}

/// Recompute a persisted Gate E receipt from its exact manifest and sessions.
pub fn verify_trust_ux_study_receipt(
    manifest: &TrustUxStudyManifest,
    manifest_bytes: &[u8],
    sessions: &[TrustUxSessionReport],
    receipt: &TrustUxStudyAggregateReceipt,
) -> Result<(), String> {
    let expected = aggregate_trust_ux_study(manifest, manifest_bytes, sessions)?;
    if &expected != receipt {
        return Err("Gate E study receipt does not match manifest or sessions".into());
    }
    Ok(())
}

fn study_receipt_digest(receipt: &TrustUxStudyAggregateReceipt) -> Result<String, String> {
    let mut semantic = receipt.clone();
    semantic.receipt_digest.clear();
    serde_json::to_vec(&semantic)
        .map(|bytes| trust_ux_evidence_digest(&bytes))
        .map_err(|error| error.to_string())
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn validate_pseudonym(label: &str, value: &str) -> Result<(), String> {
    let valid = (3..=32).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "{label} must be a 3-32 character anonymous code using A-Z, 0-9, - or _"
        ))
    }
}

fn session_digest(report: &TrustUxSessionReport) -> String {
    let bytes = serde_json::to_vec(report).expect("Trust UX session JSON");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        Some((values[middle - 1] + values[middle]) / 2.0)
    } else {
        Some(values[middle])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_session(reviewer: &str, kind: TrustUxSessionKind) -> TrustUxSessionReport {
        let results = trust_ux_task_corpus()
            .into_iter()
            .map(|task| TrustUxTaskResult {
                task_id: task.task_id,
                answer_id: Some(task.expected_answer_id),
                correct: true,
                elapsed_seconds: 10.0,
                interaction_count: 1,
                opened_card_ids: vec![task.decisive_card_id],
                interactions_to_decisive_evidence: Some(1),
                aborted: false,
            })
            .collect();
        seal_trust_ux_session(TrustUxSessionReport {
            schema_version: "0.1.0".into(),
            session_kind: kind,
            reviewer_id: reviewer.into(),
            facilitator_id: (kind == TrustUxSessionKind::Human).then(|| "fac-01".into()),
            runner_identity: "test-runner".into(),
            study_manifest_digest: (kind == TrustUxSessionKind::Human)
                .then(|| format!("sha256:{}", "d".repeat(64))),
            corpus_version: TRUST_UX_CORPUS_VERSION.into(),
            corpus_digest: trust_ux_corpus_digest(),
            started_at: "2026-08-23T00:00:00Z".into(),
            finished_at: "2026-08-23T00:03:00Z".into(),
            results,
            report_digest: String::new(),
        })
    }

    #[test]
    fn corpus_has_exactly_twelve_distinct_required_categories() {
        let corpus = trust_ux_task_corpus();
        assert_eq!(trust_ux_corpus_digest(), TRUST_UX_CORPUS_DIGEST);
        assert_eq!(corpus.len(), 12);
        let categories = corpus
            .iter()
            .map(|task| task.category.as_str())
            .collect::<BTreeSet<_>>();
        let expected_positions = corpus
            .iter()
            .map(|task| {
                task.answer_choices
                    .iter()
                    .position(|choice| choice.answer_id == task.expected_answer_id)
                    .unwrap()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(expected_positions.len(), 3);
        for required in [
            "source_drift",
            "edit_conflict",
            "crs_unit_error",
            "adapter_denial",
            "cloud_fallback",
            "changed_result",
            "uncertainty",
            "open_dispute",
        ] {
            assert!(categories.contains(required));
        }
        assert!(corpus.iter().all(|task| {
            task.evidence_cards.len() == 3
                && task
                    .evidence_cards
                    .iter()
                    .any(|card| card.card_id == task.decisive_card_id)
                && task
                    .answer_choices
                    .iter()
                    .any(|choice| choice.answer_id == task.expected_answer_id)
        }));
    }

    #[test]
    fn three_complete_humans_pass_but_automation_never_counts() {
        let sessions = vec![
            complete_session("human-01", TrustUxSessionKind::Human),
            complete_session("human-02", TrustUxSessionKind::Human),
            complete_session("human-03", TrustUxSessionKind::Human),
            complete_session("robot-01", TrustUxSessionKind::Automated),
        ];
        let aggregate = aggregate_trust_ux_sessions(&sessions);
        assert!(aggregate.passed, "{aggregate:#?}");
        assert_eq!(aggregate.admitted_human_reviewers, 3);
        assert_eq!(aggregate.excluded_automated_sessions, 1);
        assert_eq!(aggregate.admitted_tasks, 36);
        assert_eq!(aggregate.median_diagnosis_seconds, Some(10.0));
        let encoded = serde_json::to_vec(&aggregate).unwrap();
        let decoded: TrustUxAggregateReport = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, aggregate);
    }

    #[test]
    fn study_receipt_binds_manifest_runner_protocol_and_sessions() {
        let manifest = TrustUxStudyManifest {
            schema_version: "0.1.0".into(),
            prepared_at: "2026-08-27T00:00:00Z".into(),
            status: "prepared_human_sessions_pending".into(),
            corpus_version: TRUST_UX_CORPUS_VERSION.into(),
            corpus_digest: TRUST_UX_CORPUS_DIGEST.into(),
            build_lock_digest: format!("sha256:{}", "a".repeat(64)),
            runner_digest: format!("sha256:{}", "b".repeat(64)),
            protocol_digest: format!("sha256:{}", "c".repeat(64)),
            os: "test-os".into(),
            architecture: "x86_64".into(),
            terminal_columns: 120,
            terminal_rows: 30,
            protocol: "docs/reports/phase-12-trust-ux-protocol.md".into(),
            minimum_unique_human_reviewers: 3,
            task_count_per_reviewer: 12,
            raw_sessions_git_ignored: true,
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("manifest");
        let manifest_digest = trust_ux_evidence_digest(&manifest_bytes);
        let mut sessions = ["human-01", "human-02", "human-03"]
            .map(|reviewer| complete_session(reviewer, TrustUxSessionKind::Human));
        for session in &mut sessions {
            session.study_manifest_digest = Some(manifest_digest.clone());
            *session = seal_trust_ux_session(session.clone());
        }
        let receipt =
            aggregate_trust_ux_study(&manifest, &manifest_bytes, &sessions).expect("study receipt");
        assert!(receipt.aggregate.passed);
        let encoded = serde_json::to_vec(&receipt).expect("receipt JSON");
        let roundtrip: TrustUxStudyAggregateReceipt =
            serde_json::from_slice(&encoded).expect("receipt roundtrip");
        verify_trust_ux_study_receipt(&manifest, &manifest_bytes, &sessions, &roundtrip)
            .expect("verify persisted receipt");

        let mut drifted_bytes = manifest_bytes.clone();
        drifted_bytes.push(b'\n');
        assert!(
            verify_trust_ux_study_receipt(&manifest, &drifted_bytes, &sessions, &receipt).is_err()
        );

        sessions[0].study_manifest_digest = Some(format!("sha256:{}", "e".repeat(64)));
        sessions[0] = seal_trust_ux_session(sessions[0].clone());
        let rejected = aggregate_trust_ux_study(&manifest, &manifest_bytes, &sessions)
            .expect("rejected aggregate remains auditable");
        assert!(!rejected.aggregate.passed);
        assert_eq!(rejected.aggregate.rejected_sessions, 1);
    }

    #[test]
    fn tamper_duplicate_incomplete_and_missing_decisive_evidence_fail_closed() {
        let valid = complete_session("human-01", TrustUxSessionKind::Human);
        let mut tampered = valid.clone();
        tampered.results[0].correct = false;
        assert!(validate_trust_ux_session(&tampered).is_err());

        let mut incomplete = complete_session("human-02", TrustUxSessionKind::Human);
        incomplete.results.pop();
        incomplete = seal_trust_ux_session(incomplete);

        let mut guessed = complete_session("human-03", TrustUxSessionKind::Human);
        guessed.results[0].interactions_to_decisive_evidence = None;
        guessed = seal_trust_ux_session(guessed);

        let aggregate = aggregate_trust_ux_sessions(&[valid.clone(), valid, incomplete, guessed]);
        assert!(!aggregate.passed);
        assert_eq!(aggregate.admitted_human_reviewers, 1);
        assert_eq!(aggregate.rejected_sessions, 3);
    }

    #[test]
    fn no_human_evidence_serializes_missing_medians_as_null_and_never_passes() {
        let automated = complete_session("robot-01", TrustUxSessionKind::Automated);
        let aggregate = aggregate_trust_ux_sessions(&[automated]);
        assert_eq!(aggregate.median_diagnosis_seconds, None);
        assert_eq!(aggregate.median_interactions_to_decisive_evidence, None);
        assert!(!aggregate.passed);
        let encoded = serde_json::to_vec(&aggregate).unwrap();
        let decoded: TrustUxAggregateReport = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, aggregate);
    }

    #[test]
    fn real_cli_automated_abort_fixture_is_valid_but_excluded() {
        let session: TrustUxSessionReport = serde_json::from_str(include_str!(
            "../fixtures/phase-12-trust-ux-automated-abort.json"
        ))
        .unwrap();
        validate_trust_ux_session(&session).unwrap();
        let aggregate = aggregate_trust_ux_sessions(&[session]);
        assert_eq!(aggregate.excluded_automated_sessions, 1);
        assert_eq!(aggregate.aborts, 1);
        assert_eq!(aggregate.admitted_human_reviewers, 0);
        assert!(!aggregate.passed);
    }

    #[test]
    fn correctness_time_and_interaction_thresholds_fail_independently() {
        let baseline = vec![
            complete_session("human-01", TrustUxSessionKind::Human),
            complete_session("human-02", TrustUxSessionKind::Human),
            complete_session("human-03", TrustUxSessionKind::Human),
        ];

        let mut inaccurate = baseline.clone();
        for result in inaccurate[0].results.iter_mut().take(4) {
            let task = trust_ux_task_corpus()
                .into_iter()
                .find(|task| task.task_id == result.task_id)
                .unwrap();
            let expected_answer_id = task.expected_answer_id;
            result.answer_id = Some(
                task.answer_choices
                    .into_iter()
                    .find(|choice| choice.answer_id != expected_answer_id)
                    .unwrap()
                    .answer_id,
            );
            result.correct = false;
        }
        inaccurate[0] = seal_trust_ux_session(inaccurate[0].clone());
        let aggregate = aggregate_trust_ux_sessions(&inaccurate);
        assert!(!aggregate.correctness_gate);
        assert!(aggregate.time_gate && aggregate.interaction_gate);
        assert!(!aggregate.passed);

        let mut slow = baseline.clone();
        for session in &mut slow {
            for result in &mut session.results {
                result.elapsed_seconds = 121.0;
            }
            *session = seal_trust_ux_session(session.clone());
        }
        let aggregate = aggregate_trust_ux_sessions(&slow);
        assert!(!aggregate.time_gate);
        assert!(aggregate.correctness_gate && aggregate.interaction_gate);

        let mut indirect = baseline;
        for session in &mut indirect {
            for result in &mut session.results {
                let task = trust_ux_task_corpus()
                    .into_iter()
                    .find(|task| task.task_id == result.task_id)
                    .unwrap();
                let non_decisive = task
                    .evidence_cards
                    .iter()
                    .find(|card| card.card_id != task.decisive_card_id)
                    .unwrap()
                    .card_id
                    .clone();
                result.interaction_count = 3;
                result.opened_card_ids =
                    vec![non_decisive.clone(), non_decisive, task.decisive_card_id];
                result.interactions_to_decisive_evidence = Some(3);
            }
            *session = seal_trust_ux_session(session.clone());
        }
        let aggregate = aggregate_trust_ux_sessions(&indirect);
        assert!(!aggregate.interaction_gate);
        assert!(aggregate.correctness_gate && aggregate.time_gate);
    }
}
