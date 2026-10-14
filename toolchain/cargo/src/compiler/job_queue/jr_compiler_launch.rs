//! Independent compiler launch expectations, bound to scheduler anonymous Unit ids.
use super::jr_execution_graph::{owner::Owner, publish_json, unique_id};
use crate::util::data_structures::HashMap;
use cargo_util::jr_query_binding::Observer;
use serde::Serialize;
use std::hash::Hash;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::{Arc, Mutex};

const PROFILE: &str = "buildrunner-compiler-launch-v1";
const MAX_RECORDS: usize = 100_000;
const MAX_JOURNALS: u64 = 1_000_000;

#[derive(Clone)]
pub(crate) struct Registry<K: Eq + Hash = u64>(Option<Arc<Mutex<State<K>>>>);
impl<K: Eq + Hash> Default for Registry<K> {
    fn default() -> Self {
        Self(None)
    }
}
struct State<K: Eq + Hash> {
    directory: PathBuf,
    ids: HashMap<K, usize>,
    root: Root,
    finished: bool,
}
#[derive(Serialize)]
struct Root {
    schema: u32,
    profile: &'static str,
    run_id: String,
    authority_root: String,
    parent_authority_root: String,
    scheduler_capture_id: String,
    expected_units: u64,
    expected_launches: u64,
    expected_journals: u64,
    sealed: bool,
    interrupted: bool,
    lost_events: u64,
    outcome: &'static str,
    units: Vec<UnitRecord>,
    launches: Vec<LaunchRecord>,
}
#[derive(Clone, Serialize)]
struct UnitRecord {
    unit_id: u64,
    kind: &'static str,
    disposition: &'static str,
    outcome: &'static str,
    terminal: bool,
}
#[derive(Clone, Serialize)]
struct LaunchRecord {
    invocation_id: String,
    launch_ordinal: u64,
    unit_id: u64,
    parent_task_id: String,
    entry_capture_id: String,
    capture_id: String,
    spawn_attempts: u64,
    spawned: bool,
    terminal: bool,
    outcome: &'static str,
    wait_status: Option<i32>,
}
#[derive(Serialize)]
struct Event<'a> {
    schema: u32,
    profile: &'static str,
    authority_root: &'a str,
    sequence: u64,
    event: &'static str,
    unit: Option<UnitRecord>,
    launch: Option<LaunchRecord>,
}

impl<K: Eq + Hash + Clone + Send + Sync + 'static> Registry<K> {
    pub(crate) fn new(owner: &Owner) -> Self {
        let Some((directory, run_id, parent_authority_root, scheduler_capture_id)) =
            owner.compiler_identity()
        else {
            return Self::default();
        };
        let mut state = State {
            directory,
            ids: HashMap::default(),
            finished: false,
            root: Root {
                schema: 1,
                profile: PROFILE,
                run_id,
                authority_root: unique_id("compiler"),
                parent_authority_root,
                scheduler_capture_id,
                expected_units: 0,
                expected_launches: 0,
                expected_journals: 0,
                sealed: false,
                interrupted: false,
                lost_events: 0,
                outcome: "pending",
                units: vec![],
                launches: vec![],
            },
        };
        state.publish_root("declaration");
        Self(Some(Arc::new(Mutex::new(state))))
    }
    fn update(&self, f: impl FnOnce(&mut State<K>)) {
        if let Some(state) = &self.0 {
            // Poisoning is an observation failure, never a compilation failure.
            match state.lock() {
                Ok(mut state) => f(&mut state),
                Err(poison) => {
                    let mut state = poison.into_inner();
                    state.lost();
                }
            }
        }
    }
    pub(crate) fn declare(&self, unit: &K, id: Option<u64>, kind: &'static str) {
        self.update(|s| {
            s.root.expected_units = s.root.expected_units.saturating_add(1);
            let Some(id) = id else {
                s.lost();
                return;
            };
            if s.ids.contains_key(unit) || s.root.units.len() >= MAX_RECORDS {
                s.lost();
                return;
            }
            s.ids.insert(unit.clone(), s.root.units.len());
            let record = UnitRecord {
                unit_id: id,
                kind,
                disposition: "pending",
                outcome: "pending",
                terminal: false,
            };
            s.root.units.push(record.clone());
            s.event("unit-declared", Some(record), None);
        });
    }
    pub(crate) fn resolve(&self, unit: &K, disposition: &'static str) {
        self.update(|s| {
            let Some(i) = s.unit_index(unit) else {
                s.lost();
                return;
            };
            if s.root.units[i].disposition != "pending" {
                s.lost();
                return;
            }
            s.root.units[i].disposition = if disposition == "required"
                && (!cfg!(target_os = "linux") || s.root.units[i].kind != "rustc")
            {
                "unsupported"
            } else {
                disposition
            };
            s.event("unit-resolution", Some(s.root.units[i].clone()), None);
        });
    }
    pub(crate) fn terminal(&self, unit: &K, success: bool) {
        self.update(|s| {
            let Some(i) = s.unit_index(unit) else {
                s.lost();
                return;
            };
            if s.root.units[i].terminal {
                s.lost();
                return;
            }
            s.root.units[i].terminal = true;
            s.root.units[i].outcome = if success { "success" } else { "failure" };
            s.event("unit-terminal", Some(s.root.units[i].clone()), None);
        });
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn launch(
        &self,
        unit: &K,
        supported: bool,
        receipt_schema: u32,
    ) -> Option<cargo_util::jr_query_binding::Launch> {
        let mut result = None;
        self.update(|s| {
            let Some(i) = s.unit_index(unit) else {
                s.lost();
                return;
            };
            if !supported {
                s.root.units[i].disposition = "unsupported";
                s.event("unit-resolution", Some(s.root.units[i].clone()), None);
                return;
            }
            if s.root.units[i].disposition != "required" {
                s.lost();
                return;
            }
            let ordinal = s.root.expected_launches;
            s.root.expected_launches = s.root.expected_launches.saturating_add(1);
            if s.root.launches.len() >= MAX_RECORDS {
                s.lost();
                return;
            }
            let record = LaunchRecord {
                invocation_id: unique_id("rustc-invocation"),
                launch_ordinal: ordinal,
                unit_id: s.root.units[i].unit_id,
                parent_task_id: format!("cargo-unit-{}", s.root.units[i].unit_id),
                entry_capture_id: unique_id("rustc-entry"),
                capture_id: unique_id("rustc-query"),
                spawn_attempts: 0,
                spawned: false,
                terminal: false,
                outcome: "pending",
                wait_status: None,
            };
            let binding = cargo_util::jr_query_binding::Binding {
                schema: 1,
                protocol: "jr-rustc-query-binding-v1".into(),
                run_id: s.root.run_id.clone(),
                invocation_id: record.invocation_id.clone(),
                parent_invocation_id: s.root.authority_root.clone(),
                parent_task_id: record.parent_task_id.clone(),
                entry_capture_id: record.entry_capture_id.clone(),
                capture_id: record.capture_id.clone(),
                receipt_schema,
            };
            s.root.launches.push(record.clone());
            s.event("launch-declared", None, Some(record));
            result = Some(cargo_util::jr_query_binding::Launch {
                directory: s.directory.clone(),
                binding,
                observer: Arc::new(LaunchObserver {
                    registry: self.clone(),
                    ordinal,
                }),
            });
        });
        result
    }
    pub(crate) fn finish(&self, success: bool) {
        self.update(|s| {
            if s.finished {
                s.lost();
                return;
            }
            for i in 0..s.root.units.len() {
                if !s.root.units[i].terminal {
                    s.root.units[i].terminal = true;
                    s.root.units[i].outcome = if s.root.units[i].disposition == "pending" {
                        "not_started"
                    } else {
                        "cancelled"
                    };
                    s.event("unit-terminal", Some(s.root.units[i].clone()), None);
                }
            }
            s.root.sealed = true;
            s.root.outcome = if success { "success" } else { "failure" };
            s.finished = true;
            s.publish_root("manifest");
        });
    }
}
impl<K: Eq + Hash> State<K> {
    fn lost(&mut self) {
        self.root.lost_events = self.root.lost_events.saturating_add(1);
    }
    fn unit_index(&self, unit: &K) -> Option<usize> {
        self.ids.get(unit).copied()
    }
    fn publish_root(&mut self, suffix: &str) {
        let stem = format!("compiler-owner-{}.{}", self.root.authority_root, suffix);
        if publish_json(&self.directory, &stem, &self.root, 64 * 1024 * 1024).is_err() {
            self.lost();
        }
    }
    fn event(
        &mut self,
        event: &'static str,
        unit: Option<UnitRecord>,
        launch: Option<LaunchRecord>,
    ) {
        self.root.expected_journals = self.root.expected_journals.saturating_add(1);
        let sequence = self.root.expected_journals;
        if sequence > MAX_JOURNALS {
            self.lost();
            return;
        }
        let value = Event {
            schema: 1,
            profile: PROFILE,
            authority_root: &self.root.authority_root,
            sequence,
            event,
            unit,
            launch,
        };
        let stem = format!(
            "compiler-owner-{}.event-{sequence}",
            self.root.authority_root
        );
        if publish_json(&self.directory, &stem, &value, 16 * 1024).is_err() {
            self.lost();
        }
    }
}
impl<K: Eq + Hash> Drop for State<K> {
    fn drop(&mut self) {
        if !self.finished {
            self.root.interrupted = true;
            self.root.outcome = "failure";
            self.publish_root("interrupted.manifest");
        }
    }
}
struct LaunchObserver<K: Eq + Hash> {
    registry: Registry<K>,
    ordinal: u64,
}
impl<K: Eq + Hash + Clone + Send + Sync + 'static> LaunchObserver<K> {
    fn event(&self, name: &'static str, update: impl FnOnce(&mut LaunchRecord) -> bool) {
        self.registry.update(|s| {
            let Ok(i) = usize::try_from(self.ordinal) else {
                s.lost();
                return;
            };
            if s.root
                .launches
                .get(i)
                .is_none_or(|l| l.launch_ordinal != self.ordinal)
            {
                s.lost();
                return;
            }
            if !update(&mut s.root.launches[i]) {
                s.lost();
            }
            s.event(name, None, Some(s.root.launches[i].clone()));
        });
    }
}
impl<K: Eq + Hash + Clone + Send + Sync + 'static> Observer for LaunchObserver<K> {
    fn spawn_attempt(&self) {
        self.event("spawn-attempt", |l| {
            if l.terminal || l.spawned {
                return false;
            }
            l.spawn_attempts = l.spawn_attempts.saturating_add(1);
            true
        });
    }
    fn spawn_result(&self, spawned: bool, retry: bool) {
        self.event("spawn-result", |l| {
            if l.terminal || l.spawned || l.spawn_attempts == 0 {
                return false;
            }
            l.spawned = spawned;
            if !spawned && !retry {
                l.terminal = true;
                l.outcome = "spawn_failed";
            }
            true
        });
    }
    fn wait(&self, status: ExitStatus) {
        self.event("wait-result", |l| {
            if !l.spawned || l.terminal {
                return false;
            }
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                l.wait_status = Some(status.into_raw());
            }
            l.terminal = true;
            l.outcome = if status.success() {
                "success"
            } else {
                "failure"
            };
            true
        });
    }
    fn lost(&self) {
        self.registry.update(State::lost);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
