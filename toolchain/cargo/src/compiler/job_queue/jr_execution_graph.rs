//! Opt-in observation of the actual Cargo scheduler, not a whole-pipeline graph.
//! No command, source path, package identity or diagnostic is serialized.
use crate::util::data_structures::HashMap;
use std::hash::Hash;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

const MAX_RECEIPT_BYTES: usize = 64 * 1024 * 1024;
const MAX_NODES: usize = 100_000;
const MAX_EDGES: usize = 1_000_000;
const MAX_EVENTS: usize = 1_000_000;
static NEXT_CAPTURE: AtomicUsize = AtomicUsize::new(0);

pub(super) struct Capture<K: Eq + Hash> {
    enabled: Option<Enabled<K>>,
}

struct Enabled<K: Eq + Hash> {
    dir: PathBuf,
    started: Instant,
    ids: HashMap<K, usize>,
    fresh: HashMap<usize, bool>,
    receipt: Receipt,
    sealed: bool,
    finalized: bool,
}

#[derive(Serialize)]
struct Receipt {
    schema: u32,
    producer: &'static str,
    scope: &'static str,
    capture_id: String,
    structural_complete: bool,
    full_pipeline_complete: bool,
    declared_nodes: usize,
    declared_edges: usize,
    issues: Vec<&'static str>,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    events: Vec<Event>,
    opaque_boundaries: Vec<&'static str>,
}

#[derive(Serialize)]
struct Node {
    id: u64,
    kind: &'static str,
    start_ns: Option<u64>,
    end_ns: Option<u64>,
    outcome: Option<&'static str>,
}

#[derive(Serialize)]
struct Edge {
    from: u64,
    to: u64,
    artifact: &'static str,
}

#[derive(Serialize)]
struct Event {
    unit: u64,
    event: &'static str,
    at_ns: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifact: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<&'static str>,
}

impl<K: Eq + Hash + Clone> Capture<K> {
    pub(super) fn from_env(gctx: &crate::util::GlobalContext) -> Self {
        Self::new(
            gctx.get_env_os("JUSTRUST_EXECUTION_GRAPH_DIR")
                .map(PathBuf::from),
        )
    }

    fn new(dir: Option<PathBuf>) -> Self {
        Self {
            enabled: dir.map(|dir| {
                let epoch = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default();
                let capture_id = format!(
                    "cargo-{}-{}-{}",
                    std::process::id(),
                    epoch.as_nanos(),
                    NEXT_CAPTURE.fetch_add(1, Ordering::Relaxed)
                );
                Enabled {
                    dir,
                    started: Instant::now(),
                    ids: HashMap::default(),
                    fresh: HashMap::default(),
                    sealed: false,
                    finalized: false,
                    receipt: Receipt {
                        schema: 1,
                        producer: "cargo-scheduler",
                        scope: "cargo-unit-scheduler",
                        capture_id,
                        structural_complete: false,
                        full_pipeline_complete: false,
                        declared_nodes: 0,
                        declared_edges: 0,
                        issues: Vec::new(),
                        nodes: Vec::new(),
                        edges: Vec::new(),
                        events: Vec::new(),
                        opaque_boundaries: vec![
                            "rustc-internals",
                            "build-script-processes",
                            "proc-macro-execution",
                            "linker-internals",
                            "test-execution",
                            "cargo-resolution-and-fingerprints",
                            "cargo-jobserver-and-resource-waits",
                            "cache-and-client-service-work",
                        ],
                    },
                }
            }),
        }
    }

    pub(super) fn declare<'a>(
        &mut self,
        unit: &K,
        deps: impl Iterator<Item = (&'a K, &'static str)>,
    ) where
        K: 'a,
    {
        let Some(c) = self.enabled.as_mut() else {
            return;
        };
        if c.sealed {
            c.issue("declaration-after-seal");
        }
        let Some(from) = c.id(unit) else { return };
        // A referenced prerequisite is not necessarily enqueued yet. Its declaration
        // is checked independently at seal time.
        if c.receipt.nodes[from].kind == "cargo-unit" {
            c.issue("duplicate-declaration");
        }
        c.receipt.nodes[from].kind = "cargo-unit";
        c.receipt.declared_nodes += 1;
        for (dep, artifact) in deps {
            c.receipt.declared_edges += 1;
            if c.receipt.edges.len() >= MAX_EDGES {
                c.issue("edge-limit");
                continue;
            }
            if let Some(to) = c.id(dep) {
                c.receipt.edges.push(Edge {
                    from: from as u64,
                    to: to as u64,
                    artifact,
                });
            }
        }
    }

    pub(super) fn seal(&mut self) {
        if let Some(c) = self.enabled.as_mut() {
            c.sealed = true;
            if c.receipt.nodes.iter().any(|n| n.kind != "cargo-unit") {
                c.issue("undeclared-prerequisite");
            }
            // Verify the captured topology independently of scheduler assumptions.
            let mut incoming = vec![0usize; c.receipt.nodes.len()];
            let mut outgoing = vec![Vec::new(); c.receipt.nodes.len()];
            for edge in &c.receipt.edges {
                incoming[edge.to as usize] += 1;
                outgoing[edge.from as usize].push(edge.to as usize);
            }
            let mut ready: Vec<_> = incoming
                .iter()
                .enumerate()
                .filter_map(|(id, &count)| (count == 0).then_some(id))
                .collect();
            let mut visited = 0;
            while let Some(id) = ready.pop() {
                visited += 1;
                for &next in &outgoing[id] {
                    incoming[next] -= 1;
                    if incoming[next] == 0 {
                        ready.push(next);
                    }
                }
            }
            if visited != c.receipt.nodes.len() {
                c.issue("dependency-cycle");
            }
        }
    }

    pub(super) fn start(&mut self, unit: &K, fresh: bool) {
        let Some(c) = self.enabled.as_mut() else {
            return;
        };
        let Some(&id) = c.ids.get(unit) else {
            c.issue("undeclared-start");
            return;
        };
        if c.receipt.nodes[id].start_ns.is_some() || c.receipt.nodes[id].outcome.is_some() {
            c.issue("duplicate-start");
            return;
        }
        let now = c.now();
        c.receipt.nodes[id].start_ns = Some(now);
        c.fresh.insert(id, fresh);
        c.event(id, "start", now, None, None);
    }

    pub(super) fn finish(&mut self, unit: &K, artifact: &'static str, success: bool) {
        let Some(c) = self.enabled.as_mut() else {
            return;
        };
        let Some(&id) = c.ids.get(unit) else {
            c.issue("undeclared-finish");
            return;
        };
        if c.receipt.nodes[id].start_ns.is_none() || c.receipt.nodes[id].outcome.is_some() {
            c.issue("invalid-finish");
            return;
        }
        let now = c.now();
        if artifact == "rmeta" && success {
            c.event(id, "artifact-ready", now, Some(artifact), None);
        } else {
            let outcome = if !success {
                "failure"
            } else if c.fresh.get(&id) == Some(&true) {
                "fresh"
            } else {
                "success"
            };
            c.terminal(id, now, outcome);
        }
    }

    pub(super) fn finalize(&mut self) {
        if let Some(c) = self.enabled.as_mut() {
            c.finalize(false);
        }
    }
}

impl<K: Eq + Hash + Clone> Enabled<K> {
    fn id(&mut self, unit: &K) -> Option<usize> {
        if let Some(&id) = self.ids.get(unit) {
            return Some(id);
        }
        if self.receipt.nodes.len() >= MAX_NODES {
            self.issue("node-limit");
            return None;
        }
        let id = self.receipt.nodes.len();
        self.ids.insert(unit.clone(), id);
        self.receipt.nodes.push(Node {
            id: id as u64,
            kind: "prerequisite",
            start_ns: None,
            end_ns: None,
            outcome: None,
        });
        Some(id)
    }
}

impl<K: Eq + Hash> Enabled<K> {
    fn now(&self) -> u64 {
        self.started.elapsed().as_nanos().min(u64::MAX as u128) as u64
    }
    fn issue(&mut self, issue: &'static str) {
        if !self.receipt.issues.contains(&issue) {
            self.receipt.issues.push(issue);
        }
    }
    fn event(
        &mut self,
        unit: usize,
        event: &'static str,
        at_ns: u64,
        artifact: Option<&'static str>,
        outcome: Option<&'static str>,
    ) {
        if self.receipt.events.len() >= MAX_EVENTS {
            self.issue("event-limit");
            return;
        }
        self.receipt.events.push(Event {
            unit: unit as u64,
            event,
            at_ns,
            artifact,
            outcome,
        });
    }
    fn terminal(&mut self, id: usize, now: u64, outcome: &'static str) {
        self.receipt.nodes[id].outcome = Some(outcome);
        if self.receipt.nodes[id].start_ns.is_some() {
            self.receipt.nodes[id].end_ns = Some(now);
        }
        self.event(id, "terminal", now, None, Some(outcome));
    }
    fn finalize(&mut self, interrupted: bool) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        if interrupted {
            self.issue("interrupted");
        }
        if !self.sealed {
            self.issue("unsealed-declarations");
        }
        let now = self.now();
        for id in 0..self.receipt.nodes.len() {
            if self.receipt.nodes[id].outcome.is_none() {
                let outcome = if self.receipt.nodes[id].start_ns.is_some() {
                    "cancelled"
                } else {
                    "not_started"
                };
                if outcome == "cancelled" {
                    self.issue("unfinished-started-unit");
                }
                self.terminal(id, now, outcome);
            }
        }
        self.receipt.structural_complete = self.sealed && self.receipt.issues.is_empty();
        // A receipt is best-effort and never influences a build result. Create-new
        // and private permissions prevent replacement or disclosure of prior output.
        let _ = self.write();
    }
    fn write(&self) -> std::io::Result<()> {
        self.write_with_budget(MAX_RECEIPT_BYTES)
    }

    fn write_with_budget(&self, budget: usize) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(format!("{}.json", self.receipt.capture_id));
        let partial = self
            .dir
            .join(format!("{}.partial", self.receipt.capture_id));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(&partial)?;
        // Only own cleanup after create-new succeeds. Never remove another
        // capture's pre-existing partial on an open error.
        let _cleanup = PartialReceipt(partial.clone());
        let mut writer = CappedWriter {
            inner: std::io::BufWriter::new(file),
            remaining: budget,
        };
        serde_json::to_writer(&mut writer, &self.receipt)?;
        writer.flush()?;
        drop(writer);
        // A hard link publishes the finished bytes atomically without replacing
        // an existing receipt. Both names live in the same directory/filesystem.
        std::fs::hard_link(&partial, path)?;
        Ok(())
    }
}

struct CappedWriter<W> {
    inner: W,
    remaining: usize,
}

impl<W: Write> Write for CappedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(std::io::Error::other(
                "execution graph receipt exceeds byte limit",
            ));
        }
        let written = self.inner.write(bytes)?;
        self.remaining -= written;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

struct PartialReceipt(PathBuf);

impl Drop for PartialReceipt {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl<K: Eq + Hash> Drop for Capture<K> {
    fn drop(&mut self) {
        if let Some(c) = self.enabled.as_mut() {
            c.finalize(true);
        }
    }
}

#[cfg(test)]
mod tests;
