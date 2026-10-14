//! Independent, pure BuildRunner scheduler-entry coverage reconciliation.
//! This inventory is not an outer Cargo launch inventory or a compiler graph.
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PROFILE: &str = "buildrunner-scheduler-entry-v1";
pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_ENTRIES: usize = 256;
pub const MAX_ID_BYTES: usize = 128;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub profile: String,
    pub run_id: String,
    pub authority_root: String,
    pub owners: Vec<Owner>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub invocation_id: String,
    pub parent_invocation_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub sealed: bool,
    pub interrupted: bool,
    pub lost_events: u64,
    pub outcome: OwnerOutcome,
    pub tickets: Vec<Ticket>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerOutcome {
    Pending,
    Success,
    Failed,
    Cancelled,
    Interrupted,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ticket {
    pub invocation_id: String,
    pub parent_task_id: String,
    pub policy: TicketPolicy,
    pub lifecycle: TicketLifecycle,
    pub execute_entered: bool,
    pub capture_id: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketPolicy {
    Required,
    Opaque,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketLifecycle {
    Declared,
    NotStarted,
    ExecuteEntered,
    Succeeded,
    Failed,
    Cancelled,
    Opaque,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptEnvelope {
    pub schema: u32,
    pub run_id: String,
    pub invocation_id: String,
    pub parent_invocation_id: String,
    pub parent_task_id: String,
    pub capture_id: String,
    pub receipt_schema: u32,
}
/// Construct only after independently validating the complete scheduler receipt.
/// A summary is not a substitute for validation of the underlying receipt.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptSummary {
    pub schema: u32,
    pub capture_id: String,
    pub structural_complete: bool,
    pub full_pipeline_complete: bool,
}
#[derive(Debug, Serialize)]
pub struct CoverageAssessment {
    pub schema: u32,
    pub coverage: String,
    pub profile: String,
    pub run_id: String,
    pub authority_root: String,
    pub excluded_authority_roots: Vec<String>,
    pub outcome_agreement_verified: bool,
    pub scheduler_entry_complete: bool,
    pub full_pipeline_complete: bool,
    pub expected_receipts: usize,
    pub accounted_receipts: usize,
    pub issues: Vec<String>,
    pub opaque_boundaries: Vec<String>,
}
fn id(s: &str) -> Result<()> {
    if s.is_empty()
        || s.len() > MAX_ID_BYTES
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
    {
        bail!("invalid or oversized identity");
    }
    Ok(())
}
fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if bytes.len() > MAX_BYTES {
        bail!("manifest protocol byte budget exceeded");
    }
    Ok(serde_json::from_slice(bytes)?)
}
pub fn parse_manifest(bytes: &[u8]) -> Result<Manifest> {
    parse(bytes)
}
pub fn parse_envelope(bytes: &[u8]) -> Result<ReceiptEnvelope> {
    parse(bytes)
}

/// Malformed/unsupported inventories return Err. Supported but unfinished or
/// incomplete lifecycles return a non-complete assessment with explicit issues.
/// No IO, filename inference, clock comparison or compiler invocation synthesis.
pub fn reconcile(
    m: &Manifest,
    envelopes: &[ReceiptEnvelope],
    receipts: &[ReceiptSummary],
) -> Result<CoverageAssessment> {
    if m.schema != 1 || m.profile != PROFILE {
        bail!("unsupported manifest schema or profile");
    }
    id(&m.run_id)?;
    id(&m.authority_root)?;
    if m.owners.is_empty()
        || m.owners.len() > MAX_ENTRIES
        || envelopes.len() > MAX_ENTRIES
        || receipts.len() > MAX_ENTRIES
    {
        bail!("manifest entry budget exceeded or empty inventory");
    }
    let mut identities = BTreeSet::new();
    let mut owners = BTreeMap::new();
    for owner in &m.owners {
        id(&owner.invocation_id)?;
        if !identities.insert(owner.invocation_id.as_str()) {
            bail!("duplicate owner invocation identity");
        }
        owners.insert(owner.invocation_id.as_str(), owner);
    }
    let root = owners
        .get(m.authority_root.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing authority root"))?;
    if root.parent_invocation_id.is_some() || root.parent_task_id.is_some() {
        bail!("authority root has parent");
    }
    let mut tickets = BTreeMap::new();
    let mut captures = BTreeSet::new();
    let mut issues = Vec::new();
    for owner in &m.owners {
        if owner.invocation_id != m.authority_root {
            let parent = owner
                .parent_invocation_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("nonroot owner missing parent"))?;
            id(parent)?;
            id(owner
                .parent_task_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("nonroot owner missing parent task"))?)?;
            if !owners.contains_key(parent) {
                bail!("foreign owner parent");
            }
            let mut seen = BTreeSet::new();
            let mut cursor = owner;
            while let Some(parent) = cursor.parent_invocation_id.as_deref() {
                if !seen.insert(cursor.invocation_id.as_str()) {
                    bail!("owner parent cycle");
                }
                cursor = owners
                    .get(parent)
                    .ok_or_else(|| anyhow::anyhow!("foreign owner parent"))?;
            }
            if cursor.invocation_id != m.authority_root {
                bail!("disconnected authority root");
            }
        }
        if owner.sealed && owner.outcome == OwnerOutcome::Pending {
            bail!("sealed owner cannot have pending outcome");
        }
        if !owner.sealed {
            issues.push(format!("owner {} is unsealed", owner.invocation_id));
        }
        if owner.interrupted || owner.outcome == OwnerOutcome::Interrupted {
            issues.push(format!("owner {} interrupted", owner.invocation_id));
        }
        if owner.interrupted != (owner.outcome == OwnerOutcome::Interrupted) {
            bail!("conflicting owner interruption outcome");
        }
        if owner.lost_events != 0 {
            issues.push(format!("owner {} lost events", owner.invocation_id));
        }
        // This profile has exactly one scheduler slot per BuildRunner entry.
        if owner.tickets.len() != 1 {
            bail!("profile requires exactly one scheduler ticket per owner");
        }
        for ticket in &owner.tickets {
            id(&ticket.invocation_id)?;
            id(&ticket.parent_task_id)?;
            if !identities.insert(ticket.invocation_id.as_str()) {
                bail!("duplicate invocation identity");
            }
            if ticket.policy != TicketPolicy::Required
                || ticket.lifecycle == TicketLifecycle::Opaque
            {
                bail!("required scheduler slot cannot become opaque");
            }
            if let Some(capture) = &ticket.capture_id {
                id(capture)?;
                if !captures.insert(capture.as_str()) {
                    bail!("duplicate capture identity");
                }
            }
            if matches!(
                ticket.lifecycle,
                TicketLifecycle::Declared | TicketLifecycle::NotStarted
            ) && ticket.execute_entered
            {
                bail!("unstarted lifecycle conflicts with execute-enter");
            }
            if matches!(
                ticket.lifecycle,
                TicketLifecycle::ExecuteEntered
                    | TicketLifecycle::Succeeded
                    | TicketLifecycle::Failed
                    | TicketLifecycle::Cancelled
            ) && !ticket.execute_entered
            {
                bail!("execution lifecycle lacks execute-enter");
            }
            let terminal = matches!(
                ticket.lifecycle,
                TicketLifecycle::Succeeded | TicketLifecycle::Failed | TicketLifecycle::Cancelled
            );
            if !terminal {
                issues.push(format!(
                    "ticket {} has unfinished lifecycle {:?}",
                    ticket.invocation_id, ticket.lifecycle
                ));
            }
            if terminal && ticket.capture_id.is_none() {
                bail!("terminal scheduler ticket lacks capture identity");
            }
            if owner.sealed && !terminal && ticket.lifecycle != TicketLifecycle::NotStarted {
                bail!("sealed owner has nonterminal scheduler ticket");
            }
            if owner.sealed && ticket.capture_id.is_none() {
                bail!("sealed owner lacks bound scheduler capture");
            }
            if owner.outcome == OwnerOutcome::Success
                && owner.sealed
                && ticket.lifecycle != TicketLifecycle::Succeeded
            {
                bail!("successful owner has unsuccessful scheduler lifecycle");
            }
            tickets.insert(ticket.invocation_id.as_str(), (owner, ticket));
        }
    }
    let mut summaries = BTreeMap::new();
    for receipt in receipts {
        id(&receipt.capture_id)?;
        if receipt.schema != 1 || receipt.full_pipeline_complete {
            bail!("unsupported receipt schema or full pipeline claim");
        }
        if summaries
            .insert(receipt.capture_id.as_str(), receipt)
            .is_some()
        {
            bail!("duplicate receipt identity");
        }
    }
    let mut bound = BTreeMap::new();
    let mut bound_captures = BTreeSet::new();
    for envelope in envelopes {
        for value in [
            &envelope.run_id,
            &envelope.invocation_id,
            &envelope.parent_invocation_id,
            &envelope.parent_task_id,
            &envelope.capture_id,
        ] {
            id(value)?;
        }
        if envelope.schema != 1 || envelope.receipt_schema != 1 {
            bail!("unsupported envelope schema");
        }
        if envelope.run_id != m.run_id {
            bail!("foreign envelope run identity");
        }
        let (owner, ticket) = tickets
            .get(envelope.invocation_id.as_str())
            .ok_or_else(|| anyhow::anyhow!("orphan envelope or unexpected producer"))?;
        if envelope.parent_invocation_id != owner.invocation_id
            || envelope.parent_task_id != ticket.parent_task_id
            || ticket.capture_id.as_deref() != Some(envelope.capture_id.as_str())
        {
            bail!("conflicting envelope parent or capture identity");
        }
        if !bound_captures.insert(envelope.capture_id.as_str())
            || bound
                .insert(envelope.invocation_id.as_str(), envelope)
                .is_some()
        {
            bail!("duplicate envelope identity");
        }
        if !summaries.contains_key(envelope.capture_id.as_str()) {
            bail!("orphan envelope without receipt summary");
        }
        if ticket.lifecycle == TicketLifecycle::Declared {
            bail!("receipt conflicts with undeclared start lifecycle");
        }
        if !ticket.execute_entered && summaries[envelope.capture_id.as_str()].structural_complete {
            bail!("pre-execute receipt cannot claim structural completeness");
        }
    }
    for capture in summaries.keys() {
        if !bound_captures.contains(capture) {
            bail!("receipt lacks explicit envelope identity");
        }
    }
    let mut accounted = 0;
    for (invocation, (_, ticket)) in &tickets {
        match bound.get(invocation) {
            None => issues.push(format!("missing expected receipt for {invocation}")),
            Some(envelope) => {
                if summaries[envelope.capture_id.as_str()].structural_complete {
                    accounted += 1;
                } else {
                    issues.push(format!("invalid receipt for {}", ticket.invocation_id));
                }
            }
        }
    }
    Ok(CoverageAssessment {
        schema: 1,
        coverage: "buildrunner-scheduler-entry-coverage".into(),
        profile: PROFILE.into(),
        run_id: m.run_id.clone(),
        authority_root: m.authority_root.clone(),
        excluded_authority_roots: Vec::new(),
        outcome_agreement_verified: false,
        scheduler_entry_complete: issues.is_empty(),
        full_pipeline_complete: false,
        expected_receipts: tickets.len(),
        accounted_receipts: accounted,
        issues,
        opaque_boundaries: [
            "outer-cargo-launch",
            "command-setup-and-resolution",
            "compiler-queries",
            "build-scripts",
            "proc-macros",
            "native-tools",
            "linking",
            "tests",
            "fingerprints",
            "waits",
            "client-cache",
            "remote-service",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Manifest, Vec<ReceiptEnvelope>, Vec<ReceiptSummary>) {
        let m = parse_manifest(br#"{"schema":1,"profile":"buildrunner-scheduler-entry-v1","run_id":"r","authority_root":"o","owners":[{"invocation_id":"o","parent_invocation_id":null,"parent_task_id":null,"sealed":true,"interrupted":false,"lost_events":0,"outcome":"success","tickets":[{"invocation_id":"t","parent_task_id":"scheduler","policy":"required","lifecycle":"succeeded","execute_entered":true,"capture_id":"c"}]}]}"#).unwrap();
        let e = parse_envelope(br#"{"schema":1,"run_id":"r","invocation_id":"t","parent_invocation_id":"o","parent_task_id":"scheduler","capture_id":"c","receipt_schema":1}"#).unwrap();
        let s = ReceiptSummary {
            schema: 1,
            capture_id: "c".into(),
            structural_complete: true,
            full_pipeline_complete: false,
        };
        (m, vec![e], vec![s])
    }
    #[test]
    fn synthetic_two_expected_receipts_require_both() {
        let (mut m, mut e, mut s) = fixture();
        let mut owner = m.owners[0].clone();
        owner.invocation_id = "o2".into();
        owner.parent_invocation_id = Some("o".into());
        owner.parent_task_id = Some("nested".into());
        owner.tickets[0].invocation_id = "t2".into();
        owner.tickets[0].capture_id = Some("c2".into());
        m.owners.push(owner);
        let mut envelope = e[0].clone();
        envelope.invocation_id = "t2".into();
        envelope.parent_invocation_id = "o2".into();
        envelope.capture_id = "c2".into();
        e.push(envelope);
        let mut summary = s[0].clone();
        summary.capture_id = "c2".into();
        s.push(summary);
        let a = reconcile(&m, &e, &s).unwrap();
        assert!(a.scheduler_entry_complete);
        assert!(!a.full_pipeline_complete);
        assert!(!a.outcome_agreement_verified);
        assert_eq!(a.authority_root, "o");
        assert!(a.excluded_authority_roots.is_empty());
        assert_eq!(a.coverage, "buildrunner-scheduler-entry-coverage");
        for index in 0..2 {
            let mut ee = e.clone();
            let mut ss = s.clone();
            ee.remove(index);
            ss.remove(index);
            let a = reconcile(&m, &ee, &ss).unwrap();
            assert!(!a.scheduler_entry_complete);
            assert_eq!(a.accounted_receipts, 1);
        }
    }
    #[test]
    fn synthetic_rejects_identity_and_policy_attacks() {
        let (m, e, s) = fixture();
        assert!(reconcile(&m, &[e[0].clone(), e[0].clone()], &s).is_err());
        assert!(reconcile(&m, &e, &[s[0].clone(), s[0].clone()]).is_err());
        assert!(reconcile(&m, &[], &s).is_err());
        assert!(reconcile(&m, &e, &[]).is_err());
        for field in 0..5 {
            let mut ee = e.clone();
            match field {
                0 => ee[0].run_id = "foreign".into(),
                1 => ee[0].invocation_id = "foreign".into(),
                2 => ee[0].parent_invocation_id = "foreign".into(),
                3 => ee[0].parent_task_id = "foreign".into(),
                _ => ee[0].capture_id = "foreign".into(),
            };
            assert!(reconcile(&m, &ee, &s).is_err());
        }
        let mut mm = m.clone();
        mm.owners[0].tickets[0].policy = TicketPolicy::Opaque;
        assert!(reconcile(&mm, &e, &s).is_err());
        mm = m.clone();
        mm.owners[0].tickets[0].lifecycle = TicketLifecycle::Opaque;
        assert!(reconcile(&mm, &e, &s).is_err());
        mm = m.clone();
        mm.owners[0].tickets[0].lifecycle = TicketLifecycle::NotStarted;
        assert!(reconcile(&mm, &e, &s).is_err());
    }
    #[test]
    fn synthetic_unsealed_interrupted_and_failed_owners() {
        let (mut m, e, s) = fixture();
        m.owners[0].sealed = false;
        assert!(!reconcile(&m, &e, &s).unwrap().scheduler_entry_complete);
        m.owners[0].sealed = true;
        m.owners[0].interrupted = true;
        m.owners[0].outcome = OwnerOutcome::Interrupted;
        assert!(!reconcile(&m, &e, &s).unwrap().scheduler_entry_complete);
        m.owners[0].interrupted = false;
        m.owners[0].outcome = OwnerOutcome::Failed;
        m.owners[0].tickets[0].lifecycle = TicketLifecycle::Failed;
        assert!(reconcile(&m, &e, &s).unwrap().scheduler_entry_complete);
        m.owners[0].lost_events = 1;
        assert!(!reconcile(&m, &e, &s).unwrap().scheduler_entry_complete);
    }
    #[test]
    fn synthetic_authority_duplicates_cycles_and_budgets() {
        let (m, e, s) = fixture();
        let mut mm = m.clone();
        mm.owners.push(mm.owners[0].clone());
        assert!(reconcile(&mm, &e, &s).is_err());
        mm = m.clone();
        mm.authority_root = "absent".into();
        assert!(reconcile(&mm, &e, &s).is_err());
        mm = m.clone();
        mm.owners[0].parent_invocation_id = Some("o".into());
        assert!(reconcile(&mm, &e, &s).is_err());
        mm = m.clone();
        for (name, parent) in [("a", "b"), ("b", "a")] {
            let mut owner = m.owners[0].clone();
            owner.invocation_id = name.into();
            owner.parent_invocation_id = Some(parent.into());
            owner.parent_task_id = Some("nested".into());
            mm.owners.push(owner);
        }
        assert!(reconcile(&mm, &e, &s).is_err());
        mm = m.clone();
        mm.owners = vec![m.owners[0].clone(); MAX_ENTRIES + 1];
        assert!(reconcile(&mm, &e, &s).is_err());
        assert!(reconcile(&m, &vec![e[0].clone(); MAX_ENTRIES + 1], &s).is_err());
        assert!(reconcile(&m, &e, &vec![s[0].clone(); MAX_ENTRIES + 1]).is_err());
        mm = m.clone();
        mm.owners[0].tickets.clear();
        assert!(reconcile(&mm, &e, &s).is_err());
        mm = m.clone();
        mm.owners[0].tickets[0].invocation_id = "o".into();
        assert!(reconcile(&mm, &e, &s).is_err());
    }
    #[test]
    fn synthetic_invalid_receipt_and_snapshot_lifecycle() {
        let (m, e, mut s) = fixture();
        s[0].structural_complete = false;
        let a = reconcile(&m, &e, &s).unwrap();
        assert!(!a.scheduler_entry_complete);
        assert_eq!(a.accounted_receipts, 0);
        s[0].full_pipeline_complete = true;
        assert!(reconcile(&m, &e, &s).is_err());
        s[0].full_pipeline_complete = false;
        s[0].schema = 2;
        assert!(reconcile(&m, &e, &s).is_err());
        for lifecycle in [
            TicketLifecycle::Declared,
            TicketLifecycle::NotStarted,
            TicketLifecycle::ExecuteEntered,
        ] {
            let mut mm = m.clone();
            mm.owners[0].tickets[0].lifecycle = lifecycle;
            mm.owners[0].tickets[0].execute_entered = lifecycle == TicketLifecycle::ExecuteEntered;
            assert!(reconcile(&mm, &[], &[]).is_err());
            mm.owners[0].sealed = false;
            let a = reconcile(&mm, &[], &[]).unwrap();
            assert!(!a.scheduler_entry_complete);
            assert!(
                a.issues
                    .iter()
                    .any(|issue| issue.contains("missing expected receipt"))
            );
        }
        assert!(parse_manifest(br#"{"schema":1,"schema":1}"#).is_err());
        let mut value = serde_json::to_value(&m).unwrap();
        value["owners"][0]["extra"] = true.into();
        assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
        value["owners"][0].as_object_mut().unwrap().remove("extra");
        value["owners"][0]["tickets"][0]["lifecycle"] = "invented".into();
        assert!(parse_manifest(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    #[test]
    fn synthetic_preparation_failure_retains_required_receipt() {
        let (mut m, e, mut s) = fixture();
        m.owners[0].outcome = OwnerOutcome::Failed;
        m.owners[0].tickets[0].lifecycle = TicketLifecycle::NotStarted;
        m.owners[0].tickets[0].execute_entered = false;
        assert!(reconcile(&m, &e, &s).is_err());
        s[0].structural_complete = false;
        assert!(!reconcile(&m, &e, &s).unwrap().scheduler_entry_complete);
        let a = reconcile(&m, &[], &[]).unwrap();
        assert!(!a.scheduler_entry_complete);
        assert!(
            a.issues
                .iter()
                .any(|issue| issue.contains("missing expected receipt"))
        );
        m.owners[0].tickets[0].execute_entered = true;
        assert!(reconcile(&m, &e, &s).is_err());
        m.owners[0].tickets[0].execute_entered = false;
        m.owners[0].tickets[0].lifecycle = TicketLifecycle::Succeeded;
        assert!(reconcile(&m, &e, &s).is_err());
    }
    #[test]
    fn synthetic_pending_owner_requires_final_outcome() {
        let (mut m, e, s) = fixture();
        m.owners[0].outcome = OwnerOutcome::Pending;
        assert!(reconcile(&m, &e, &s).is_err());
        m.owners[0].sealed = false;
        assert!(!reconcile(&m, &e, &s).unwrap().scheduler_entry_complete);
        m.owners[0].interrupted = true;
        assert!(reconcile(&m, &e, &s).is_err());
    }
    #[test]
    fn synthetic_bounds_versions_and_strict_json() {
        let (mut m, e, s) = fixture();
        m.schema = 2;
        assert!(reconcile(&m, &e, &s).is_err());
        m.schema = 1;
        m.profile = "future".into();
        assert!(reconcile(&m, &e, &s).is_err());
        assert!(parse_manifest(&vec![b' '; MAX_BYTES + 1]).is_err());
        assert!(parse_envelope(br#"{"schema":1,"extra":true}"#).is_err());
        assert!(id(&"a".repeat(MAX_ID_BYTES + 1)).is_err());
        assert!(id("../escape").is_err());
    }
}
