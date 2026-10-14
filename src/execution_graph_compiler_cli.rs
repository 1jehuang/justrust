//! File orchestration for one explicit compiler-launch authority.
//! Unknown/other authorities never count toward the selected authority.
use crate::execution_graph::read_bytes;
use crate::execution_graph_compiler as protocol;
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::Path;

const MAX_FILES: usize = 20_000;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Deserialize)]
struct Header {
    producer: Option<String>,
    profile: Option<String>,
    authority_root: Option<String>,
    capture_id: Option<String>,
}

pub fn command(directory: &Path, manifest_path: &Path, json: bool) -> Result<()> {
    let output = assess_directory(directory, manifest_path)?;
    if !json {
        println!("Selected native compiler-launch coverage, not full-pipeline coverage:");
    }
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn assess_directory(directory: &Path, manifest_path: &Path) -> Result<serde_json::Value> {
    ensure!(
        std::fs::symlink_metadata(directory)?.is_dir(),
        "compiler reconciliation requires a non-symlink capture directory"
    );
    let bytes = read_bytes(manifest_path, protocol::MAX_MANIFEST_BYTES as u64)?;
    let manifest = protocol::parse_manifest(&bytes)?;
    let prefix = format!("compiler-owner-{}", manifest.authority_root);
    let initial_name = format!("{prefix}.declaration.json");
    let initial_bytes = read_bytes(
        &directory.join(&initial_name),
        protocol::MAX_MANIFEST_BYTES as u64,
    )
    .context("missing independent compiler authority declaration")?;
    let initial = protocol::parse_manifest(&initial_bytes)?;
    let (scheduler, parent_bytes) = crate::execution_graph::compiler_scheduler_summary(
        directory,
        &manifest.parent_authority_root,
        &manifest.scheduler_capture_id,
    )?;
    let mut total = (bytes.len() + initial_bytes.len()) as u64 + parent_bytes;
    ensure!(
        total <= MAX_TOTAL_BYTES,
        "compiler reconciliation exceeds total byte budget"
    );
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        files.push(entry?.path());
        ensure!(
            files.len() <= MAX_FILES,
            "compiler reconciliation exceeds file budget"
        );
    }
    files.sort();
    let expected_queries: BTreeSet<&str> = manifest
        .launches
        .iter()
        .map(|launch| launch.capture_id.as_str())
        .collect();
    let mut journals = Vec::new();
    let mut entries = Vec::new();
    let mut queries = Vec::new();
    let mut task_bindings = std::collections::BTreeMap::new();
    let mut excluded = BTreeSet::new();
    let mut unreconciled = 0usize;
    let mut selected_files = Vec::new();
    let mut documents = Vec::new();
    for file in files {
        if !file.extension().is_some_and(|value| value == "json") {
            continue;
        }
        let name = file
            .file_name()
            .and_then(|value| value.to_str())
            .context("capture filename is not UTF-8")?;
        if file == manifest_path || name == initial_name {
            continue;
        }
        let bytes = read_bytes(&file, MAX_TOTAL_BYTES)?;
        total = total.saturating_add(bytes.len() as u64);
        ensure!(
            total <= MAX_TOTAL_BYTES,
            "compiler reconciliation exceeds total byte budget"
        );
        selected_files.push(file);
        documents.push(bytes);
    }
    let selected_queries: BTreeSet<String> =
        expected_queries.iter().map(|id| (*id).to_owned()).collect();
    // Original bytes are reconciled before query summaries are flattened. The
    // selected launch manifest is the only source of eligible query identities.
    // Resource endorsements can link descendants, never authorize a launch.
    let resource_attachments = crate::execution_graph_resource_attachment::collect_documents(
        &documents,
        Some(&selected_queries),
    )?;
    let pools = crate::execution_graph::collect_pool_inventories(&documents)?;
    let mut selected_pools = BTreeSet::new();
    // Query declarations, not filenames or reused numeric task IDs, select the
    // pool authorities eligible to contribute to this compiler root.
    for bytes in &documents {
        if crate::execution_graph::resource_producer(bytes)?.is_some() {
            continue;
        }
        let header: Header = serde_json::from_slice(bytes).context("invalid capture header")?;
        if header.producer.as_deref() == Some("rustc-query-admission")
            && header
                .capture_id
                .as_deref()
                .is_some_and(|id| expected_queries.contains(id))
        {
            let query = crate::execution_graph_query::collect_bytes(bytes)?;
            if let Some(bindings) = &query.bindings {
                selected_pools.extend(bindings.pool_capture_id().map(str::to_owned));
            }
        }
    }
    for (file, bytes) in selected_files.iter().zip(&documents) {
        let name = file
            .file_name()
            .and_then(|value| value.to_str())
            .context("capture filename is not UTF-8")?;
        if crate::execution_graph::resource_producer(bytes)?.is_some() {
            // Content determines new resource routing even when an arbitrary
            // filename resembles an owner journal. All three roles and exact
            // selected query authority are checked by the original-byte pass.
            continue;
        }
        let header: Header = serde_json::from_slice(bytes).context("invalid capture header")?;
        if let Some(sequence) = name
            .strip_prefix(&format!("{prefix}.event-"))
            .and_then(|suffix| suffix.strip_suffix(".json"))
        {
            let sequence: u64 = sequence
                .parse()
                .context("invalid compiler journal filename")?;
            let journal = protocol::parse_journal(bytes)?;
            ensure!(
                journal.sequence == sequence,
                "journal filename and sequence disagree"
            );
            journals.push(journal);
            continue;
        }
        if header.profile.as_deref() == Some(protocol::PROFILE) {
            if name.contains(".event-") {
                let journal = protocol::parse_journal(bytes)?;
                ensure!(
                    header.authority_root.as_deref() == Some(journal.authority_root.as_str()),
                    "journal authority header disagrees"
                );
                ensure!(
                    name == format!(
                        "compiler-owner-{}.event-{}.json",
                        journal.authority_root, journal.sequence
                    ),
                    "compiler journal filename and authority disagree"
                );
                if journal.authority_root == manifest.authority_root {
                    bail!("selected compiler journal has noncanonical filename");
                }
                excluded.insert(journal.authority_root);
                continue;
            }
            let other = protocol::parse_manifest(bytes)?;
            if other.authority_root == manifest.authority_root {
                // The selected final file can be outside the capture directory,
                // but a contradictory local final may not be silently ignored.
                ensure!(
                    other == manifest,
                    "conflicting selected compiler authority snapshot"
                );
                ensure!(
                    name == format!("{prefix}.manifest.json"),
                    "unexpected selected compiler authority snapshot"
                );
            } else {
                excluded.insert(other.authority_root);
            }
            continue;
        }
        if header.profile.as_deref() == Some("buildrunner-scheduler-entry-v1") {
            let other = crate::execution_graph_manifest::parse_manifest(bytes)?;
            if other.authority_root != manifest.parent_authority_root {
                excluded.insert(other.authority_root);
            } else {
                let stem = format!("owner-{}", manifest.parent_authority_root);
                ensure!(
                    [
                        "manifest",
                        "declaration",
                        "execute-entered",
                        "execute-result"
                    ]
                    .iter()
                    .any(|suffix| name == format!("{stem}.{suffix}.json")),
                    "unexpected selected parent journal observation"
                );
            }
            continue;
        }
        match header.producer.as_deref() {
            Some("rustc-query-admission-entry") => {
                let entry = protocol::parse_entry(bytes)?;
                ensure!(
                    file.file_stem().and_then(|value| value.to_str())
                        == Some(entry.capture_id.as_str()),
                    "entry filename and capture identity disagree"
                );
                if entry.parent_invocation_id == manifest.authority_root {
                    entries.push(entry);
                } else {
                    excluded.insert(entry.parent_invocation_id);
                }
            }
            Some("rustc-query-admission")
                if header
                    .capture_id
                    .as_deref()
                    .is_some_and(|id| expected_queries.contains(id)) =>
            {
                let query = crate::execution_graph::collect_query(bytes, &pools)?;
                ensure!(
                    file.file_stem().and_then(|value| value.to_str())
                        == Some(query.capture_id.as_str()),
                    "query filename and capture identity disagree"
                );
                if let Some(bindings) = query.task_bindings {
                    ensure!(
                        task_bindings
                            .insert(query.capture_id.clone(), bindings)
                            .is_none(),
                        "duplicate selected query task binding authority"
                    );
                }
                queries.push(protocol::QuerySummary {
                    schema: query.schema,
                    capture_id: query.capture_id,
                    structural_complete: query.structural_complete,
                });
            }
            Some("cargo-scheduler")
                if header.capture_id.as_deref() == Some(manifest.scheduler_capture_id.as_str()) => {
            }
            Some("rustc-thread-pool-join")
                if header
                    .capture_id
                    .as_ref()
                    .is_some_and(|id| selected_pools.contains(id)) =>
            {
                // Already validated in pass one and reconciled against the
                // query's complete authority tuple in pass two.
            }
            _ => {
                unreconciled += 1;
            }
        }
    }
    let assessment = protocol::reconcile(
        &manifest, &initial, &journals, &scheduler, &entries, &queries,
    )?;
    let mut output = serde_json::to_value(&assessment)?;
    output["excluded_authority_roots"] = serde_json::to_value(excluded)?;
    output["unreconciled_receipt_files"] = serde_json::to_value(unreconciled)?;
    output["selection"] = serde_json::json!("explicit-compiler-authority");
    output["resource_attachments"] = serde_json::to_value(resource_attachments)?;
    if !task_bindings.is_empty() {
        output["task_bindings"] = serde_json::to_value(task_bindings)?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn write(directory: &Path, name: &str, value: &Value) {
        std::fs::write(directory.join(name), serde_json::to_vec(value).unwrap()).unwrap();
    }

    fn empty_authority(directory: &Path) -> std::path::PathBuf {
        let parent = json!({
            "schema":1,"profile":"buildrunner-scheduler-entry-v1","run_id":"run",
            "authority_root":"parent","owners":[{
                "invocation_id":"parent","parent_invocation_id":null,"parent_task_id":null,
                "sealed":true,"interrupted":false,"lost_events":0,"outcome":"success",
                "tickets":[{"invocation_id":"scheduler-invocation","parent_task_id":"scheduler-task",
                    "policy":"required","lifecycle":"succeeded","execute_entered":true,"capture_id":"scheduler"}]
            }]
        });
        write(directory, "owner-parent.manifest.json", &parent);
        for suffix in ["declaration", "execute-entered", "execute-result"] {
            let mut journal = parent.clone();
            journal["owners"][0]["sealed"] = json!(false);
            journal["owners"][0]["outcome"] = json!("pending");
            journal["owners"][0]["tickets"][0]["lifecycle"] = json!(match suffix {
                "declaration" => "declared",
                "execute-entered" => "execute_entered",
                _ => "succeeded",
            });
            journal["owners"][0]["tickets"][0]["execute_entered"] = json!(suffix != "declaration");
            write(directory, &format!("owner-parent.{suffix}.json"), &journal);
        }
        write(
            directory,
            "scheduler.envelope.json",
            &json!({
                "schema":1,"run_id":"run","invocation_id":"scheduler-invocation",
                "parent_invocation_id":"parent","parent_task_id":"scheduler-task",
                "capture_id":"scheduler","receipt_schema":1
            }),
        );
        write(
            directory,
            "scheduler.json",
            &json!({
                "schema":1,"producer":"cargo-scheduler","scope":"cargo-unit-scheduler",
                "capture_id":"scheduler","structural_complete":true,"full_pipeline_complete":false,
                "issues":[],"declared_nodes":0,"declared_edges":0,"events":[],"nodes":[],"edges":[],
                "opaque_boundaries":["compiler-internals"]
            }),
        );
        let mut root = json!({
            "schema":1,"profile":"buildrunner-compiler-launch-v1","run_id":"run",
            "authority_root":"compiler","parent_authority_root":"parent","scheduler_capture_id":"scheduler",
            "expected_units":0,"expected_launches":0,"expected_journals":0,
            "sealed":false,"interrupted":false,"lost_events":0,"outcome":"pending","units":[],"launches":[]
        });
        write(directory, "compiler-owner-compiler.declaration.json", &root);
        root["sealed"] = json!(true);
        root["outcome"] = json!("success");
        let name = "compiler-owner-compiler.manifest.json";
        write(directory, name, &root);
        directory.join(name)
    }

    #[test]
    fn public_compiler_resource_documents_do_not_create_launch_authority() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| crate::paths::home().unwrap());
        let directory = root.join(format!(
            "compiler-resource-files-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let manifest = empty_authority(&directory);
        let baseline = assess_directory(&directory, &manifest).unwrap();
        assert_eq!(baseline["structural_complete"], true, "{baseline}");
        assert_eq!(baseline["resource_attachments"]["state"], "unknown");
        for (name, producer) in [
            (
                "compiler-owner-compiler.event-999.json",
                "rustc-frontend-allowance",
            ),
            (
                "foreign.manifest.json",
                "rustc-frontend-admission-attachment",
            ),
        ] {
            write(
                &directory,
                name,
                &json!({"schema":1,"producer":producer,"capture_id":name,"profile":42}),
            );
        }
        let candidate = assess_directory(&directory, &manifest).unwrap();
        for key in [
            "structural_complete",
            "expected_launches",
            "accounted_launches",
            "issues",
        ] {
            assert_eq!(
                candidate[key], baseline[key],
                "optional malformed resources changed {key}"
            );
        }
        assert_eq!(candidate["full_pipeline_complete"], false);
        command(&directory, &manifest, true).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn public_compiler_resource_selection_is_exact_query_scoped_not_filename_scoped() {
        let directory = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| crate::paths::home().unwrap())
            .join(format!("compiler-resource-selected-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let manifest = empty_authority(&directory);
        let mut root: Value =
            serde_json::from_slice(&read_bytes(&manifest, MAX_TOTAL_BYTES).unwrap()).unwrap();
        let documents = crate::execution_graph_resource_attachment::tests::documents();
        let query_id = documents[0]["capture_id"].as_str().unwrap();
        root["expected_units"] = json!(1);
        root["expected_launches"] = json!(1);
        // Deliberately omit an independently expected journal. Resources must
        // never repair incomplete compiler-launch authority.
        root["expected_journals"] = json!(1);
        root["units"] = json!([{"unit_id":0,"kind":"rustc","disposition":"required","outcome":"success","terminal":true}]);
        root["launches"] = json!([{
            "invocation_id":"invocation","launch_ordinal":0,"unit_id":0,"parent_task_id":"cargo-unit-0",
            "entry_capture_id":"entry","capture_id":query_id,"spawn_attempts":1,"spawned":true,
            "terminal":true,"outcome":"success","wait_status":0
        }]);
        write(&directory, "compiler-owner-compiler.manifest.json", &root);
        write(&directory, &format!("{query_id}.json"), &documents[0]);
        write(&directory, "pool.json", &documents[1]);
        let baseline = assess_directory(&directory, &manifest).unwrap();
        assert_eq!(baseline["structural_complete"], false);
        for (index, document) in documents.iter().enumerate().skip(2) {
            write(
                &directory,
                &format!("compiler-owner-compiler.event-{}.json", index + 10),
                document,
            );
        }
        let mut foreign = documents.clone();
        foreign[0]["capture_id"] = json!("foreign-query");
        foreign[0]["task_bindings"]["pool"]["capture_id"] = json!("foreign-pool");
        foreign[1]["capture_id"] = json!("foreign-pool");
        foreign[1]["query_capture_id"] = json!("foreign-query");
        foreign[2]["capture_id"] = json!("foreign-resource");
        foreign[2]["attachment_capture_id"] = json!("foreign-attachment");
        for document in &mut foreign[3..] {
            document["capture_id"] =
                json!(format!("foreign-{}", document["role"].as_str().unwrap()));
            document["attachment"]["attachment_capture_id"] = json!("foreign-attachment");
            document["attachment"]["resource_capture_id"] = json!("foreign-resource");
            document["attachment"]["query_capture_id"] = json!("foreign-query");
            document["attachment"]["pool_capture_id"] = json!("foreign-pool");
        }
        for (index, document) in foreign.iter().enumerate() {
            write(&directory, &format!("foreign-{index}.json"), document);
        }
        let candidate = assess_directory(&directory, &manifest).unwrap();
        assert_eq!(
            candidate["resource_attachments"]["state"], "validated",
            "{candidate}"
        );
        let resources = candidate["resource_attachments"]["resources"]
            .as_array()
            .unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0]["capture_id"], documents[2]["capture_id"]);
        for diagnostic in candidate["resource_attachments"]["documents"]
            .as_array()
            .unwrap()
        {
            assert!(
                !diagnostic["capture_id"]
                    .as_str()
                    .unwrap()
                    .starts_with("foreign-")
            );
        }
        for key in [
            "structural_complete",
            "expected_launches",
            "accounted_launches",
            "issues",
        ] {
            assert_eq!(
                candidate[key], baseline[key],
                "resource authority repaired compiler {key}"
            );
        }
        command(&directory, &manifest, true).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
