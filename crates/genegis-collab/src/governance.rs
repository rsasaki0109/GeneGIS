//! Policy-driven organization roles, approvals, retention, and audit export.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// A capability granted by an organization role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceCapability {
    View,
    Execute,
    Publish,
    ManageProjects,
    ManageMembership,
    Approve,
    ExportAudit,
    ManageRetention,
}

/// A governed operation that may require approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernedAction {
    ViewProject,
    ExecuteWorkflow,
    PublishResult,
    ChangeProjectPolicy,
    ChangeMembership,
    ExportAudit,
    ApplyRetention,
}

impl GovernedAction {
    fn capability(self) -> GovernanceCapability {
        match self {
            Self::ViewProject => GovernanceCapability::View,
            Self::ExecuteWorkflow => GovernanceCapability::Execute,
            Self::PublishResult => GovernanceCapability::Publish,
            Self::ChangeProjectPolicy => GovernanceCapability::ManageProjects,
            Self::ChangeMembership => GovernanceCapability::ManageMembership,
            Self::ExportAudit => GovernanceCapability::ExportAudit,
            Self::ApplyRetention => GovernanceCapability::ManageRetention,
        }
    }
}

/// Named role and its exact capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganizationRole {
    pub id: String,
    pub capabilities: BTreeSet<GovernanceCapability>,
}

/// Organization member bound to a role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganizationMember {
    pub subject_id: String,
    pub role_id: String,
    pub active: bool,
}

/// Project ownership and data classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganizationProject {
    pub project_id: String,
    pub organization_id: String,
    pub classification: String,
}

/// Retention rules applied to auditable records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionPolicy {
    pub minimum_days: u32,
    pub maximum_days: u32,
    pub protected_classes: BTreeSet<String>,
}

/// Versioned organization policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganizationPolicy {
    pub schema_version: String,
    pub organization_id: String,
    pub policy_version: String,
    pub roles: BTreeMap<String, OrganizationRole>,
    /// Minimum distinct approvers by action. Zero means no approval gate.
    pub approval_thresholds: BTreeMap<GovernedAction, u16>,
    pub retention: RetentionPolicy,
}

/// One pending or completed approval chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingApproval {
    pub id: String,
    pub project_id: String,
    pub action: GovernedAction,
    pub resource_digest: String,
    pub requested_by: String,
    pub requested_at: String,
    pub required_approvals: u16,
    pub approvers: BTreeSet<String>,
    pub completed_at: Option<String>,
    pub approval_digest: String,
}

/// Append-only, hash-chained audit event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEvent {
    pub sequence: u64,
    pub occurred_at: String,
    pub actor_id: String,
    pub project_id: Option<String>,
    pub event_type: String,
    pub subject_digest: String,
    pub previous_event_digest: Option<String>,
    pub event_digest: String,
}

/// Authorization outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum GovernanceDecision {
    Authorized { audit_event_digest: String },
    ApprovalRequired { approval: PendingApproval },
    Approved { approval: PendingApproval },
}

/// One record considered by retention policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionRecord {
    pub record_id: String,
    pub class: String,
    pub created_at: String,
    pub legal_hold: bool,
    pub digest: String,
}

/// Non-destructive retention decision. Deletion execution is a separate command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "disposition")]
pub enum RetentionDisposition {
    Keep { record_id: String, reason: String },
    EligibleForDeletion { record_id: String, digest: String },
}

/// Portable audit export with chain and policy identities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditExport {
    pub schema_version: String,
    pub organization_id: String,
    pub policy_digest: String,
    pub exported_at: String,
    pub events: Vec<AuditEvent>,
    pub export_digest: String,
}

/// Organization governance state shared by desktop, browser, and server hosts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GovernanceState {
    pub policy: OrganizationPolicy,
    pub policy_digest: String,
    pub members: BTreeMap<String, OrganizationMember>,
    pub projects: BTreeMap<String, OrganizationProject>,
    pub approvals: BTreeMap<String, PendingApproval>,
    pub audit_events: Vec<AuditEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GovernanceError {
    #[error("invalid governance contract: {0}")]
    Invalid(String),
    #[error("governance access denied: {0}")]
    Denied(String),
    #[error("governance digest mismatch")]
    Digest,
    #[error("governance serialization failed: {0}")]
    Serialization(String),
}

impl GovernanceState {
    /// Validate and create an empty policy-governed organization state.
    pub fn new(policy: OrganizationPolicy) -> Result<Self, GovernanceError> {
        validate_policy(&policy)?;
        let policy_digest = semantic_digest(&policy)?;
        Ok(Self {
            policy,
            policy_digest,
            members: BTreeMap::new(),
            projects: BTreeMap::new(),
            approvals: BTreeMap::new(),
            audit_events: Vec::new(),
        })
    }

    pub fn add_member(&mut self, member: OrganizationMember) -> Result<(), GovernanceError> {
        if member.subject_id.trim().is_empty() || !self.policy.roles.contains_key(&member.role_id) {
            return Err(GovernanceError::Invalid(
                "member identity or role is invalid".into(),
            ));
        }
        self.members.insert(member.subject_id.clone(), member);
        Ok(())
    }

    pub fn add_project(&mut self, project: OrganizationProject) -> Result<(), GovernanceError> {
        if project.project_id.trim().is_empty()
            || project.organization_id != self.policy.organization_id
            || project.classification.trim().is_empty()
        {
            return Err(GovernanceError::Invalid(
                "project ownership is invalid".into(),
            ));
        }
        self.projects.insert(project.project_id.clone(), project);
        Ok(())
    }

    /// Authorize an operation or create an exact digest-bound approval request.
    pub fn authorize(
        &mut self,
        actor_id: &str,
        project_id: &str,
        action: GovernedAction,
        resource_digest: &str,
        occurred_at: &str,
    ) -> Result<GovernanceDecision, GovernanceError> {
        parse_time(occurred_at)?;
        require_digest(resource_digest)?;
        self.require_project(project_id)?;
        self.require_capability(actor_id, action.capability())?;
        let threshold = self
            .policy
            .approval_thresholds
            .get(&action)
            .copied()
            .unwrap_or(0);
        if threshold == 0 {
            let event = self.append_event(
                actor_id,
                Some(project_id),
                "authorized",
                resource_digest,
                occurred_at,
            )?;
            return Ok(GovernanceDecision::Authorized {
                audit_event_digest: event.event_digest,
            });
        }
        let id = semantic_digest(&(project_id, action, resource_digest, actor_id, occurred_at))?;
        let mut approval = PendingApproval {
            id: id.clone(),
            project_id: project_id.into(),
            action,
            resource_digest: resource_digest.into(),
            requested_by: actor_id.into(),
            requested_at: occurred_at.into(),
            required_approvals: threshold,
            approvers: BTreeSet::new(),
            completed_at: None,
            approval_digest: String::new(),
        };
        approval.approval_digest = approval_digest(&approval)?;
        self.approvals.insert(id, approval.clone());
        self.append_event(
            actor_id,
            Some(project_id),
            "approval_requested",
            &approval.approval_digest,
            occurred_at,
        )?;
        Ok(GovernanceDecision::ApprovalRequired { approval })
    }

    /// Add one distinct authorized approver and close the request at its threshold.
    pub fn approve(
        &mut self,
        approval_id: &str,
        approver_id: &str,
        occurred_at: &str,
    ) -> Result<GovernanceDecision, GovernanceError> {
        parse_time(occurred_at)?;
        self.require_capability(approver_id, GovernanceCapability::Approve)?;
        let approval = self
            .approvals
            .get_mut(approval_id)
            .ok_or_else(|| GovernanceError::Invalid("unknown approval".into()))?;
        if approval.completed_at.is_some()
            || approval.requested_by == approver_id
            || !approval.approvers.insert(approver_id.into())
        {
            return Err(GovernanceError::Denied(
                "approval must be pending, distinct, and independent".into(),
            ));
        }
        if approval.approvers.len() >= approval.required_approvals as usize {
            approval.completed_at = Some(occurred_at.into());
        }
        approval.approval_digest = approval_digest(approval)?;
        let result = approval.clone();
        let event_type = if result.completed_at.is_some() {
            "approval_completed"
        } else {
            "approval_recorded"
        };
        self.append_event(
            approver_id,
            Some(&result.project_id),
            event_type,
            &result.approval_digest,
            occurred_at,
        )?;
        if result.completed_at.is_some() {
            Ok(GovernanceDecision::Approved { approval: result })
        } else {
            Ok(GovernanceDecision::ApprovalRequired { approval: result })
        }
    }

    /// Evaluate records without deleting data.
    pub fn retention_plan(
        &self,
        records: &[RetentionRecord],
        as_of: &str,
    ) -> Result<Vec<RetentionDisposition>, GovernanceError> {
        let as_of = parse_time(as_of)?;
        records
            .iter()
            .map(|record| {
                require_digest(&record.digest)?;
                let created = parse_time(&record.created_at)?;
                let age = as_of.signed_duration_since(created);
                if age < Duration::zero() {
                    return Err(GovernanceError::Invalid(
                        "record timestamp is in the future".into(),
                    ));
                }
                let protected = record.legal_hold
                    || self
                        .policy
                        .retention
                        .protected_classes
                        .contains(&record.class);
                if protected || age.num_days() < i64::from(self.policy.retention.maximum_days) {
                    Ok(RetentionDisposition::Keep {
                        record_id: record.record_id.clone(),
                        reason: if protected {
                            "protected_or_legal_hold".into()
                        } else {
                            "within_retention_window".into()
                        },
                    })
                } else {
                    Ok(RetentionDisposition::EligibleForDeletion {
                        record_id: record.record_id.clone(),
                        digest: record.digest.clone(),
                    })
                }
            })
            .collect()
    }

    /// Export the complete verifiable audit chain after an explicit capability check.
    pub fn export_audit(
        &mut self,
        actor_id: &str,
        exported_at: &str,
    ) -> Result<AuditExport, GovernanceError> {
        parse_time(exported_at)?;
        self.require_capability(actor_id, GovernanceCapability::ExportAudit)?;
        let policy_digest = self.policy_digest.clone();
        self.append_event(
            actor_id,
            None,
            "audit_exported",
            &policy_digest,
            exported_at,
        )?;
        verify_audit_chain(&self.audit_events)?;
        let mut export = AuditExport {
            schema_version: "0.1.0".into(),
            organization_id: self.policy.organization_id.clone(),
            policy_digest,
            exported_at: exported_at.into(),
            events: self.audit_events.clone(),
            export_digest: String::new(),
        };
        export.export_digest = semantic_digest(&export)?;
        Ok(export)
    }

    fn require_project(&self, project_id: &str) -> Result<(), GovernanceError> {
        self.projects
            .get(project_id)
            .filter(|p| p.organization_id == self.policy.organization_id)
            .map(|_| ())
            .ok_or_else(|| {
                GovernanceError::Denied("project is outside the organization boundary".into())
            })
    }

    fn require_capability(
        &self,
        subject_id: &str,
        capability: GovernanceCapability,
    ) -> Result<(), GovernanceError> {
        let member = self
            .members
            .get(subject_id)
            .filter(|member| member.active)
            .ok_or_else(|| GovernanceError::Denied("subject is not an active member".into()))?;
        let role =
            self.policy.roles.get(&member.role_id).ok_or_else(|| {
                GovernanceError::Denied("member role is absent from policy".into())
            })?;
        if !role.capabilities.contains(&capability) {
            return Err(GovernanceError::Denied(
                "role does not grant the required capability".into(),
            ));
        }
        Ok(())
    }

    fn append_event(
        &mut self,
        actor: &str,
        project: Option<&str>,
        event_type: &str,
        subject_digest: &str,
        occurred_at: &str,
    ) -> Result<AuditEvent, GovernanceError> {
        let mut event = AuditEvent {
            sequence: self.audit_events.len() as u64 + 1,
            occurred_at: occurred_at.into(),
            actor_id: actor.into(),
            project_id: project.map(str::to_owned),
            event_type: event_type.into(),
            subject_digest: subject_digest.into(),
            previous_event_digest: self
                .audit_events
                .last()
                .map(|event| event.event_digest.clone()),
            event_digest: String::new(),
        };
        event.event_digest = audit_event_digest(&event)?;
        self.audit_events.push(event.clone());
        Ok(event)
    }
}

pub fn verify_audit_chain(events: &[AuditEvent]) -> Result<(), GovernanceError> {
    let mut previous: Option<&str> = None;
    for (index, event) in events.iter().enumerate() {
        if event.sequence != index as u64 + 1
            || event.previous_event_digest.as_deref() != previous
            || audit_event_digest(event)? != event.event_digest
        {
            return Err(GovernanceError::Digest);
        }
        previous = Some(&event.event_digest);
    }
    Ok(())
}

fn validate_policy(policy: &OrganizationPolicy) -> Result<(), GovernanceError> {
    if policy.schema_version != "0.1.0"
        || policy.organization_id.trim().is_empty()
        || policy.policy_version.trim().is_empty()
        || policy.roles.is_empty()
        || policy.retention.minimum_days > policy.retention.maximum_days
        || policy.retention.maximum_days == 0
        || policy
            .roles
            .iter()
            .any(|(id, role)| id != &role.id || id.trim().is_empty())
    {
        return Err(GovernanceError::Invalid(
            "organization policy is invalid".into(),
        ));
    }
    Ok(())
}

fn approval_digest(approval: &PendingApproval) -> Result<String, GovernanceError> {
    let mut semantic = approval.clone();
    semantic.approval_digest.clear();
    semantic_digest(&semantic)
}

fn audit_event_digest(event: &AuditEvent) -> Result<String, GovernanceError> {
    let mut semantic = event.clone();
    semantic.event_digest.clear();
    semantic_digest(&semantic)
}

fn semantic_digest<T: Serialize>(value: &T) -> Result<String, GovernanceError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| GovernanceError::Serialization(error.to_string()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, GovernanceError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| GovernanceError::Invalid("timestamp must be RFC 3339".into()))
}

fn require_digest(value: &str) -> Result<(), GovernanceError> {
    if value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        Ok(())
    } else {
        Err(GovernanceError::Invalid("digest must be SHA-256".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> GovernanceState {
        let roles = BTreeMap::from([
            (
                "analyst".into(),
                OrganizationRole {
                    id: "analyst".into(),
                    capabilities: BTreeSet::from([
                        GovernanceCapability::View,
                        GovernanceCapability::Execute,
                        GovernanceCapability::Publish,
                    ]),
                },
            ),
            (
                "reviewer".into(),
                OrganizationRole {
                    id: "reviewer".into(),
                    capabilities: BTreeSet::from([
                        GovernanceCapability::Approve,
                        GovernanceCapability::ExportAudit,
                    ]),
                },
            ),
        ]);
        let mut state = GovernanceState::new(OrganizationPolicy {
            schema_version: "0.1.0".into(),
            organization_id: "org-1".into(),
            policy_version: "1".into(),
            roles,
            approval_thresholds: BTreeMap::from([(GovernedAction::PublishResult, 1)]),
            retention: RetentionPolicy {
                minimum_days: 30,
                maximum_days: 365,
                protected_classes: BTreeSet::from(["provenance".into()]),
            },
        })
        .expect("policy");
        state
            .add_member(OrganizationMember {
                subject_id: "alice".into(),
                role_id: "analyst".into(),
                active: true,
            })
            .expect("analyst");
        state
            .add_member(OrganizationMember {
                subject_id: "bob".into(),
                role_id: "reviewer".into(),
                active: true,
            })
            .expect("reviewer");
        state
            .add_project(OrganizationProject {
                project_id: "project-1".into(),
                organization_id: "org-1".into(),
                classification: "internal".into(),
            })
            .expect("project");
        state
    }

    #[test]
    fn enforces_role_independent_approval_retention_and_audit_chain() {
        let mut state = state();
        let digest = format!("sha256:{}", "a".repeat(64));
        let requested = state
            .authorize(
                "alice",
                "project-1",
                GovernedAction::PublishResult,
                &digest,
                "2026-08-26T10:00:00Z",
            )
            .expect("request");
        let GovernanceDecision::ApprovalRequired { approval } = requested else {
            panic!("approval required")
        };
        assert!(state
            .approve(&approval.id, "alice", "2026-08-26T10:01:00Z")
            .is_err());
        assert!(matches!(
            state
                .approve(&approval.id, "bob", "2026-08-26T10:02:00Z")
                .expect("approve"),
            GovernanceDecision::Approved { .. }
        ));
        let plan = state
            .retention_plan(
                &[
                    RetentionRecord {
                        record_id: "old".into(),
                        class: "cache".into(),
                        created_at: "2025-01-01T00:00:00Z".into(),
                        legal_hold: false,
                        digest: digest.clone(),
                    },
                    RetentionRecord {
                        record_id: "proof".into(),
                        class: "provenance".into(),
                        created_at: "2025-01-01T00:00:00Z".into(),
                        legal_hold: false,
                        digest,
                    },
                ],
                "2026-08-26T11:00:00Z",
            )
            .expect("retention");
        assert!(matches!(
            plan[0],
            RetentionDisposition::EligibleForDeletion { .. }
        ));
        assert!(matches!(plan[1], RetentionDisposition::Keep { .. }));
        let export = state
            .export_audit("bob", "2026-08-26T12:00:00Z")
            .expect("export");
        verify_audit_chain(&export.events).expect("chain");
    }

    #[test]
    fn denies_cross_boundary_and_tampered_audit() {
        let mut state = state();
        let digest = format!("sha256:{}", "b".repeat(64));
        assert!(state
            .authorize(
                "bob",
                "project-1",
                GovernedAction::ExecuteWorkflow,
                &digest,
                "2026-08-26T10:00:00Z"
            )
            .is_err());
        assert!(state
            .authorize(
                "alice",
                "other",
                GovernedAction::ExecuteWorkflow,
                &digest,
                "2026-08-26T10:00:00Z"
            )
            .is_err());
        state
            .authorize(
                "alice",
                "project-1",
                GovernedAction::ExecuteWorkflow,
                &digest,
                "2026-08-26T10:00:00Z",
            )
            .expect("authorized");
        state.audit_events[0].actor_id = "mallory".into();
        assert!(verify_audit_chain(&state.audit_events).is_err());
    }
}
