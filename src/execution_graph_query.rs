//! Independent validation of admitted query jobs, not query requests or a dependency graph.
//! Only synthetic fixtures exercise this module until a rebuilt native compiler is accepted.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_CONTEXTS: usize = 1024;
const MAX_EVENTS: usize = 100_000;
const MAX_JOBS: u64 = 100_000;
const REQUIRED_BOUNDARIES: &[&str] = &[
    "fast-cache-hits",
    "race-cache-hits",
    "green-ensure-no-job",
    "feeds",
    "direct-paths",
    "nonquery",
    "resource-internals",
    "driver-and-process-outcome",
    "cargo-parent-binding",
    "cache-read-dependencies",
    "wait-and-resource-dependencies",
];

// deserialize_with makes nullable fields mandatory rather than silently defaulting missing keys.
fn nullable<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<u64>, D::Error> {
    Option::<u64>::deserialize(d)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u32,
    producer: String,
    scope: String,
    capture_id: String,
    full_pipeline_complete: bool,
    #[serde(deserialize_with = "nullable")]
    cpu_work_ns: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    execution_span_ns: Option<u64>,
    interface_outcome: InterfaceOutcome,
    capture_complete: bool,
    lost_events: u64,
    opaque_boundaries: Vec<String>,
    expected_contexts: u64,
    contexts: Vec<QueryContext>,
    events: Vec<Event>,
}

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum InterfaceOutcome {
    Returned,
    Interrupted,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryContext {
    context_id: u64,
    allocator_baseline: u64,
    #[serde(deserialize_with = "nullable")]
    allocator_high_water: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    expected_jobs: Option<u64>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum Event {
    Admitted {
        context_id: u64,
        job_id: u64,
        #[serde(deserialize_with = "nullable")]
        parent_job_id: Option<u64>,
        at_ns: u64,
    },
    Completed {
        context_id: u64,
        job_id: u64,
        at_ns: u64,
    },
    Poisoned {
        context_id: u64,
        job_id: u64,
        at_ns: u64,
    },
}

#[derive(Debug, Serialize)]
pub struct QueryAssessment {
    pub schema: u32,
    pub producer: String,
    pub scope: String,
    pub capture_id: String,
    pub structural_complete: bool,
    pub full_pipeline_complete: bool,
    /// Unique admitted jobs, namespaced by context.
    pub nodes: usize,
    /// Descriptive parent links only, not waits or dependency edges.
    pub edges: usize,
    pub issues: Vec<String>,
    pub opaque_boundaries: Vec<String>,
    pub cpu_work_ns: Option<u64>,
    pub execution_span_ns: Option<u64>,
}

/// Parse the original bytes directly so duplicate fields cannot disappear through a Value.
/// Malformed/unsupported receipts fail parsing. Lifecycle discrepancies return issues.
/// Capture identity uniqueness across receipts must be enforced by the caller.
pub fn assess_bytes(bytes: &[u8]) -> Result<QueryAssessment> {
    if bytes.len() > MAX_BYTES {
        bail!("query receipt exceeds 64 MiB byte budget");
    }
    let r: Receipt = serde_json::from_slice(bytes).context("invalid query admission receipt")?;
    if r.schema != 1
        || r.producer != "rustc-query-admission"
        || r.scope != "query-job-admission-lifecycle"
    {
        bail!("unsupported query receipt schema, producer or scope");
    }
    if r.capture_id.is_empty() || r.capture_id.len() > 128 {
        bail!("query capture identity must contain 1..=128 bytes");
    }
    if r.expected_contexts > MAX_CONTEXTS as u64
        || r.contexts.len() > MAX_CONTEXTS
        || r.events.len() > MAX_EVENTS
        || r.opaque_boundaries.len() > 1024
        || r.opaque_boundaries
            .iter()
            .any(|s| s.is_empty() || s.len() > 128)
    {
        bail!("query receipt exceeds context, event or boundary budget");
    }
    let mut issues = BTreeSet::new();
    if r.full_pipeline_complete {
        issues.insert("query admissions cannot attest full pipeline completeness".into());
    }
    if r.cpu_work_ns.is_some() || r.execution_span_ns.is_some() {
        issues.insert("query admissions cannot attest CPU work or execution span".into());
    }
    for label in REQUIRED_BOUNDARIES {
        if !r.opaque_boundaries.iter().any(|s| s == label) {
            issues.insert(format!("missing required opaque boundary: {label}"));
        }
    }
    if !r.capture_complete {
        issues.insert("capture is incomplete".into());
    }
    if r.interface_outcome != InterfaceOutcome::Returned {
        issues.insert("compiler interface was interrupted".into());
    }
    if r.lost_events != 0 {
        issues.insert("capture lost events".into());
    }
    let mut contexts = BTreeMap::new();
    let mut total_expected = 0u64;
    for c in &r.contexts {
        if contexts.insert(c.context_id, c).is_some() {
            issues.insert(format!("duplicate context {}", c.context_id));
        }
        if c.allocator_baseline != 1 {
            issues.insert(format!("context {} baseline is not 1", c.context_id));
        }
        if c.allocator_high_water
            .is_some_and(|high| high > MAX_JOBS + 1)
            || c.expected_jobs.is_some_and(|expected| expected > MAX_JOBS)
        {
            bail!("allocator counts exceed job budget");
        }
        match (c.allocator_high_water, c.expected_jobs) {
            (Some(high), Some(expected)) => {
                if expected > MAX_JOBS || high > MAX_JOBS + 1 {
                    bail!("allocator counts exceed job budget");
                }
                total_expected = total_expected.saturating_add(expected);
                if high.checked_sub(c.allocator_baseline) != Some(expected) {
                    issues.insert(format!("context {} allocator count mismatch", c.context_id));
                }
            }
            _ => {
                issues.insert(format!(
                    "context {} missing allocator accounting",
                    c.context_id
                ));
            }
        }
    }
    if contexts.len() as u64 != r.expected_contexts {
        issues.insert("independent context allocation count mismatch".into());
    }
    if contexts.keys().copied().ne(1..=r.expected_contexts) {
        issues.insert("context IDs do not exactly cover independent allocator range".into());
    }
    if total_expected > MAX_JOBS {
        bail!("aggregate allocator count exceeds job budget");
    }
    if contexts.is_empty()
        && (!r.events.is_empty() || r.interface_outcome != InterfaceOutcome::Returned)
    {
        issues
            .insert("empty contexts require an explicitly returned interface and no events".into());
    }
    // Index and timestamp order are checked only within each job, never globally.
    let mut admitted = BTreeMap::new();
    let mut terminals = BTreeMap::new();
    for (index, event) in r.events.iter().enumerate() {
        let (context_id, job_id, at_ns) = match event {
            Event::Admitted {
                context_id,
                job_id,
                at_ns,
                ..
            }
            | Event::Completed {
                context_id,
                job_id,
                at_ns,
            }
            | Event::Poisoned {
                context_id,
                job_id,
                at_ns,
            } => (*context_id, *job_id, *at_ns),
        };
        let key = (context_id, job_id);
        match contexts.get(&context_id) {
            None => {
                issues.insert(format!("event references unknown context {context_id}"));
            }
            Some(c) => {
                if job_id < c.allocator_baseline
                    || !matches!(c.allocator_high_water, Some(high) if job_id < high)
                {
                    issues.insert(format!("job {key:?} outside allocator range"));
                }
            }
        }
        match event {
            Event::Admitted { parent_job_id, .. } => match admitted.entry(key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert((index, at_ns, *parent_job_id));
                }
                std::collections::btree_map::Entry::Occupied(_) => {
                    issues.insert(format!("duplicate admission {key:?}"));
                }
            },
            _ => match terminals.entry(key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert((index, at_ns));
                }
                std::collections::btree_map::Entry::Occupied(_) => {
                    issues.insert(format!("duplicate terminal {key:?}"));
                }
            },
        }
    }
    let mut edges = 0;
    for (&key, &(index, at_ns, parent)) in &admitted {
        match terminals.get(&key) {
            Some(&(terminal_index, terminal_ns))
                if terminal_index > index && terminal_ns >= at_ns => {}
            Some(_) => {
                issues.insert(format!(
                    "invalid per-job terminal order or timestamp {key:?}"
                ));
            }
            None => {
                issues.insert(format!("missing terminal {key:?}"));
            }
        }
        if let Some(parent) = parent {
            edges += 1;
            // Parent IDs are descriptive. Cross-thread event append order is not global order.
            if parent >= key.1
                || !matches!(admitted.get(&(key.0, parent)), Some(&(_, parent_ns, _)) if parent_ns <= at_ns)
            {
                issues.insert(format!("invalid same-context parent for {key:?}"));
            }
        }
    }
    for key in terminals.keys() {
        if !admitted.contains_key(key) {
            issues.insert(format!("terminal references unadmitted job {key:?}"));
        }
    }
    for c in contexts.values() {
        if let (Some(high), Some(expected)) = (c.allocator_high_water, c.expected_jobs) {
            let count = admitted
                .range((c.context_id, 0)..=(c.context_id, u64::MAX))
                .count() as u64;
            if count != expected {
                issues.insert(format!("context {} admission count mismatch", c.context_id));
            }
            // Range membership plus exact cardinality proves contiguous coverage independently.
            if (c.allocator_baseline..high).any(|job| !admitted.contains_key(&(c.context_id, job)))
            {
                issues.insert(format!(
                    "context {} has missing contiguous admissions",
                    c.context_id
                ));
            }
        }
    }
    Ok(QueryAssessment {
        schema: r.schema,
        producer: r.producer,
        scope: r.scope,
        capture_id: r.capture_id,
        structural_complete: r.capture_complete
            && r.interface_outcome == InterfaceOutcome::Returned
            && r.lost_events == 0
            && issues.is_empty(),
        full_pipeline_complete: false,
        nodes: admitted.len(),
        edges,
        issues: issues.into_iter().collect(),
        opaque_boundaries: r.opaque_boundaries,
        cpu_work_ns: None,
        execution_span_ns: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn fixture() -> Value {
        json!({"schema":1,"producer":"rustc-query-admission","scope":"query-job-admission-lifecycle",
            "capture_id":"synthetic-1","full_pipeline_complete":false,"cpu_work_ns":null,
            "execution_span_ns":null,"interface_outcome":"returned","capture_complete":true,
            "lost_events":0,"expected_contexts":1,"opaque_boundaries":REQUIRED_BOUNDARIES,
            "contexts":[{"context_id":1,"allocator_baseline":1,"allocator_high_water":3,"expected_jobs":2}],
            "events":[{"kind":"admitted","context_id":1,"job_id":1,"parent_job_id":null,"at_ns":1},
                {"kind":"admitted","context_id":1,"job_id":2,"parent_job_id":1,"at_ns":10},
                {"kind":"completed","context_id":1,"job_id":2,"at_ns":11},
                {"kind":"poisoned","context_id":1,"job_id":1,"at_ns":2}]})
    }
    fn assess(v: &Value) -> QueryAssessment {
        assess_bytes(&serde_json::to_vec(v).unwrap()).unwrap()
    }
    fn incomplete(v: &Value) {
        let a = assess(v);
        assert!(!a.structural_complete, "{a:?}");
        assert!(!a.issues.is_empty());
    }

    #[test]
    fn nonmonotonic_cross_thread_time_is_valid() {
        let a = assess(&fixture());
        assert!(a.structural_complete, "{a:?}");
        assert_eq!((a.nodes, a.edges), (2, 1));
        assert!(!a.full_pipeline_complete);
        assert_eq!((a.cpu_work_ns, a.execution_span_ns), (None, None));
    }
    #[test]
    fn missing_duplicate_and_unknown_lifecycles() {
        let mut v = fixture();
        v["events"].as_array_mut().unwrap().pop();
        incomplete(&v);
        let mut v = fixture();
        let duplicate = v["events"][0].clone();
        v["events"].as_array_mut().unwrap().push(duplicate);
        incomplete(&v);
        let mut v = fixture();
        let duplicate = v["events"][2].clone();
        v["events"].as_array_mut().unwrap().push(duplicate);
        incomplete(&v);
        let mut v = fixture();
        v["events"][3]["job_id"] = json!(3);
        incomplete(&v);
        let mut v = fixture();
        v["events"][0]["kind"] = json!("requested");
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    #[test]
    fn context_namespaces_allow_id_reuse_but_not_duplicate_contexts() {
        let mut v = fixture();
        v["expected_contexts"] = json!(2);
        let mut c = v["contexts"][0].clone();
        c["context_id"] = json!(2);
        v["contexts"].as_array_mut().unwrap().push(c);
        let extra: Vec<_> = v["events"]
            .as_array()
            .unwrap()
            .iter()
            .cloned()
            .map(|mut e| {
                e["context_id"] = json!(2);
                e
            })
            .collect();
        v["events"].as_array_mut().unwrap().extend(extra);
        assert!(assess(&v).structural_complete);
        v["contexts"][1]["context_id"] = json!(1);
        incomplete(&v);
        let mut v = fixture();
        v["events"][0]["context_id"] = json!(99);
        incomplete(&v);
    }
    #[test]
    fn allocator_and_parent_mismatches() {
        for (field, value) in [
            ("allocator_baseline", json!(0)),
            ("expected_jobs", json!(1)),
            ("allocator_high_water", json!(4)),
            ("allocator_high_water", Value::Null),
        ] {
            let mut v = fixture();
            v["contexts"][0][field] = value;
            incomplete(&v);
        }
        for parent in [0, 2, 9] {
            let mut v = fixture();
            v["events"][1]["parent_job_id"] = json!(parent);
            incomplete(&v);
        }
        let mut v = fixture();
        v["events"].as_array_mut().unwrap().swap(0, 1);
        assert!(assess(&v).structural_complete);
    }
    #[test]
    fn per_job_order_and_time_are_required() {
        let mut v = fixture();
        v["events"][2]["at_ns"] = json!(1);
        incomplete(&v);
        let mut v = fixture();
        v["events"].as_array_mut().unwrap().swap(1, 2);
        incomplete(&v);
    }
    #[test]
    fn incomplete_capture_and_false_claims_never_pass() {
        for (field, value) in [
            ("interface_outcome", json!("interrupted")),
            ("capture_complete", json!(false)),
            ("lost_events", json!(1)),
            ("full_pipeline_complete", json!(true)),
            ("cpu_work_ns", json!(1)),
            ("execution_span_ns", json!(1)),
        ] {
            let mut v = fixture();
            v[field] = value;
            incomplete(&v);
        }
    }
    #[test]
    fn opaque_privacy_scopes_are_required_individually() {
        for index in 0..REQUIRED_BOUNDARIES.len() {
            let mut v = fixture();
            v["opaque_boundaries"].as_array_mut().unwrap().remove(index);
            incomplete(&v);
        }
        let mut v = fixture();
        v["query_key"] = json!("private key");
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    #[test]
    fn typed_parse_rejects_duplicates_unknown_fields_and_missing_nulls() {
        let bytes = serde_json::to_string(&fixture()).unwrap();
        let duplicate = bytes.replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1);
        assert!(assess_bytes(duplicate.as_bytes()).is_err());
        for field in ["cpu_work_ns", "execution_span_ns"] {
            let mut v = fixture();
            v.as_object_mut().unwrap().remove(field);
            assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        }
        let mut v = fixture();
        v["events"][2]["parent_job_id"] = Value::Null;
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = fixture();
        v["events"][0]
            .as_object_mut()
            .unwrap()
            .remove("parent_job_id");
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        for field in ["schema", "producer", "scope"] {
            let mut v = fixture();
            v[field] = if field == "schema" {
                json!(2)
            } else {
                json!("unknown")
            };
            assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        }
    }
    #[test]
    fn empty_returned_capture_is_not_full_work() {
        let mut v = fixture();
        v["expected_contexts"] = json!(0);
        v["contexts"] = json!([]);
        v["events"] = json!([]);
        let a = assess(&v);
        assert!(a.structural_complete);
        assert_eq!(a.nodes, 0);
        assert!(!a.full_pipeline_complete);
        v["interface_outcome"] = json!("interrupted");
        incomplete(&v);
    }
    #[test]
    fn independent_inventory_detects_whole_context_deletion() {
        let mut v = fixture();
        v["contexts"] = json!([]);
        v["events"] = json!([]);
        incomplete(&v);

        // Delete a trailing context and every one of its events. Remaining jobs
        // are perfectly accounted for, but the independent root allocator is not.
        let mut v = fixture();
        v["expected_contexts"] = json!(2);
        incomplete(&v);

        // Delete the first context instead, leaving the same number of valid jobs.
        v["contexts"][0]["context_id"] = json!(2);
        for e in v["events"].as_array_mut().unwrap() {
            e["context_id"] = json!(2);
        }
        incomplete(&v);
    }
    #[test]
    fn context_ids_must_exactly_cover_independent_inventory() {
        for id in [0, 2, u64::MAX] {
            let mut v = fixture();
            v["contexts"][0]["context_id"] = json!(id);
            for e in v["events"].as_array_mut().unwrap() {
                e["context_id"] = json!(id);
            }
            incomplete(&v);
        }
        let mut v = fixture();
        v["expected_contexts"] = json!(0);
        incomplete(&v);
        let mut v = fixture();
        let duplicate = v["contexts"][0].clone();
        v["contexts"].as_array_mut().unwrap().push(duplicate);
        incomplete(&v);
    }
    #[test]
    fn independent_inventory_field_is_required_and_bounded() {
        let mut v = fixture();
        v.as_object_mut().unwrap().remove("expected_contexts");
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        for value in [Value::Null, json!("1"), json!(1025), json!(u64::MAX)] {
            let mut v = fixture();
            v["expected_contexts"] = value;
            assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        }
        let raw = serde_json::to_string(&fixture()).unwrap();
        let duplicate = raw.replacen(
            "\"expected_contexts\":1",
            "\"expected_contexts\":1,\"expected_contexts\":1",
            1,
        );
        assert!(assess_bytes(duplicate.as_bytes()).is_err());
    }
    #[test]
    fn parent_admission_time_cannot_follow_child() {
        let mut v = fixture();
        v["events"][0]["at_ns"] = json!(11);
        v["events"][3]["at_ns"] = json!(12);
        incomplete(&v);
        let mut v = fixture();
        v["events"][1]["at_ns"] = json!(1);
        assert!(assess(&v).structural_complete);
    }
    #[test]
    fn budgets_reject_unreasonable_input() {
        let mut v = fixture();
        v["capture_id"] = json!("x".repeat(129));
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = fixture();
        v["contexts"][0]["expected_jobs"] = json!(u64::MAX);
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = fixture();
        v["opaque_boundaries"][0] = json!("x".repeat(129));
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        assert!(assess_bytes(&vec![b' '; MAX_BYTES + 1]).is_err());
    }
}
