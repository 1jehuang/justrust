//! Strict validation of the native join slice, never a full scheduler graph.
use anyhow::{Result, bail};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_EVENTS: usize = 100_000;
const MAX_TASKS: u64 = 100_000;
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
    observer: u64,
    full_pipeline_complete: bool,
    #[serde(deserialize_with = "nullable")]
    cpu_work_ns: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    execution_span_ns: Option<u64>,
    scope_coverage: bool,
    scheduler_coverage: bool,
    pre_attach_coverage: bool,
    sealed: bool,
    interface_outcome: Outcome,
    task_allocator_baseline: u64,
    task_allocator_high_water: u64,
    expected_tasks: u64,
    dropped_events: u64,
    events: Vec<Event>,
}
#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Returned,
    Unwound,
    Unknown,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Terminal {
    Normal,
    Panic,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Root {
        task: u64,
        at_ns: u64,
    },
    Fork {
        parent: u64,
        child: u64,
        continuation: u64,
        at_ns: u64,
    },
    Start {
        task: u64,
        at_ns: u64,
    },
    Finish {
        task: u64,
        outcome: Terminal,
        at_ns: u64,
    },
    Join {
        branch: u64,
        child: u64,
        continuation: u64,
        at_ns: u64,
    },
    Resume {
        continuation: u64,
        at_ns: u64,
    },
}
#[derive(Clone, Copy)]
enum Definition {
    Root,
    Child,
    Continuation(u64),
}
#[derive(Default)]
struct Task {
    definition: Option<Definition>,
    defined_at: Option<u64>,
    start: Option<u64>,
    finish: Option<u64>,
    resume: Option<u64>,
}
#[derive(Default)]
struct Group {
    parent: Option<u64>,
    branches: BTreeSet<u64>,
    joins: BTreeMap<u64, (u64, u64)>,
}
#[derive(Serialize)]
pub(crate) struct Assessment {
    pub schema: u32,
    pub producer: String,
    pub scope: String,
    pub capture_id: String,
    pub structural_complete: bool,
    pub full_pipeline_complete: bool,
    pub nodes: usize,
    pub edges: usize,
    pub issues: Vec<String>,
    pub opaque_boundaries: Vec<String>,
    pub cpu_work_ns: Option<u64>,
    pub execution_span_ns: Option<u64>,
}
fn define(tasks: &mut BTreeMap<u64, Task>, id: u64, definition: Definition, at: u64) -> Result<()> {
    let task = tasks.entry(id).or_default();
    if task.definition.replace(definition).is_some() {
        bail!("duplicate task definition");
    }
    task.defined_at = Some(at);
    Ok(())
}
fn lineage(
    tasks: &BTreeMap<u64, Task>,
    successors: &BTreeMap<u64, u64>,
    forward: bool,
) -> Result<BTreeMap<u64, Option<u64>>> {
    let mut result = BTreeMap::new();
    for &initial in tasks.keys() {
        if result.contains_key(&initial) {
            continue;
        }
        let mut trail = Vec::new();
        let mut seen = BTreeSet::new();
        let mut id = initial;
        let terminal = loop {
            if let Some(known) = result.get(&id) {
                break *known;
            }
            if !seen.insert(id) {
                bail!("continuation lineage cycle");
            }
            trail.push(id);
            if forward {
                match successors.get(&id) {
                    Some(next) => id = *next,
                    None => break Some(id),
                }
            } else {
                match tasks.get(&id).and_then(|t| t.definition) {
                    Some(Definition::Continuation(parent)) => id = parent,
                    Some(Definition::Root | Definition::Child) => break Some(id),
                    None => break None,
                }
            }
        };
        for id in trail {
            result.insert(id, terminal);
        }
    }
    Ok(result)
}
pub(crate) fn assess_bytes(bytes: &[u8]) -> Result<Assessment> {
    if bytes.len() > MAX_BYTES {
        bail!("join receipt byte budget exceeded");
    }
    let r: Receipt = serde_json::from_slice(bytes)?;
    if r.schema != 1
        || r.producer != "rustc-thread-pool-join"
        || r.scope != "native-join-continuations"
    {
        bail!("unsupported join protocol");
    }
    if r.capture_id.is_empty()
        || r.capture_id.len() > 128
        || !r
            .capture_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
        || r.observer == 0
    {
        bail!("invalid join identity");
    }
    if r.full_pipeline_complete
        || r.cpu_work_ns.is_some()
        || r.execution_span_ns.is_some()
        || r.scope_coverage
        || r.scheduler_coverage
        || r.pre_attach_coverage
    {
        bail!("unsupported join coverage or work/span claim");
    }
    if r.task_allocator_baseline != 1
        || r.task_allocator_high_water == 0
        || r.expected_tasks != r.task_allocator_high_water - 1
        || r.expected_tasks > MAX_TASKS
        || r.events.len() > MAX_EVENTS
    {
        bail!("invalid or oversized independent task inventory");
    }
    let mut tasks: BTreeMap<u64, Task> = BTreeMap::new();
    let mut groups: BTreeMap<u64, Group> = BTreeMap::new();
    let mut successors = BTreeMap::new();
    let mut adjacency: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
    let mut issues = Vec::new();
    let mut edges = 0;
    for event in r.events {
        match event {
            Event::Root { task, at_ns } => {
                define(&mut tasks, task, Definition::Root, at_ns)?;
            }
            Event::Fork {
                parent,
                child,
                continuation,
                at_ns,
            } => {
                if parent == child || parent == continuation || child == continuation {
                    bail!("aliased fork endpoints");
                }
                define(&mut tasks, child, Definition::Child, at_ns)?;
                tasks.entry(parent).or_default();
                let group = groups.entry(continuation).or_default();
                if let Some(old) = group.parent {
                    if old != parent {
                        bail!("continuation shared by unrelated forks");
                    }
                } else {
                    group.parent = Some(parent);
                    define(
                        &mut tasks,
                        continuation,
                        Definition::Continuation(parent),
                        at_ns,
                    )?;
                    if successors.insert(parent, continuation).is_some() {
                        bail!("segment forked twice");
                    }
                }
                if !group.branches.insert(child) {
                    bail!("duplicate fork branch");
                }
                adjacency.entry(parent).or_default().insert(child);
                edges += 1;
            }
            Event::Start { task, at_ns } => {
                if tasks
                    .entry(task)
                    .or_default()
                    .start
                    .replace(at_ns)
                    .is_some()
                {
                    bail!("duplicate task start");
                }
            }
            Event::Finish {
                task,
                outcome,
                at_ns,
            } => {
                let _ = outcome;
                if tasks
                    .entry(task)
                    .or_default()
                    .finish
                    .replace(at_ns)
                    .is_some()
                {
                    bail!("duplicate task terminal");
                }
            }
            Event::Join {
                branch,
                child,
                continuation,
                at_ns,
            } => {
                if branch == continuation || child == continuation {
                    bail!("aliased join endpoints");
                }
                tasks.entry(branch).or_default();
                tasks.entry(child).or_default();
                tasks.entry(continuation).or_default();
                if groups
                    .entry(continuation)
                    .or_default()
                    .joins
                    .insert(branch, (child, at_ns))
                    .is_some()
                {
                    bail!("duplicate branch join");
                }
                adjacency.entry(child).or_default().insert(continuation);
                edges += 1;
            }
            Event::Resume {
                continuation,
                at_ns,
            } => {
                if tasks
                    .entry(continuation)
                    .or_default()
                    .resume
                    .replace(at_ns)
                    .is_some()
                {
                    bail!("duplicate continuation resume");
                }
            }
        }
    }
    if tasks
        .keys()
        .any(|id| *id == 0 || *id >= r.task_allocator_high_water)
    {
        bail!("foreign task endpoint");
    }
    if tasks.len() as u64 != r.expected_tasks {
        issues.push("independent task inventory has missing tasks".into());
    }
    let mut indegrees: BTreeMap<u64, usize> = tasks.keys().map(|id| (*id, 0)).collect();
    for children in adjacency.values() {
        for child in children {
            *indegrees
                .get_mut(child)
                .expect("all event endpoints are inventoried") += 1;
        }
    }
    let mut ready: VecDeque<u64> = indegrees
        .iter()
        .filter_map(|(id, n)| (*n == 0).then_some(*id))
        .collect();
    let mut visited = 0;
    while let Some(parent) = ready.pop_front() {
        visited += 1;
        if let Some(children) = adjacency.get(&parent) {
            for child in children {
                let n = indegrees
                    .get_mut(child)
                    .expect("all event endpoints are inventoried");
                *n -= 1;
                if *n == 0 {
                    ready.push_back(*child);
                }
            }
        }
    }
    if visited != tasks.len() {
        bail!("cyclic causal task graph");
    }
    for (&id, task) in &tasks {
        if task.definition.is_none() {
            issues.push(format!("task {id} has no definition"));
        }
        if let (Some(defined), Some(start)) = (task.defined_at, task.start)
            && defined > start
        {
            bail!("task started before definition");
        }
        match (task.start, task.finish) {
            (Some(start), Some(finish)) if start <= finish => {}
            (Some(_), Some(_)) => bail!("task terminal precedes start"),
            _ => issues.push(format!("task {id} lifecycle incomplete")),
        }
        match task.definition {
            Some(Definition::Continuation(_)) => match (task.resume, task.start) {
                (Some(resume), Some(start)) if resume <= start => {}
                (Some(_), Some(_)) => bail!("continuation started before resume"),
                _ => issues.push(format!("continuation {id} resume incomplete")),
            },
            _ if task.resume.is_some() => bail!("non-continuation resumed"),
            _ => {}
        }
    }
    let origins = lineage(&tasks, &successors, false)?;
    let terminals = lineage(&tasks, &successors, true)?;
    for (&continuation, group) in &groups {
        if group.parent.is_none() || group.branches.len() != 2 || group.joins.len() != 2 {
            issues.push(format!("join {continuation} obligations incomplete"));
        }
        for (&branch, &(terminal, notified)) in &group.joins {
            if !group.branches.contains(&branch) {
                bail!("join discharged unrelated branch");
            }
            if origins[&terminal].is_some_and(|origin| origin != branch) {
                bail!("join terminal belongs to another branch");
            }
            if Some(terminal) != terminals[&branch] {
                bail!("join bypassed nested continuation");
            }
            if let (Some(finish), Some(resume)) = (
                tasks[&terminal].finish,
                tasks.get(&continuation).and_then(|t| t.resume),
            ) && (finish > notified || notified > resume)
            {
                bail!("join precedes child completion or follows resume");
            }
        }
        if let Some(parent) = group.parent {
            for child in &group.branches {
                if let (Some(start), Some(finish), Some(fork)) = (
                    tasks[&parent].start,
                    tasks[&parent].finish,
                    tasks[child].defined_at,
                ) && (start > fork || fork > finish)
                {
                    bail!("fork outside parent segment");
                }
                if let (Some(finish), Some(start)) = (tasks[&parent].finish, tasks[child].start)
                    && finish > start
                {
                    bail!("child started before parent fork segment ended");
                }
            }
        }
    }
    if r.dropped_events != 0 {
        issues.push(format!("{} join events lost", r.dropped_events));
    }
    if !r.sealed || r.interface_outcome != Outcome::Returned {
        issues.push("join owner is not sealed after normal interface return".into());
    }
    Ok(Assessment {
        schema: 1,
        producer: r.producer,
        scope: r.scope,
        capture_id: r.capture_id,
        structural_complete: issues.is_empty(),
        full_pipeline_complete: false,
        nodes: tasks.len(),
        edges,
        issues,
        opaque_boundaries: vec![
            "scope-tasks".into(),
            "scheduler-and-resource-grants".into(),
            "pre-attach-work".into(),
            "query-to-task-binding".into(),
            "cross-capture-clock-binding".into(),
            "cargo-and-process-binding".into(),
        ],
        cpu_work_ns: None,
        execution_span_ns: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    fn fixture() -> Value {
        json!({"schema":1,"producer":"rustc-thread-pool-join","scope":"native-join-continuations","capture_id":"pool-test","observer":1,"full_pipeline_complete":false,"cpu_work_ns":null,"execution_span_ns":null,"scope_coverage":false,"scheduler_coverage":false,"pre_attach_coverage":false,"sealed":true,"interface_outcome":"returned","task_allocator_baseline":1,"task_allocator_high_water":5,"expected_tasks":4,"dropped_events":0,"events":[
            {"kind":"root","task":1,"at_ns":0},{"kind":"start","task":1,"at_ns":1},
            {"kind":"fork","parent":1,"child":2,"continuation":4,"at_ns":2},{"kind":"fork","parent":1,"child":3,"continuation":4,"at_ns":2},{"kind":"finish","task":1,"outcome":"normal","at_ns":3},
            {"kind":"start","task":2,"at_ns":4},{"kind":"finish","task":2,"outcome":"normal","at_ns":5},{"kind":"start","task":3,"at_ns":4},{"kind":"finish","task":3,"outcome":"panic","at_ns":6},
            {"kind":"join","branch":2,"child":2,"continuation":4,"at_ns":7},{"kind":"join","branch":3,"child":3,"continuation":4,"at_ns":7},{"kind":"resume","continuation":4,"at_ns":8},{"kind":"start","task":4,"at_ns":9},{"kind":"finish","task":4,"outcome":"panic","at_ns":10}]})
    }
    fn assess(v: &Value) -> Result<Assessment> {
        assess_bytes(&serde_json::to_vec(v).unwrap())
    }
    #[test]
    fn valid_parallel_and_panic_branch_has_no_sibling_edge() {
        let a = assess(&fixture()).unwrap();
        assert!(a.structural_complete);
        assert_eq!(a.nodes, 4);
        assert_eq!(a.edges, 4);
        assert!(!a.full_pipeline_complete);
    }
    #[test]
    fn independent_missing_whole_task_is_incomplete() {
        let mut v = fixture();
        v["expected_tasks"] = json!(5);
        v["task_allocator_high_water"] = json!(6);
        assert!(!assess(&v).unwrap().structural_complete);
    }
    #[test]
    fn loss_and_unsealed_snapshots_are_incomplete() {
        for key in ["sealed", "dropped_events"] {
            let mut v = fixture();
            v[key] = if key == "sealed" {
                json!(false)
            } else {
                json!(1)
            };
            assert!(!assess(&v).unwrap().structural_complete);
        }
    }
    #[test]
    fn rejects_wrong_branch_and_foreign_endpoint() {
        let mut v = fixture();
        v["events"][9]["branch"] = json!(3);
        assert!(assess(&v).is_err());
        let mut v = fixture();
        v["events"][9]["child"] = json!(5);
        assert!(assess(&v).is_err());
    }
    #[test]
    fn rejects_duplicate_terminals_and_unknown_fields() {
        let mut v = fixture();
        let terminal = v["events"][6].clone();
        v["events"].as_array_mut().unwrap().push(terminal);
        assert!(assess(&v).is_err());
        let mut v = fixture();
        v["invented"] = json!(true);
        assert!(assess(&v).is_err());
    }
    #[test]
    fn rejects_false_coverage_and_missing_nullable_fields() {
        for key in [
            "full_pipeline_complete",
            "scope_coverage",
            "scheduler_coverage",
            "pre_attach_coverage",
        ] {
            let mut v = fixture();
            v[key] = json!(true);
            assert!(assess(&v).is_err());
        }
        let mut v = fixture();
        v.as_object_mut().unwrap().remove("cpu_work_ns");
        assert!(assess(&v).is_err());
    }
    #[test]
    fn rejects_time_inversion() {
        let mut v = fixture();
        v["events"][6]["at_ns"] = json!(99);
        assert!(assess(&v).is_err());
    }

    #[test]
    fn nested_join_must_discharge_the_terminal_continuation() {
        let mut v = fixture();
        v["task_allocator_high_water"] = json!(8);
        v["expected_tasks"] = json!(7);
        v["events"] = json!([
            {"kind":"root","task":1,"at_ns":0},
            {"kind":"start","task":1,"at_ns":1},
            {"kind":"fork","parent":1,"child":2,"continuation":4,"at_ns":2},
            {"kind":"fork","parent":1,"child":3,"continuation":4,"at_ns":2},
            {"kind":"finish","task":1,"outcome":"normal","at_ns":3},
            {"kind":"start","task":2,"at_ns":4},
            {"kind":"start","task":3,"at_ns":4},
            {"kind":"fork","parent":2,"child":5,"continuation":7,"at_ns":5},
            {"kind":"fork","parent":2,"child":6,"continuation":7,"at_ns":5},
            {"kind":"finish","task":2,"outcome":"normal","at_ns":6},
            {"kind":"finish","task":3,"outcome":"normal","at_ns":6},
            {"kind":"start","task":5,"at_ns":7},
            {"kind":"start","task":6,"at_ns":7},
            {"kind":"finish","task":5,"outcome":"normal","at_ns":8},
            {"kind":"finish","task":6,"outcome":"normal","at_ns":9},
            {"kind":"join","branch":5,"child":5,"continuation":7,"at_ns":10},
            {"kind":"join","branch":6,"child":6,"continuation":7,"at_ns":11},
            {"kind":"resume","continuation":7,"at_ns":12},
            {"kind":"start","task":7,"at_ns":13},
            {"kind":"finish","task":7,"outcome":"normal","at_ns":14},
            {"kind":"join","branch":2,"child":7,"continuation":4,"at_ns":15},
            {"kind":"join","branch":3,"child":3,"continuation":4,"at_ns":15},
            {"kind":"resume","continuation":4,"at_ns":16},
            {"kind":"start","task":4,"at_ns":17},
            {"kind":"finish","task":4,"outcome":"normal","at_ns":18}
        ]);
        let a = assess(&v).unwrap();
        assert!(a.structural_complete);
        assert_eq!((a.nodes, a.edges), (7, 8));
        v["events"][20]["child"] = json!(2);
        assert!(assess(&v).is_err());
    }

    #[test]
    fn long_lineage_is_memoized_and_cycles_are_rejected() {
        let mut tasks = BTreeMap::new();
        let mut successors = BTreeMap::new();
        tasks.insert(
            1,
            Task {
                definition: Some(Definition::Root),
                ..Task::default()
            },
        );
        for id in 2..=30_000 {
            tasks.insert(
                id,
                Task {
                    definition: Some(Definition::Continuation(id - 1)),
                    ..Task::default()
                },
            );
            successors.insert(id - 1, id);
        }
        let origins = lineage(&tasks, &successors, false).unwrap();
        let terminals = lineage(&tasks, &successors, true).unwrap();
        assert_eq!(origins[&30_000], Some(1));
        assert_eq!(terminals[&1], Some(30_000));
        successors.insert(30_000, 1);
        assert!(lineage(&tasks, &successors, true).is_err());
    }

    #[test]
    fn a_missing_definition_is_incomplete_not_a_panic() {
        let mut v = fixture();
        v["events"].as_array_mut().unwrap().remove(0);
        v["dropped_events"] = json!(1);
        assert!(!assess(&v).unwrap().structural_complete);
    }
}
