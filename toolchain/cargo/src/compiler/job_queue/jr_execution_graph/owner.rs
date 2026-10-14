//! One BuildRunner entry, not an outer Cargo invocation or whole-run authority.
//! Journal publication is independent of the scheduler receipt writer.
use super::{publish_json, unique_id};
use serde::Serialize;
use std::path::PathBuf;

const MAX_OWNER_BYTES: usize = 16 * 1024;

pub(crate) struct Owner {
    enabled: Option<Enabled>,
}

struct Enabled {
    dir: PathBuf,
    manifest: Manifest,
    finished: bool,
}

#[derive(Serialize)]
struct Manifest {
    schema: u32,
    profile: &'static str,
    run_id: String,
    authority_root: String,
    owners: [OwnerRecord; 1],
}

#[derive(Serialize)]
struct OwnerRecord {
    invocation_id: String,
    parent_invocation_id: Option<String>,
    parent_task_id: Option<String>,
    sealed: bool,
    interrupted: bool,
    lost_events: u32,
    outcome: &'static str,
    tickets: [Ticket; 1],
}

#[derive(Serialize)]
struct Ticket {
    invocation_id: String,
    parent_task_id: &'static str,
    policy: &'static str,
    lifecycle: &'static str,
    execute_entered: bool,
    capture_id: String,
}

#[derive(Clone, Serialize)]
pub(super) struct Envelope {
    schema: u32,
    run_id: String,
    invocation_id: String,
    parent_invocation_id: String,
    parent_task_id: &'static str,
    pub(super) capture_id: String,
    receipt_schema: u32,
}

impl Owner {
    pub(crate) fn from_env(gctx: &crate::util::GlobalContext) -> Self {
        Self::new(
            gctx.get_env_os("JUSTRUST_EXECUTION_GRAPH_DIR")
                .map(PathBuf::from),
        )
    }

    fn new(dir: Option<PathBuf>) -> Self {
        let enabled = dir.map(|dir| {
            let id = unique_id("buildrunner");
            let mut enabled = Enabled {
                dir,
                finished: false,
                manifest: Manifest {
                    schema: 1,
                    profile: "buildrunner-scheduler-entry-v1",
                    run_id: id.clone(),
                    authority_root: id.clone(),
                    owners: [OwnerRecord {
                        invocation_id: id,
                        parent_invocation_id: None,
                        parent_task_id: None,
                        sealed: false,
                        interrupted: false,
                        lost_events: 0,
                        outcome: "pending",
                        tickets: [Ticket {
                            invocation_id: unique_id("scheduler"),
                            parent_task_id: "scheduler-0",
                            policy: "required",
                            lifecycle: "declared",
                            execute_entered: false,
                            capture_id: unique_id("cargo"),
                        }],
                    }],
                },
            };
            // Persist positive expectation before constructing the child queue.
            enabled.publish("declaration");
            enabled
        });
        Self { enabled }
    }

    pub(super) fn binding(&self) -> Option<(PathBuf, Envelope)> {
        self.enabled.as_ref().map(|e| {
            let owner = &e.manifest.owners[0];
            let ticket = &owner.tickets[0];
            (
                e.dir.clone(),
                Envelope {
                    schema: 1,
                    run_id: e.manifest.run_id.clone(),
                    invocation_id: ticket.invocation_id.clone(),
                    parent_invocation_id: owner.invocation_id.clone(),
                    parent_task_id: ticket.parent_task_id,
                    capture_id: ticket.capture_id.clone(),
                    receipt_schema: 1,
                },
            )
        })
    }

    pub(crate) fn execute_enter(&mut self) {
        if let Some(e) = &mut self.enabled {
            e.manifest.owners[0].tickets[0].execute_entered = true;
            e.manifest.owners[0].tickets[0].lifecycle = "execute_entered";
            e.publish("execute-entered");
        }
    }

    pub(crate) fn execute_result(&mut self, success: bool) {
        if let Some(e) = &mut self.enabled {
            e.manifest.owners[0].tickets[0].lifecycle =
                if success { "succeeded" } else { "failed" };
            e.publish("execute-result");
        }
    }

    pub(crate) fn finish(&mut self, success: bool) {
        if let Some(e) = &mut self.enabled {
            let owner = &mut e.manifest.owners[0];
            owner.sealed = true;
            owner.interrupted = false;
            owner.outcome = if success { "success" } else { "failed" };
            if owner.tickets[0].lifecycle == "declared" {
                owner.tickets[0].lifecycle = "not_started";
            }
            e.finished = true;
            e.publish("manifest");
        }
    }
}

impl Enabled {
    fn publish(&mut self, suffix: &str) {
        let stem = format!("owner-{}.{}", self.manifest.authority_root, suffix);
        if publish_json(&self.dir, &stem, &self.manifest, MAX_OWNER_BYTES).is_err() {
            self.manifest.owners[0].lost_events =
                self.manifest.owners[0].lost_events.saturating_add(1);
        }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(e) = &mut self.enabled {
            if !e.finished {
                let owner = &mut e.manifest.owners[0];
                owner.interrupted = true;
                owner.outcome = "interrupted";
                // Never invent not_started after execute-enter, nor seal on Drop.
                e.publish("interrupted.manifest");
            }
        }
    }
}

#[cfg(test)]
mod tests;
