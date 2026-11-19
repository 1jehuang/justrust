//! Reciprocal three-owner endorsement validation. No inferred numeric or filename pairing.
use crate::execution_graph_resource::{self as resource, ResourceAssessment, safe_id};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Attachment {
    attachment_capture_id: String,
    attachment_id: u64,
    controller_id: String,
    resource_capture_id: String,
    query_capture_id: String,
    query_root_namespace: u64,
    pool_capture_id: String,
    pool_observer: u64,
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
enum Role {
    Query,
    Pool,
    Resource,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Endorsement {
    schema: u32,
    producer: String,
    scope: String,
    capture_id: String,
    role: Role,
    sealed: bool,
    expected_endorsements: u64,
    endorsement_allocator_baseline: u64,
    endorsement_allocator_high_water: u64,
    lost_events: u64,
    loss_overflowed: bool,
    attachment: Attachment,
}
#[derive(Debug, Clone, Serialize)]
pub struct EndorsementAssessment {
    pub schema: u32,
    pub producer: String,
    pub capture_id: String,
    pub valid: bool,
    pub role: String,
    pub attachment_capture_id: String,
    pub full_pipeline_complete: bool,
    pub resource_full_complete: bool,
    pub cpu_work_ns: Option<u64>,
    pub execution_span_ns: Option<u64>,
}
#[derive(Debug, Clone, Serialize)]
pub struct DocumentDiagnostic {
    pub document_index: usize,
    pub producer: String,
    pub capture_id: Option<String>,
    pub valid: bool,
    pub error: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct CrossWaitEdge {
    pub resource_capture_id: String,
    pub acquire_return_id: u64,
    pub query_capture_id: String,
    pub wait_id: u64,
    pub kind: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct AttachmentLinkAssessment {
    pub attachment_capture_id: String,
    pub state: String,
    pub context_links_valid: bool,
    pub context_links_complete: bool,
    pub cross_wait_edges: Vec<CrossWaitEdge>,
    pub issues: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct AttachmentAssessment {
    pub state: String,
    pub documents: Vec<DocumentDiagnostic>,
    pub resources: Vec<ResourceAssessment>,
    pub attachments: Vec<AttachmentLinkAssessment>,
    pub issues: Vec<String>,
    pub full_pipeline_complete: bool,
    pub resource_full_complete: bool,
    pub cpu_work_ns: Option<u64>,
    pub execution_span_ns: Option<u64>,
}
// Only reciprocal validation can construct this wrapper. Deserialization cannot.
struct EndorsementTriple {
    attachment: Attachment,
}
fn parse(bytes: &[u8]) -> Result<Endorsement> {
    if bytes.len() > resource::MAX_BYTES {
        bail!("endorsement exceeds 32 MiB");
    }
    let e: Endorsement = serde_json::from_slice(bytes).context("invalid attachment endorsement")?;
    let a = &e.attachment;
    if e.schema != 1
        || e.producer != "rustc-frontend-admission-attachment"
        || e.scope != "selected-resource-query-pool-attachment"
        || !e.sealed
        || e.expected_endorsements != 1
        || e.endorsement_allocator_baseline != 0
        || e.endorsement_allocator_high_water != 1
        || e.lost_events != 0
        || e.loss_overflowed
    {
        bail!("invalid endorsement authority or inventory");
    }
    if ![
        &e.capture_id,
        &a.attachment_capture_id,
        &a.controller_id,
        &a.resource_capture_id,
        &a.query_capture_id,
        &a.pool_capture_id,
    ]
    .iter()
    .all(|s| safe_id(s))
        || a.attachment_id == 0
        || a.query_root_namespace == 0
        || a.pool_observer == 0
    {
        bail!("invalid endorsement namespace");
    }
    if e.capture_id == a.attachment_capture_id
        || e.capture_id == a.controller_id
        || e.capture_id == a.resource_capture_id
        || e.capture_id == a.query_capture_id
        || e.capture_id == a.pool_capture_id
    {
        bail!("endorsement capture ID aliases participant authority");
    }
    Ok(e)
}
pub fn assess_bytes(bytes: &[u8]) -> Result<EndorsementAssessment> {
    let e = parse(bytes)?;
    Ok(EndorsementAssessment {
        schema: 1,
        producer: e.producer,
        capture_id: e.capture_id,
        valid: true,
        role: match e.role {
            Role::Query => "query",
            Role::Pool => "pool",
            Role::Resource => "resource",
        }
        .into(),
        attachment_capture_id: e.attachment.attachment_capture_id,
        full_pipeline_complete: false,
        resource_full_complete: false,
        cpu_work_ns: None,
        execution_span_ns: None,
    })
}
#[derive(Deserialize)]
struct Header {
    producer: String,
    capture_id: Option<String>,
}
#[derive(Deserialize)]
struct QueryProjection {
    task_bindings: BindingProjection,
}
#[derive(Deserialize)]
struct BindingProjection {
    pool: Option<PoolProjection>,
}
#[derive(Deserialize)]
struct PoolProjection {
    capture_id: String,
    observer: u64,
}

pub fn collect_documents(
    documents: &[Vec<u8>],
    selected_queries: Option<&BTreeSet<String>>,
) -> Result<AttachmentAssessment> {
    collect_documents_with_scan_budget(documents, selected_queries, 64 * 1024 * 1024)
}

/// Increase only the bytes scanned for candidate identities. Authority parsers
/// below retain their own strict byte/event limits, including query receipts.
pub(crate) fn collect_documents_with_scan_budget(
    documents: &[Vec<u8>],
    selected_queries: Option<&BTreeSet<String>>,
    max_scan_bytes: usize,
) -> Result<AttachmentAssessment> {
    if !(1..=512 * 1024 * 1024).contains(&max_scan_bytes) {
        bail!("resource scan policy outside hard byte bounds");
    }
    if documents.len() > 20_000
        || documents
            .iter()
            .try_fold(0usize, |n, d| n.checked_add(d.len()))
            .is_none_or(|n| n > max_scan_bytes)
    {
        bail!("resource aggregate budget exceeded");
    }
    let mut capture_counts: BTreeMap<String, usize> = BTreeMap::new();
    let headers: Vec<_> = documents
        .iter()
        .map(|d| serde_json::from_slice::<Header>(d).ok())
        .collect();
    for h in headers.iter().flatten() {
        if let Some(id) = &h.capture_id {
            *capture_counts.entry(id.clone()).or_default() += 1;
        }
    }
    let mut result = AttachmentAssessment {
        state: "unknown".into(),
        documents: Vec::new(),
        resources: Vec::new(),
        attachments: Vec::new(),
        issues: Vec::new(),
        full_pipeline_complete: false,
        resource_full_complete: false,
        cpu_work_ns: None,
        execution_span_ns: None,
    };
    let mut endorsements = Vec::new();
    let mut resources = Vec::new();
    let mut queries = BTreeMap::new();
    let mut pools = BTreeMap::new();
    for (index, (bytes, header)) in documents.iter().zip(&headers).enumerate() {
        let Some(header) = header else {
            continue;
        };
        let id = header.capture_id.clone();
        let unique = id
            .as_ref()
            .is_some_and(|id| capture_counts.get(id) == Some(&1));
        match header.producer.as_str() {
            "rustc-frontend-allowance" => match resource::collect_bytes(bytes) {
                Ok(mut prefix) => {
                    if !unique {
                        prefix.assessment.prefix_protocol_valid = false;
                        prefix.assessment.prefix_records_complete = false;
                        prefix.assessment.local_resource_edges.clear();
                        prefix.returns.clear();
                        prefix
                            .assessment
                            .issues
                            .push("ambiguous duplicate capture authority".into());
                    }
                    let valid = unique && prefix.assessment.prefix_protocol_valid;
                    result.documents.push(DocumentDiagnostic {
                        document_index: index,
                        producer: header.producer.clone(),
                        capture_id: id,
                        valid,
                        error: (!unique).then(|| "ambiguous capture authority".into()),
                    });
                    resources.push((index, unique, prefix));
                }
                Err(error) => result.documents.push(DocumentDiagnostic {
                    document_index: index,
                    producer: header.producer.clone(),
                    capture_id: id,
                    valid: false,
                    error: Some(format!("{error:#}")),
                }),
            },
            "rustc-frontend-admission-attachment" => match parse(bytes) {
                Ok(e) => {
                    result.documents.push(DocumentDiagnostic {
                        document_index: index,
                        producer: header.producer.clone(),
                        capture_id: id,
                        valid: unique,
                        error: (!unique).then(|| "ambiguous capture authority".into()),
                    });
                    endorsements.push((index, unique, e));
                }
                Err(error) => result.documents.push(DocumentDiagnostic {
                    document_index: index,
                    producer: header.producer.clone(),
                    capture_id: id,
                    valid: false,
                    error: Some(format!("{error:#}")),
                }),
            },
            "rustc-query-admission" if unique => {
                if let Ok(q) = crate::execution_graph_query::collect_bytes(bytes)
                    && let Some(wait) = q.wait_inventory
                    && let Ok(projection) = serde_json::from_slice::<QueryProjection>(bytes)
                {
                    queries.insert(
                        q.assessment.capture_id,
                        (wait, projection.task_bindings.pool),
                    );
                }
            }
            "rustc-thread-pool-join" if unique => {
                if let Ok(p) = crate::execution_graph_continuation::collect_bytes(bytes)
                    && let Some(inventory) = p.inventory
                {
                    pools.insert(inventory.capture_id.clone(), inventory);
                }
            }
            _ => {}
        }
    }
    let selected_anchors: BTreeSet<_> = endorsements
        .iter()
        .filter(|e| {
            selected_queries.is_none_or(|set| set.contains(&e.2.attachment.query_capture_id))
        })
        .map(|e| e.2.attachment.attachment_capture_id.clone())
        .collect();
    let mut groups: BTreeMap<String, Vec<&(usize, bool, Endorsement)>> = BTreeMap::new();
    for e in &endorsements {
        if selected_anchors.contains(&e.2.attachment.attachment_capture_id) {
            groups
                .entry(e.2.attachment.attachment_capture_id.clone())
                .or_default()
                .push(e);
        }
    }
    let mut selected_doc_indices = BTreeSet::new();
    let mut selected_resource_indices = BTreeSet::new();
    for (anchor, group) in &groups {
        let tuple = &group[0].2.attachment;
        let roles: BTreeSet<_> = group.iter().map(|e| e.2.role).collect();
        let mut issues = Vec::new();
        let mut ambiguous = false;
        if group.len() != 3 || roles.len() != 3 {
            issues.push("missing or duplicate endorsement role".into());
            ambiguous = group.len() > 3 || roles.len() < group.len();
        }
        if group.iter().any(|e| !e.1 || e.2.attachment != *tuple) {
            issues.push("conflicting or ambiguous endorsement tuple".into());
            ambiguous = true;
        }
        for e in group {
            selected_doc_indices.insert(e.0);
        }
        let prefix = resources
            .iter()
            .find(|(_, _, p)| p.receipt.capture_id == tuple.resource_capture_id);
        let query = queries.get(&tuple.query_capture_id);
        let pool = pools.get(&tuple.pool_capture_id);
        if let Some((index, unique, p)) = prefix {
            selected_doc_indices.insert(*index);
            selected_resource_indices.insert(*index);
            if !unique
                || p.receipt.controller_id != tuple.controller_id
                || p.receipt.attachment_capture_id.as_ref() != Some(anchor)
                || p.receipt.expected_attachments != Some(1)
            {
                issues.push("resource anchor/controller mismatch".into());
            }
        } else {
            issues.push("missing resource prefix".into());
        }
        match (query, pool) {
            (Some((_, Some(reference))), Some(pool))
                if reference.capture_id == tuple.pool_capture_id
                    && reference.observer == tuple.pool_observer
                    && pool.observer == tuple.pool_observer
                    && pool.query_capture_id.as_ref() == Some(&tuple.query_capture_id) => {}
            _ => issues.push("missing reciprocal query4/pool2 authority".into()),
        }
        let triple = issues.is_empty().then(|| EndorsementTriple {
            attachment: tuple.clone(),
        });
        let attachment_valid = triple.is_some();
        let mut cross = Vec::new();
        let mut links_valid = triple.is_some();
        let mut links_complete = false;
        if let (Some(triple), Some((_, _, prefix)), Some((wait, _))) = (triple, prefix, query) {
            links_complete =
                prefix.assessment.prefix_records_complete && prefix.context_coverage_complete;
            if prefix.contexts.iter().any(|c| {
                c.attachment_id != triple.attachment.attachment_id
                    || c.query_root_namespace != triple.attachment.query_root_namespace
            }) {
                links_valid = false;
                links_complete = false;
                issues.push("qualified context namespace differs from endorsed tuple".into());
            }
            for returned in &prefix.returns {
                let c = &returned.context;
                let a = &triple.attachment;
                if c.attachment_id == a.attachment_id
                    && c.query_root_namespace == a.query_root_namespace
                    && c.phase == resource::Phase::QueryWaitReacquire
                    && wait.contains_resumed(&a.query_capture_id, c.wait_id)
                {
                    cross.push(CrossWaitEdge {
                        resource_capture_id: a.resource_capture_id.clone(),
                        acquire_return_id: returned.return_id,
                        query_capture_id: a.query_capture_id.clone(),
                        wait_id: c.wait_id,
                        kind: "acquire_returned_to_wait_resumed".into(),
                    });
                } else {
                    links_valid = false;
                    links_complete = false;
                    issues.push(
                        "returned context lacks exact qualified validated WaitResumed".into(),
                    );
                }
            }
            if !prefix.assessment.prefix_protocol_valid {
                links_valid = false;
                links_complete = false;
            }
        }
        let state = if ambiguous {
            "ambiguous"
        } else if attachment_valid {
            "validated"
        } else {
            "incomplete"
        };
        result.attachments.push(AttachmentLinkAssessment {
            attachment_capture_id: anchor.clone(),
            state: state.into(),
            context_links_valid: links_valid,
            context_links_complete: links_complete,
            cross_wait_edges: cross,
            issues,
        });
    }
    for (index, _, prefix) in &resources {
        let explicitly_selected = prefix
            .receipt
            .attachment_capture_id
            .as_ref()
            .is_some_and(|id| groups.contains_key(id));
        if selected_queries.is_none() || explicitly_selected {
            selected_resource_indices.insert(*index);
            selected_doc_indices.insert(*index);
            if prefix.receipt.expected_attachments == Some(1) && !explicitly_selected {
                result.attachments.push(AttachmentLinkAssessment {
                    attachment_capture_id: prefix
                        .receipt
                        .attachment_capture_id
                        .clone()
                        .unwrap_or_default(),
                    state: "incomplete".into(),
                    context_links_valid: false,
                    context_links_complete: false,
                    cross_wait_edges: Vec::new(),
                    issues: vec![
                        "selected resource expected triple endorsements but none were observed"
                            .into(),
                    ],
                });
            }
        }
    }
    for (index, _, prefix) in resources {
        if selected_queries.is_none() || selected_resource_indices.contains(&index) {
            result.resources.push(prefix.assessment);
        }
    }
    if selected_queries.is_some() {
        result
            .documents
            .retain(|d| selected_doc_indices.contains(&d.document_index));
    }
    result.state = if result.attachments.iter().any(|a| a.state == "ambiguous") {
        "ambiguous"
    } else if result.attachments.iter().any(|a| a.state != "validated")
        || result.documents.iter().any(|d| !d.valid)
    {
        "incomplete"
    } else if !result.attachments.is_empty() {
        "validated"
    } else {
        "unknown"
    }
    .into();
    Ok(result)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{Value, json};
    pub(crate) fn endorsement(role: &str) -> Value {
        json!({"schema":1,"producer":"rustc-frontend-admission-attachment","scope":"selected-resource-query-pool-attachment","capture_id":format!("endorsement-{role}"),"role":role,"sealed":true,"expected_endorsements":1,"endorsement_allocator_baseline":0,"endorsement_allocator_high_water":1,"lost_events":0,"loss_overflowed":false,"attachment":{"attachment_capture_id":"attachment-1","attachment_id":1,"controller_id":"controller-1","resource_capture_id":"resource-1","query_capture_id":"query_123-safe","query_root_namespace":1,"pool_capture_id":"pool-1","pool_observer":1}})
    }
    pub(crate) fn documents() -> Vec<Value> {
        let mut query = crate::execution_graph_query::tests::task_binding_fixture();
        query["capture_id"] = json!("query_123-safe");
        let pool = json!({"schema":2,"producer":"rustc-thread-pool-join","scope":"native-join-continuations","capture_id":"pool-1","query_capture_id":"query_123-safe","observer":1,"full_pipeline_complete":false,"cpu_work_ns":null,"execution_span_ns":null,"scope_coverage":false,"scheduler_coverage":false,"pre_attach_coverage":false,"sealed":true,"interface_outcome":"returned","task_allocator_baseline":1,"task_allocator_high_water":2,"expected_tasks":1,"dropped_events":0,"events":[{"kind":"root","task":1,"at_ns":0},{"kind":"start","task":1,"at_ns":1},{"kind":"finish","task":1,"at_ns":20,"outcome":"normal"}]});
        vec![
            query,
            pool,
            resource::tests::fixture(),
            endorsement("query"),
            endorsement("pool"),
            endorsement("resource"),
        ]
    }
    fn collect(v: &[Value]) -> AttachmentAssessment {
        collect_documents(
            &v.iter()
                .map(|d| serde_json::to_vec(d).unwrap())
                .collect::<Vec<_>>(),
            None,
        )
        .unwrap()
    }
    #[test]
    fn large_capture_scan_budget_is_explicit_and_hard_bounded() {
        let docs = documents()
            .iter()
            .map(|d| serde_json::to_vec(d).unwrap())
            .collect::<Vec<_>>();
        let total = docs.iter().map(Vec::len).sum::<usize>();
        assert_eq!(
            collect_documents_with_scan_budget(&docs, None, total)
                .unwrap()
                .state,
            "validated"
        );
        assert!(collect_documents_with_scan_budget(&docs, None, total - 1).is_err());
        assert!(collect_documents_with_scan_budget(&[], None, 0).is_err());
        assert!(collect_documents_with_scan_budget(&docs, None, 512 * 1024 * 1024 + 1).is_err());
        // File-count bounds must not be relaxed with the aggregate scan policy.
        assert!(
            collect_documents_with_scan_budget(&vec![Vec::new(); 20_001], None, 512 * 1024 * 1024)
                .is_err()
        );
    }

    #[test]
    fn large_capture_scan_never_promotes_oversized_query_authority() {
        let mut docs = documents()
            .iter()
            .map(|d| serde_json::to_vec(d).unwrap())
            .collect::<Vec<_>>();
        // A syntactically valid schema4 query with padding exceeds the strict
        // authority budget, while remaining within the explicit scan budget.
        docs[0].resize(64 * 1024 * 1024 + 1, b' ');
        assert!(collect_documents(&docs, None).is_err());
        assert!(crate::execution_graph_query::collect_bytes(&docs[0]).is_err());
        let assessment =
            collect_documents_with_scan_budget(&docs, None, 512 * 1024 * 1024).unwrap();
        assert_ne!(assessment.state, "validated");
        assert_eq!(assessment.attachments.len(), 1);
        let attachment = &assessment.attachments[0];
        assert!(!attachment.context_links_valid && !attachment.context_links_complete);
        assert!(attachment.cross_wait_edges.is_empty());
        assert!(
            attachment
                .issues
                .iter()
                .any(|s| s == "missing reciprocal query4/pool2 authority")
        );
        // Individual resource parser budget remains independent as well.
        docs[2].resize(resource::MAX_BYTES + 1, b' ');
        let assessment =
            collect_documents_with_scan_budget(&docs, None, 512 * 1024 * 1024).unwrap();
        assert!(
            assessment
                .documents
                .iter()
                .any(|d| d.producer == "rustc-frontend-allowance" && !d.valid)
        );
    }

    #[test]
    fn exact_three_roles_and_native_return_chain() {
        let a = collect(&documents());
        assert_eq!(a.state, "validated", "{a:?}");
        assert_eq!(a.attachments[0].cross_wait_edges.len(), 1);
        assert!(a.attachments[0].context_links_valid);
    }
    #[test]
    fn each_role_missing_and_all_missing_are_not_zero_complete() {
        for i in 3..6 {
            let mut v = documents();
            v.remove(i);
            let a = collect(&v);
            assert_ne!(a.state, "validated");
            assert!(a.attachments.iter().all(|a| a.cross_wait_edges.is_empty()));
        }
        let mut v = documents();
        v.truncate(3);
        assert_eq!(collect(&v).state, "incomplete");
        v.truncate(2);
        assert_eq!(collect(&v).state, "unknown");
    }
    #[test]
    fn all_eight_tuple_fields_fail_closed() {
        for field in [
            "attachment_capture_id",
            "attachment_id",
            "controller_id",
            "resource_capture_id",
            "query_capture_id",
            "query_root_namespace",
            "pool_capture_id",
            "pool_observer",
        ] {
            let mut v = documents();
            let old = &v[4]["attachment"][field];
            v[4]["attachment"][field] = if old.is_string() {
                json!("foreign-copy")
            } else {
                json!(2)
            };
            let a = collect(&v);
            assert_ne!(a.state, "validated", "{field}");
            assert!(a.attachments.iter().all(|a| a.cross_wait_edges.is_empty()));
        }
    }
    #[test]
    fn duplicate_capture_and_unilateral_roles_poison() {
        let mut v = documents();
        v.push(v[3].clone());
        assert_eq!(collect(&v).state, "ambiguous");
        let mut v = documents();
        for item in &mut v[3..] {
            item["role"] = json!("resource");
        }
        assert_ne!(collect(&v).state, "validated");
    }
    #[test]
    fn malformed_and_loss_do_not_erase_independent_receipts() {
        let mut v = documents();
        v[2]["unknown"] = json!(true);
        let a = collect(&v);
        assert_eq!(a.state, "incomplete");
        assert_eq!(
            crate::execution_graph_query::assess_bytes(&serde_json::to_vec(&v[0]).unwrap())
                .unwrap()
                .wait_structural_complete,
            Some(true)
        );
        let mut v = documents();
        v[2]["loss_overflowed"] = json!(true);
        v[2]["records_complete"] = json!(false);
        assert!(collect(&v).attachments[0].cross_wait_edges.is_empty());
    }
    #[test]
    fn wait_endpoint_and_optional_task_membership_are_separate() {
        let mut v = documents();
        v[0]["wait_events"].as_array_mut().unwrap().pop();
        assert!(collect(&v).attachments[0].cross_wait_edges.is_empty());
        let mut v = documents();
        v[0]["task_bindings"]["events"][6]["target"]["at_pool_ns"] = json!(u64::MAX);
        assert_eq!(collect(&v).attachments[0].cross_wait_edges.len(), 1);
    }
    #[test]
    fn original_bytes_duplicate_and_required_fields() {
        let raw = serde_json::to_string(&endorsement("query")).unwrap();
        assert!(
            assess_bytes(
                raw.replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1)
                    .as_bytes()
            )
            .is_err()
        );
        for name in ["sealed", "lost_events", "attachment"] {
            let mut v = endorsement("query");
            v.as_object_mut().unwrap().remove(name);
            assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
        }
        let mut v = endorsement("query");
        v["attachment"]["controller_id"] = json!(1);
        assert!(assess_bytes(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    #[test]
    fn selected_anchor_keeps_foreign_query_conflict() {
        let mut v = documents();
        let mut extra = endorsement("query");
        extra["capture_id"] = json!("foreign-endorsement");
        extra["attachment"]["query_capture_id"] = json!("foreign-query");
        v.push(extra);
        let docs = v
            .iter()
            .map(|d| serde_json::to_vec(d).unwrap())
            .collect::<Vec<_>>();
        let selected = BTreeSet::from(["query_123-safe".to_string()]);
        let a = collect_documents(&docs, Some(&selected)).unwrap();
        assert_eq!(a.state, "ambiguous");
        assert!(a.attachments.iter().all(|a| a.cross_wait_edges.is_empty()));
    }
    #[test]
    fn selected_queries_filter_foreign_prefixes_without_merging() {
        let v = documents();
        let docs = v
            .iter()
            .map(|d| serde_json::to_vec(d).unwrap())
            .collect::<Vec<_>>();
        let selected = BTreeSet::from(["other-query".to_string()]);
        let a = collect_documents(&docs, Some(&selected)).unwrap();
        assert_eq!(a.state, "unknown");
        assert!(a.resources.is_empty() && a.documents.is_empty());
    }
}
