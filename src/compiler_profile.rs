//! Bounded JSON profile output. Never serializes CFG or ownership payloads.
use justrust_engine::{Report, compiler::ir::PlanUsageSnapshot};
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};

mod dependencies;

pub(crate) struct Profile<'a> {
    pub mode: &'a str,
    pub workers: usize,
    pub compiler_wall_ns: u128,
    pub report: Option<&'a Report>,
    pub resource_usage: &'a PlanUsageSnapshot,
    pub allocator_json: &'a str,
    pub error: Option<&'a str>,
}

pub(crate) fn json_string(out: &mut impl Write, text: &str) -> io::Result<()> {
    write!(out, "\"")?;
    for ch in text.chars() {
        match ch {
            '"' => write!(out, "\\\"")?,
            '\\' => write!(out, "\\\\")?,
            '\n' => write!(out, "\\n")?,
            '\r' => write!(out, "\\r")?,
            '\t' => write!(out, "\\t")?,
            c if c < ' ' => write!(out, "\\u{:04x}", c as u32)?,
            c => write!(out, "{c}")?,
        }
    }
    write!(out, "\"")
}

impl Profile<'_> {
    pub(crate) fn write_json(&self, out: &mut impl Write) -> io::Result<()> {
        write!(out, "{{\"schema\":1,\"mode\":")?;
        json_string(out, self.mode)?;
        write!(
            out,
            ",\"workers\":{},\"compiler_wall_ns\":{},\"success\":{},\"error\":",
            self.workers,
            self.compiler_wall_ns,
            self.error.is_none()
        )?;
        if let Some(error) = self.error {
            json_string(out, error)?;
        } else {
            write!(out, "null")?;
        }
        write!(out, ",\"planning_budget\":")?;
        write!(out, "{}", self.resource_usage.to_json())?;
        write!(out, ",\"first_failure_task\":")?;
        let failure_node = self
            .resource_usage
            .first_failure
            .and_then(|failure| failure.task_id)
            .and_then(|task| {
                self.report?
                    .events
                    .iter()
                    .find(|event| event.task == Some(task))?
                    .node
            });
        let failure_key = self.report.and_then(|report| {
            report
                .node_ids
                .iter()
                .zip(&report.node_evaluations)
                .find(|(id, _)| Some(**id) == failure_node)
        });
        if let Some((node, (key, _))) = failure_key {
            write!(
                out,
                "{{\"node\":{node},\"kind\":{},\"phase\":\"{}\",\"params\":[",
                key.kind,
                phase(key.kind)
            )?;
            for (index, parameter) in key.params.iter().enumerate() {
                if index != 0 {
                    write!(out, ",")?;
                }
                write!(out, "{parameter}")?;
            }
            write!(out, "]}}")?;
        } else {
            write!(out, "null")?;
        }
        write!(
            out,
            ",\"allocator\":{},\"measurement\":\"contended/uncontrolled\",\"cpu_sampling\":\"external profiler required\",\"graph\":",
            self.allocator_json
        )?;
        if let Some(report) = self.report {
            write_graph(out, report)?;
        } else {
            write!(out, "null")?;
        }
        writeln!(out, "}}")
    }
}

fn phase(kind: u32) -> &'static str {
    match kind {
        101 => "signatures",
        102 => "body",
        103 => "object",
        104 => "check",
        105 => "build",
        106 => "resolve",
        107 => "source",
        108 => "lower",
        109 => "analyze",
        110 => "template_body",
        111 => "universal_lower",
        112 => "universal_analyze",
        114 => "concrete_proofs",
        115 => "effects_ready",
        116 => "body_lexical",
        117 => "capture_env",
        118 => "const_decl",
        119 => "const_body",
        120 => "const_value",
        125 => "root_cleanup_demand",
        126 => "canonical_body",
        127 => "opaque_representation",
        128 => "source_admission",
        130 => "ssa",
        131 => "local_opt",
        132 => "summary",
        133 => "ipo_opt",
        134 => "associated_const_header",
        135 => "associated_const_select",
        136 => "associated_const_value",
        138 => "nominal_decl_well_formed",
        139 => "nominal_realization",
        140 => "opaque_call_dispatch",
        141 => "item_inventory",
        142 => "instance",
        143 => "signature",
        144 => "trait_impls",
        145 => "impl_coherence",
        146 => "item_coherence",
        121 => "parse_file",
        148 => "resolve_module",
        149 => "signature_module",
        147 => "module_decls",
        150 => "admit",
        _ => "other",
    }
}

fn write_graph(out: &mut impl Write, report: &Report) -> io::Result<()> {
    // Event storage is bounded by the engine. These maps cannot exceed that bound.
    let mut active = HashMap::new();
    let mut times = BTreeMap::<u64, u64>::new();
    let mut unmatched = 0usize;
    for event in &report.events {
        let (Some(task), Some(node)) = (event.task, event.node) else {
            continue;
        };
        if event.kind == "running" {
            if active.insert(task, (node, event.at_ns)).is_some() {
                unmatched += 1;
            }
        } else if matches!(event.kind.as_str(), "suspended" | "queued" | "task-done")
            && let Some((running_node, start)) = active.remove(&task)
        {
            let entry = times.entry(running_node).or_default();
            *entry = entry.saturating_add(event.at_ns.saturating_sub(start));
        }
    }
    unmatched += active.len();
    let mut phases = BTreeMap::<u32, u64>::new();
    let mut rows = Vec::with_capacity(report.node_ids.len());
    for (id, (key, evaluations)) in report.node_ids.iter().zip(&report.node_evaluations) {
        let ns = times.get(id).copied().unwrap_or(0);
        let entry = phases.entry(key.kind).or_default();
        *entry = entry.saturating_add(ns);
        rows.push((*id, key, *evaluations, ns));
    }
    rows.sort_by_key(|row| (std::cmp::Reverse(row.3), row.0));
    let max_rows = 128;
    write!(
        out,
        "{{\"locality_workers\":{locality_workers},\"locality_cpus\":{locality_cpus:?},\"elapsed_ns\":{},\"summed_poll_execution_ns\":{},\"queue_ns\":{},\"wait_ns\":{},\"nodes\":{},\"edges\":{},\"polls\":{},\"failures\":{},\"events_dropped\":{},\"capture_complete\":{},\"unmatched_intervals\":{},\"interval_measurement\":\"summed task wall intervals, not CPU time; includes transition bookkeeping\",\"phase_intervals\":[",
        report.elapsed_ns,
        report.execution_ns,
        report.queue_ns,
        report.wait_ns,
        report.nodes,
        report.edges,
        report.polls,
        report.failures,
        report.events_dropped,
        report.capture_complete,
        unmatched,
        locality_workers = report.locality_workers,
        locality_cpus = report.locality_cpus
    )?;
    for (i, (kind, ns)) in phases.iter().enumerate() {
        if i != 0 {
            write!(out, ",")?;
        }
        write!(
            out,
            "{{\"kind\":{kind},\"phase\":\"{}\",\"summed_wall_ns\":{ns}}}",
            phase(*kind)
        )?;
    }
    write!(
        out,
        "],\"top_nodes_limit\":{max_rows},\"omitted_nodes\":{},\"top_nodes\":[",
        rows.len().saturating_sub(max_rows)
    )?;
    for (i, (id, key, evaluations, ns)) in rows.iter().take(max_rows).enumerate() {
        if i != 0 {
            write!(out, ",")?;
        }
        write!(
            out,
            "{{\"id\":{id},\"kind\":{},\"version\":{},\"phase\":\"{}\",\"evaluations\":{evaluations},\"summed_wall_ns\":{ns},\"params\":[",
            key.kind,
            key.version,
            phase(key.kind)
        )?;
        for (j, param) in key.params.iter().enumerate() {
            if j != 0 {
                write!(out, ",")?;
            }
            write!(out, "{param}")?;
        }
        write!(out, "]}}")?;
    }
    write!(out, "],\"dependency_leverage\":")?;
    dependencies::write(out, report, &times, unmatched)?;
    write!(out, "}}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use justrust_engine::{Event, Key};
    #[test]
    fn frontend_and_backend_costs_have_distinct_phase_labels() {
        for (kind, name) in [
            (128, "source_admission"),
            (126, "canonical_body"),
            (138, "nominal_decl_well_formed"),
            (139, "nominal_realization"),
            (141, "item_inventory"),
            (143, "signature"),
            (121, "parse_file"),
            (148, "resolve_module"),
            (149, "signature_module"),
            (147, "module_decls"),
            (103, "object"),
            (105, "build"),
        ] {
            assert_eq!(phase(kind), name);
        }
        assert_eq!(phase(u32::MAX), "other");
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)] // The cycle witness is private engine authority.
    fn profile_intervals_keep_task_identity_and_escape_errors() {
        let event = |at_ns, kind: &str, task, node| Event {
            at_ns,
            kind: kind.into(),
            task: Some(task),
            node: Some(node),
            demand: None,
            detail: String::new(),
        };
        let mut report = Report::default();
        report.node_ids = vec![3, 7];
        report.node_evaluations = vec![(Key::new(109, vec![2]), 1), (Key::new(108, vec![2]), 1)];
        report.events = vec![
            event(10, "running", 1, 3),
            event(12, "running", 2, 7),
            event(20, "suspended", 1, 3),
            event(25, "task-done", 2, 7),
            event(30, "running", 1, 3),
            event(40, "task-done", 1, 3),
        ];
        let mut out = Vec::new();
        write_graph(&mut out, &report).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\"unmatched_intervals\":0"));
        assert!(text.contains("\"phase\":\"analyze\",\"summed_wall_ns\":20"));
        assert!(text.contains("\"phase\":\"lower\",\"summed_wall_ns\":13"));
        let mut out = Vec::new();
        json_string(&mut out, "\"\\\n\u{1}").unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "\"\\\"\\\\\\n\\u0001\"");
    }
}
