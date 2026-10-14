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
    let mut excluded = BTreeSet::new();
    let mut unreconciled = 0usize;
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
        if let Some(sequence) = name
            .strip_prefix(&format!("{prefix}.event-"))
            .and_then(|suffix| suffix.strip_suffix(".json"))
        {
            let sequence: u64 = sequence
                .parse()
                .context("invalid compiler journal filename")?;
            let journal = protocol::parse_journal(&bytes)?;
            ensure!(
                journal.sequence == sequence,
                "journal filename and sequence disagree"
            );
            journals.push(journal);
            continue;
        }
        let header: Header = serde_json::from_slice(&bytes).context("invalid capture header")?;
        if header.profile.as_deref() == Some(protocol::PROFILE) {
            if name.contains(".event-") {
                let journal = protocol::parse_journal(&bytes)?;
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
            let other = protocol::parse_manifest(&bytes)?;
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
            let other = crate::execution_graph_manifest::parse_manifest(&bytes)?;
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
                let entry = protocol::parse_entry(&bytes)?;
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
                let query = crate::execution_graph_query::assess_bytes(&bytes)?;
                ensure!(
                    file.file_stem().and_then(|value| value.to_str())
                        == Some(query.capture_id.as_str()),
                    "query filename and capture identity disagree"
                );
                queries.push(protocol::QuerySummary {
                    schema: query.schema,
                    capture_id: query.capture_id,
                    structural_complete: query.structural_complete,
                });
            }
            Some("cargo-scheduler")
                if header.capture_id.as_deref() == Some(manifest.scheduler_capture_id.as_str()) => {
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
    if !json {
        println!("Selected native compiler-launch coverage, not full-pipeline coverage:");
    }
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
