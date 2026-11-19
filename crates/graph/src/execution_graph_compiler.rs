//! Coverage of a selected native Cargo compiler-launch authority, not a full pipeline.
//!
//! Pure, bounded validation. The caller must independently validate the scheduler
//! receipt and the original schema-3 query bytes before constructing summaries.
//! Neither a query interface outcome nor an entry implies a process exit outcome.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use std::collections::{BTreeMap, BTreeSet};

pub const PROFILE: &str = "buildrunner-compiler-launch-v1";
pub const SCOPE: &str = "buildrunner-compiler-launch-coverage";
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_SMALL_BYTES: usize = 16 * 1024;
pub const MAX_RECORDS: usize = 100_000;
pub const MAX_JOURNALS: usize = 1_000_000;
pub const MAX_ID_BYTES: usize = 128;

// A custom deserializer makes Option fields mandatory even when their value is null.
fn nullable<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pending,
    Success,
    Failure,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum UnitKind {
    Rustc,
    Rustdoc,
    BuildScript,
    DocTest,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Pending,
    Fresh,
    Cutoff,
    Required,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UnitOutcome {
    Pending,
    Success,
    Failure,
    NotStarted,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LaunchOutcome {
    Pending,
    Success,
    Failure,
    SpawnFailed,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    pub unit_id: u64,
    pub kind: UnitKind,
    pub disposition: Disposition,
    pub outcome: UnitOutcome,
    pub terminal: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub invocation_id: String,
    pub launch_ordinal: u64,
    pub unit_id: u64,
    pub parent_task_id: String,
    pub entry_capture_id: String,
    pub capture_id: String,
    pub spawn_attempts: u64,
    pub spawned: bool,
    pub terminal: bool,
    pub outcome: LaunchOutcome,
    #[serde(deserialize_with = "nullable")]
    pub wait_status: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub profile: String,
    pub run_id: String,
    pub authority_root: String,
    pub parent_authority_root: String,
    pub scheduler_capture_id: String,
    pub expected_units: u64,
    pub expected_launches: u64,
    pub expected_journals: u64,
    pub sealed: bool,
    pub interrupted: bool,
    pub lost_events: u64,
    pub outcome: Outcome,
    pub units: Vec<Unit>,
    pub launches: Vec<Launch>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Event {
    UnitDeclared,
    UnitResolution,
    UnitTerminal,
    LaunchDeclared,
    SpawnAttempt,
    SpawnResult,
    WaitResult,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub schema: u32,
    pub profile: String,
    pub authority_root: String,
    pub sequence: u64,
    pub event: Event,
    #[serde(deserialize_with = "nullable")]
    pub unit: Option<Unit>,
    #[serde(deserialize_with = "nullable")]
    pub launch: Option<Launch>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub schema: u32,
    pub producer: String,
    pub scope: String,
    pub capture_id: String,
    pub run_id: String,
    pub invocation_id: String,
    pub parent_invocation_id: String,
    pub parent_task_id: String,
    pub expected_query_captures: u64,
    pub query_capture_ids: Vec<String>,
    pub query_receipt_schema: u32,
    pub full_pipeline_complete: bool,
    #[serde(deserialize_with = "nullable")]
    pub cpu_work_ns: Option<u64>,
    #[serde(deserialize_with = "nullable")]
    pub execution_span_ns: Option<u64>,
}

/// Trusted projection of an independently strictly validated Cargo scheduler receipt.
/// `authority_root` is the scheduler's parent owner, not its capture id or a JobId.
#[derive(Clone, Debug)]
pub struct SchedulerSummary {
    pub run_id: String,
    pub authority_root: String,
    pub capture_id: String,
    pub unit_ids: BTreeSet<u64>,
    pub structural_complete: bool,
}

/// Do not deserialize raw query JSON into this type to bypass query validation.
/// Construct it only after assessing the original bytes with the query consumer.
#[derive(Clone, Debug)]
pub struct QuerySummary {
    pub schema: u32,
    pub capture_id: String,
    pub structural_complete: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CoverageAssessment {
    pub scope: &'static str,
    pub run_id: String,
    pub authority_root: String,
    pub expected_units: u64,
    pub observed_units: usize,
    pub expected_launches: u64,
    pub observed_launches: usize,
    pub expected_journals: u64,
    pub observed_journals: usize,
    pub observed_entries: usize,
    pub observed_queries: usize,
    pub structural_complete: bool,
    pub issues: Vec<String>,
    pub outcome: Outcome,
    pub full_pipeline_complete: bool,
    pub cpu_work_ns: Option<u64>,
    pub execution_span_ns: Option<u64>,
}

fn parse<T: DeserializeOwned>(bytes: &[u8], limit: usize, label: &str) -> Result<T> {
    ensure!(bytes.len() <= limit, "{label} exceeds byte limit {limit}");
    // Typed serde structs reject unknown and duplicate fields, including nested
    // records. Parsing through serde_json::Value would erase duplicate keys.
    serde_json::from_slice(bytes).with_context(|| format!("invalid {label}"))
}

pub fn parse_manifest(bytes: &[u8]) -> Result<Manifest> {
    let value: Manifest = parse(bytes, MAX_MANIFEST_BYTES, "compiler manifest")?;
    validate_manifest(&value)?;
    Ok(value)
}

pub fn parse_journal(bytes: &[u8]) -> Result<Journal> {
    let value: Journal = parse(bytes, MAX_SMALL_BYTES, "compiler journal")?;
    validate_journal(&value)?;
    Ok(value)
}

pub fn parse_entry(bytes: &[u8]) -> Result<Entry> {
    let value: Entry = parse(bytes, MAX_SMALL_BYTES, "compiler entry")?;
    validate_entry(&value)?;
    Ok(value)
}

fn id(value: &str, label: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= MAX_ID_BYTES
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "invalid anonymous {label}: expected 1..={MAX_ID_BYTES} ASCII identifier bytes"
    );
    Ok(())
}

fn header(schema: u32, profile: &str) -> Result<()> {
    ensure!(schema == 1, "compiler registry requires schema 1");
    ensure!(profile == PROFILE, "unexpected compiler registry profile");
    Ok(())
}

fn validate_unit(unit: &Unit) -> Result<()> {
    ensure!(
        unit.terminal == (unit.outcome != UnitOutcome::Pending),
        "unit {} terminal/outcome contradiction",
        unit.unit_id
    );
    ensure!(
        unit.disposition != Disposition::Required || unit.kind == UnitKind::Rustc,
        "only rustc units have supported required compiler coverage"
    );
    if unit.disposition == Disposition::Pending && unit.terminal {
        ensure!(
            unit.outcome == UnitOutcome::NotStarted,
            "unresolved terminal unit must be not_started"
        );
    }
    Ok(())
}

fn validate_launch(launch: &Launch) -> Result<()> {
    id(&launch.invocation_id, "invocation id")?;
    id(&launch.parent_task_id, "parent task id")?;
    id(&launch.entry_capture_id, "entry capture id")?;
    id(&launch.capture_id, "query capture id")?;
    ensure!(
        launch.parent_task_id == format!("cargo-unit-{}", launch.unit_id),
        "launch task is not its scheduler unit"
    );
    ensure!(
        launch.spawn_attempts <= MAX_JOURNALS as u64,
        "spawn attempt count exceeds bound"
    );
    ensure!(
        launch.terminal == (launch.outcome != LaunchOutcome::Pending),
        "launch terminal/outcome contradiction"
    );
    ensure!(
        launch.wait_status.is_none_or(|raw| raw >= 0),
        "negative raw wait status"
    );
    if launch.spawned {
        ensure!(launch.spawn_attempts > 0, "spawned launch has no attempt");
        if launch.terminal {
            let raw = launch
                .wait_status
                .context("terminal spawned launch missing raw wait status")?;
            ensure!(
                launch.outcome
                    == if raw == 0 {
                        LaunchOutcome::Success
                    } else {
                        LaunchOutcome::Failure
                    },
                "launch outcome disagrees with raw wait status"
            );
        } else {
            ensure!(
                launch.wait_status.is_none(),
                "nonterminal launch has wait status"
            );
        }
    } else {
        ensure!(
            launch.wait_status.is_none(),
            "unspawned launch has wait status"
        );
        if launch.terminal {
            ensure!(
                launch.outcome == LaunchOutcome::SpawnFailed && launch.spawn_attempts > 0,
                "unspawned terminal launch needs observed spawn failure"
            );
        }
    }
    Ok(())
}

fn validate_manifest(manifest: &Manifest) -> Result<()> {
    header(manifest.schema, &manifest.profile)?;
    id(&manifest.run_id, "run id")?;
    id(&manifest.authority_root, "compiler authority")?;
    id(&manifest.parent_authority_root, "parent authority")?;
    id(&manifest.scheduler_capture_id, "scheduler capture")?;
    ensure!(
        manifest.authority_root != manifest.parent_authority_root,
        "compiler authority cannot be its own parent"
    );
    ensure!(
        manifest.expected_units <= MAX_RECORDS as u64 && manifest.units.len() <= MAX_RECORDS,
        "unit count exceeds bound"
    );
    ensure!(
        manifest.expected_launches <= MAX_RECORDS as u64 && manifest.launches.len() <= MAX_RECORDS,
        "launch count exceeds bound"
    );
    ensure!(
        manifest.expected_journals <= MAX_JOURNALS as u64,
        "journal count exceeds bound"
    );
    let mut units = BTreeSet::new();
    for unit in &manifest.units {
        validate_unit(unit)?;
        ensure!(
            units.insert(unit.unit_id),
            "duplicate unit identity {}",
            unit.unit_id
        );
    }
    let mut ordinals = BTreeSet::new();
    let mut identities = BTreeSet::new();
    for launch in &manifest.launches {
        validate_launch(launch)?;
        ensure!(
            ordinals.insert(launch.launch_ordinal),
            "duplicate launch ordinal"
        );
        for identity in [
            &launch.invocation_id,
            &launch.entry_capture_id,
            &launch.capture_id,
        ] {
            ensure!(
                identity != &manifest.authority_root && identities.insert(identity),
                "duplicate compiler child identity"
            );
        }
    }
    Ok(())
}

fn validate_journal(journal: &Journal) -> Result<()> {
    header(journal.schema, &journal.profile)?;
    id(&journal.authority_root, "journal authority")?;
    ensure!(
        journal.sequence > 0 && journal.sequence <= MAX_JOURNALS as u64,
        "journal sequence out of range"
    );
    match journal.event {
        Event::UnitDeclared | Event::UnitResolution | Event::UnitTerminal => {
            ensure!(journal.launch.is_none(), "unit journal contains launch");
            validate_unit(journal.unit.as_ref().context("unit journal missing unit")?)?;
        }
        Event::LaunchDeclared | Event::SpawnAttempt | Event::SpawnResult | Event::WaitResult => {
            ensure!(journal.unit.is_none(), "launch journal contains unit");
            validate_launch(
                journal
                    .launch
                    .as_ref()
                    .context("launch journal missing launch")?,
            )?;
        }
    }
    Ok(())
}

fn validate_entry(entry: &Entry) -> Result<()> {
    ensure!(
        entry.schema == 1 && entry.producer == "rustc-query-admission-entry",
        "unexpected compiler entry producer/schema"
    );
    ensure!(
        entry.scope == "compiler-entry-to-query-capture-binding",
        "unexpected compiler entry scope"
    );
    for identity in [
        &entry.capture_id,
        &entry.run_id,
        &entry.invocation_id,
        &entry.parent_invocation_id,
        &entry.parent_task_id,
    ] {
        id(identity, "entry binding")?;
    }
    ensure!(
        entry.expected_query_captures == 1 && entry.query_capture_ids.len() == 1,
        "entry must independently expect exactly one query capture"
    );
    id(&entry.query_capture_ids[0], "entry query capture")?;
    ensure!(
        matches!(entry.query_receipt_schema, 3 | 4),
        "entry requires a supported final query receipt schema (3 or 4)"
    );
    ensure!(
        !entry.full_pipeline_complete
            && entry.cpu_work_ns.is_none()
            && entry.execution_span_ns.is_none(),
        "entry cannot claim full pipeline/timing coverage"
    );
    Ok(())
}

fn same_launch_identity(old: &Launch, next: &Launch) -> bool {
    old.invocation_id == next.invocation_id
        && old.launch_ordinal == next.launch_ordinal
        && old.unit_id == next.unit_id
        && old.parent_task_id == next.parent_task_id
        && old.entry_capture_id == next.entry_capture_id
        && old.capture_id == next.capture_id
}

// Replay only a complete contiguous journal. A gap is incomplete evidence, not
// permission to infer transitions from a later snapshot or final root.
fn replay(manifest: &Manifest, journals: &BTreeMap<u64, &Journal>) -> Result<()> {
    let mut units: BTreeMap<u64, Unit> = BTreeMap::new();
    let mut launches: BTreeMap<u64, Launch> = BTreeMap::new();
    let mut attempt_open = BTreeSet::new();
    let mut child_ids = BTreeSet::new();
    let mut launched_units = BTreeSet::new();
    let mut open_launches: BTreeMap<u64, u64> = BTreeMap::new();
    for journal in journals.values() {
        match journal.event {
            Event::UnitDeclared => {
                let unit = journal
                    .unit
                    .as_ref()
                    .context("unit declaration missing snapshot")?;
                ensure!(!units.contains_key(&unit.unit_id), "redeclared unit");
                ensure!(
                    unit.disposition == Disposition::Pending
                        && unit.outcome == UnitOutcome::Pending
                        && !unit.terminal,
                    "unit declaration is not initial state"
                );
                units.insert(unit.unit_id, unit.clone());
            }
            Event::UnitResolution | Event::UnitTerminal => {
                let unit = journal
                    .unit
                    .as_ref()
                    .context("unit transition missing snapshot")?;
                let old = units
                    .get(&unit.unit_id)
                    .context("unit transition precedes declaration")?;
                ensure!(
                    old.kind == unit.kind && !old.terminal,
                    "unit kind changed or transition after terminal"
                );
                if journal.event == Event::UnitResolution {
                    ensure!(
                        unit.outcome == old.outcome && !unit.terminal,
                        "resolution changes unit outcome"
                    );
                    ensure!(
                        (old.disposition == Disposition::Pending
                            && unit.disposition != Disposition::Pending)
                            || (old.disposition == Disposition::Required
                                && unit.disposition == Disposition::Unsupported
                                && !launched_units.contains(&unit.unit_id)),
                        "invalid unit disposition transition"
                    );
                } else {
                    ensure!(
                        unit.disposition == old.disposition && unit.terminal,
                        "invalid unit terminal transition"
                    );
                    if unit.outcome == UnitOutcome::Success {
                        ensure!(
                            open_launches.get(&unit.unit_id).copied().unwrap_or(0) == 0,
                            "unit success precedes terminal parent wait/spawn result"
                        );
                    }
                }
                units.insert(unit.unit_id, unit.clone());
            }
            Event::LaunchDeclared => {
                let launch = journal
                    .launch
                    .as_ref()
                    .context("launch declaration missing snapshot")?;
                ensure!(
                    launch.launch_ordinal == launches.len() as u64,
                    "launch declarations must allocate contiguous ordinals from zero"
                );
                let unit = units
                    .get(&launch.unit_id)
                    .context("launch unit was not declared")?;
                ensure!(
                    unit.disposition == Disposition::Required && !unit.terminal,
                    "launch must belong to an active required unit"
                );
                ensure!(
                    launch.spawn_attempts == 0
                        && !launch.spawned
                        && !launch.terminal
                        && launch.outcome == LaunchOutcome::Pending
                        && launch.wait_status.is_none(),
                    "launch declaration is not initial state"
                );
                for identity in [
                    &launch.invocation_id,
                    &launch.entry_capture_id,
                    &launch.capture_id,
                ] {
                    ensure!(
                        identity != &manifest.authority_root && child_ids.insert(identity.clone()),
                        "duplicate journal child identity"
                    );
                }
                launched_units.insert(launch.unit_id);
                *open_launches.entry(launch.unit_id).or_default() += 1;
                launches.insert(launch.launch_ordinal, launch.clone());
            }
            Event::SpawnAttempt | Event::SpawnResult | Event::WaitResult => {
                let next = journal
                    .launch
                    .as_ref()
                    .context("launch transition missing snapshot")?;
                let old = launches
                    .get(&next.launch_ordinal)
                    .context("launch transition precedes declaration")?;
                ensure!(
                    same_launch_identity(old, next) && !old.terminal,
                    "launch identity changed or transition after terminal"
                );
                match journal.event {
                    Event::SpawnAttempt => {
                        ensure!(
                            !old.spawned && !attempt_open.contains(&next.launch_ordinal),
                            "overlapping attempt or attempt after spawn"
                        );
                        let mut expected = old.clone();
                        expected.spawn_attempts = expected
                            .spawn_attempts
                            .checked_add(1)
                            .context("attempt counter overflow")?;
                        ensure!(
                            &expected == next,
                            "spawn attempt must increment only attempt counter by one"
                        );
                        attempt_open.insert(next.launch_ordinal);
                    }
                    Event::SpawnResult => {
                        ensure!(
                            !old.spawned && attempt_open.remove(&next.launch_ordinal),
                            "spawn result without independent attempt"
                        );
                        let mut expected = old.clone();
                        expected.spawned = next.spawned;
                        if !next.spawned && next.terminal {
                            expected.terminal = true;
                            expected.outcome = LaunchOutcome::SpawnFailed;
                        }
                        ensure!(&expected == next, "invalid spawn result transition");
                    }
                    Event::WaitResult => {
                        ensure!(
                            old.spawned && !attempt_open.contains(&next.launch_ordinal),
                            "wait without observed spawn result"
                        );
                        let mut expected = old.clone();
                        expected.terminal = true;
                        expected.wait_status = next.wait_status;
                        expected.outcome = next.outcome;
                        ensure!(
                            &expected == next && next.terminal && next.wait_status.is_some(),
                            "invalid wait result transition"
                        );
                    }
                    _ => unreachable!(),
                }
                if next.terminal {
                    let remaining = open_launches
                        .get_mut(&next.unit_id)
                        .context("terminal launch lacks active unit counter")?;
                    *remaining = remaining
                        .checked_sub(1)
                        .context("active launch counter underflow")?;
                }
                launches.insert(next.launch_ordinal, next.clone());
            }
        }
    }
    let final_units: BTreeMap<_, _> = manifest
        .units
        .iter()
        .map(|u| (u.unit_id, u.clone()))
        .collect();
    let final_launches: BTreeMap<_, _> = manifest
        .launches
        .iter()
        .map(|l| (l.launch_ordinal, l.clone()))
        .collect();
    ensure!(
        units == final_units,
        "journal unit replay differs from final root"
    );
    ensure!(
        launches == final_launches,
        "journal launch replay differs from final root"
    );
    ensure!(
        units.len() as u64 == manifest.expected_units
            && launches.len() as u64 == manifest.expected_launches,
        "journal independent declaration counters differ from final root"
    );
    Ok(())
}

/// Reconcile one selected authority. Missing evidence yields incomplete coverage.
/// Contradictory identities, malformed snapshots and impossible lifecycles fail.
/// The caller must require both initial declaration and final manifest on disk:
/// neither can be synthesized from journal snapshots or child receipts.
pub fn reconcile(
    manifest: &Manifest,
    initial: &Manifest,
    journals: &[Journal],
    scheduler: &SchedulerSummary,
    entries: &[Entry],
    queries: &[QuerySummary],
) -> Result<CoverageAssessment> {
    validate_manifest(manifest)?;
    validate_manifest(initial)?;
    ensure!(
        journals.len() <= MAX_JOURNALS
            && entries.len() <= MAX_RECORDS
            && queries.len() <= MAX_RECORDS,
        "child input count exceeds bound"
    );
    ensure!(
        initial.run_id == manifest.run_id
            && initial.authority_root == manifest.authority_root
            && initial.parent_authority_root == manifest.parent_authority_root
            && initial.scheduler_capture_id == manifest.scheduler_capture_id,
        "initial declaration identity differs from final root"
    );
    ensure!(
        initial.expected_units == 0
            && initial.expected_launches == 0
            && initial.expected_journals == 0
            && initial.units.is_empty()
            && initial.launches.is_empty()
            && !initial.sealed
            && !initial.interrupted
            && initial.lost_events == 0
            && initial.outcome == Outcome::Pending,
        "initial declaration must be empty, pending, unsealed and lossless"
    );
    id(&scheduler.run_id, "scheduler run")?;
    id(&scheduler.authority_root, "scheduler owner")?;
    id(&scheduler.capture_id, "scheduler capture")?;
    ensure!(
        scheduler.unit_ids.len() <= MAX_RECORDS,
        "scheduler unit count exceeds bound"
    );
    let mut issues = Vec::new();
    if !manifest.sealed {
        issues.push("missing final compiler seal".into());
    }
    if manifest.interrupted {
        issues.push("compiler authority interrupted".into());
    }
    if manifest.lost_events != 0 {
        issues.push("compiler authority lost events".into());
    }
    if manifest.outcome == Outcome::Pending {
        issues.push("compiler authority outcome remains pending".into());
    }
    if manifest.sealed {
        ensure!(
            manifest.outcome != Outcome::Pending,
            "sealed root has pending outcome"
        );
    }
    if manifest.units.len() as u64 != manifest.expected_units {
        issues.push("independent expected_units differs from unit records".into());
    }
    if manifest.launches.len() as u64 != manifest.expected_launches {
        issues.push("independent expected_launches differs from launch records".into());
    }
    let ordinals: BTreeSet<_> = manifest.launches.iter().map(|l| l.launch_ordinal).collect();
    if ordinals.iter().copied().ne(0..manifest.expected_launches) {
        issues.push("launch ordinals do not cover independent expected range".into());
    }
    if !scheduler.structural_complete {
        issues.push("scheduler receipt is incomplete".into());
    }
    if scheduler.run_id != manifest.run_id
        || scheduler.authority_root != manifest.parent_authority_root
        || scheduler.capture_id != manifest.scheduler_capture_id
    {
        issues
            .push("compiler root is not bound to the validated scheduler owner/run/capture".into());
    }
    let unit_ids: BTreeSet<_> = manifest.units.iter().map(|u| u.unit_id).collect();
    if unit_ids != scheduler.unit_ids {
        issues.push("compiler units differ from exact scheduler receipt unit map".into());
    }

    let mut journal_map = BTreeMap::new();
    for journal in journals {
        validate_journal(journal)?;
        ensure!(
            journal.authority_root == manifest.authority_root,
            "foreign compiler journal"
        );
        ensure!(
            journal_map.insert(journal.sequence, journal).is_none(),
            "duplicate compiler journal sequence"
        );
    }
    let contiguous = journal_map.len() as u64 == manifest.expected_journals
        && journal_map
            .keys()
            .copied()
            .eq(1..=manifest.expected_journals);
    if contiguous {
        replay(manifest, &journal_map)?;
    } else {
        issues
            .push("compiler journal count/range differs from independent expected_journals".into());
    }

    let units: BTreeMap<_, _> = manifest.units.iter().map(|u| (u.unit_id, u)).collect();
    let mut unit_launches: BTreeMap<u64, Vec<&Launch>> = BTreeMap::new();
    for launch in &manifest.launches {
        unit_launches
            .entry(launch.unit_id)
            .or_default()
            .push(launch);
    }
    for unit in &manifest.units {
        if !unit.terminal {
            issues.push(format!("unit {} is nonterminal", unit.unit_id));
        }
        if unit.disposition == Disposition::Unsupported {
            issues.push(format!(
                "unit {} is unsupported: no compiler coverage claimed",
                unit.unit_id
            ));
        }
        let launches = unit_launches
            .get(&unit.unit_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if unit.disposition != Disposition::Required || unit.outcome == UnitOutcome::NotStarted {
            ensure!(
                launches.is_empty(),
                "non-required/not_started unit has compiler launch"
            );
        }
        if unit.disposition == Disposition::Required
            && unit.outcome == UnitOutcome::Success
            && launches.is_empty()
        {
            issues.push(format!(
                "successful required unit {} missing launch declaration",
                unit.unit_id
            ));
        }
        if unit.outcome == UnitOutcome::Success {
            ensure!(
                launches.iter().all(|l| !matches!(
                    l.outcome,
                    LaunchOutcome::Failure | LaunchOutcome::SpawnFailed
                )),
                "unit success incompatible with failed compiler launch"
            );
        }
    }

    let mut entry_map = BTreeMap::new();
    let mut entry_invocations = BTreeSet::new();
    let mut entry_queries = BTreeSet::new();
    for entry in entries {
        validate_entry(entry)?;
        ensure!(
            entry_map.insert(entry.capture_id.as_str(), entry).is_none()
                && entry_invocations.insert(entry.invocation_id.as_str())
                && entry_queries.insert(entry.query_capture_ids[0].as_str()),
            "duplicate compiler entry identity/binding"
        );
    }
    let mut query_map = BTreeMap::new();
    for query in queries {
        id(&query.capture_id, "query summary capture")?;
        ensure!(
            query_map.insert(query.capture_id.as_str(), query).is_none(),
            "duplicate final query capture identity"
        );
    }
    let mut bound_entries = BTreeSet::new();
    let mut bound_queries = BTreeSet::new();
    for launch in &manifest.launches {
        let unit = units
            .get(&launch.unit_id)
            .context("launch references unknown scheduler unit")?;
        ensure!(
            unit.disposition == Disposition::Required,
            "compiler launch for non-required unit"
        );
        if !launch.terminal {
            issues.push(format!("launch {} is nonterminal", launch.launch_ordinal));
        }
        if launch.spawned {
            match entry_map.get(launch.entry_capture_id.as_str()) {
                Some(entry)
                    if entry.run_id == manifest.run_id
                        && entry.invocation_id == launch.invocation_id
                        && entry.parent_invocation_id == manifest.authority_root
                        && entry.parent_task_id == launch.parent_task_id
                        && entry.query_capture_ids[0] == launch.capture_id =>
                {
                    bound_entries.insert(entry.capture_id.as_str());
                }
                Some(_) => issues.push(format!(
                    "launch {} entry binding mismatch",
                    launch.launch_ordinal
                )),
                None => issues.push(format!(
                    "launch {} missing independent compiler entry",
                    launch.launch_ordinal
                )),
            }
            match query_map.get(launch.capture_id.as_str()) {
                Some(query)
                    if entry_map
                        .get(launch.entry_capture_id.as_str())
                        .is_some_and(|entry| query.schema == entry.query_receipt_schema)
                        && query.structural_complete
                        && bound_entries.contains(launch.entry_capture_id.as_str()) =>
                {
                    bound_queries.insert(query.capture_id.as_str());
                }
                Some(_) => issues.push(format!(
                    "launch {} final query requires structural completeness and the exact schema declared by its bound entry",
                    launch.launch_ordinal
                )),
                None => issues.push(format!(
                    "launch {} missing final query receipt",
                    launch.launch_ordinal
                )),
            }
        } else {
            ensure!(
                !entry_map.contains_key(launch.entry_capture_id.as_str())
                    && !query_map.contains_key(launch.capture_id.as_str()),
                "unspawned launch has compiler child evidence"
            );
        }
    }
    if bound_entries.len() != entries.len() {
        issues.push("foreign or unbound compiler entries cannot count".into());
    }
    if bound_queries.len() != queries.len() {
        issues.push("foreign, incomplete or unbound query receipts cannot count".into());
    }
    if manifest.outcome == Outcome::Success {
        ensure!(
            manifest.units.iter().all(|u| !matches!(
                u.outcome,
                UnitOutcome::Failure | UnitOutcome::Cancelled | UnitOutcome::NotStarted
            )),
            "successful root contains failed/non-started/cancelled unit"
        );
        ensure!(
            manifest.launches.iter().all(|l| !matches!(
                l.outcome,
                LaunchOutcome::Failure | LaunchOutcome::SpawnFailed
            )),
            "successful root contains failed launch"
        );
    }
    Ok(CoverageAssessment {
        scope: SCOPE,
        run_id: manifest.run_id.clone(),
        authority_root: manifest.authority_root.clone(),
        expected_units: manifest.expected_units,
        observed_units: manifest.units.len(),
        expected_launches: manifest.expected_launches,
        observed_launches: manifest.launches.len(),
        expected_journals: manifest.expected_journals,
        observed_journals: journals.len(),
        observed_entries: bound_entries.len(),
        observed_queries: bound_queries.len(),
        structural_complete: issues.is_empty(),
        issues,
        outcome: manifest.outcome,
        full_pipeline_complete: false,
        cpu_work_ns: None,
        execution_span_ns: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: Manifest,
        initial: Manifest,
        journals: Vec<Journal>,
        scheduler: SchedulerSummary,
        entries: Vec<Entry>,
        queries: Vec<QuerySummary>,
    }

    impl Fixture {
        fn assess(&self) -> Result<CoverageAssessment> {
            reconcile(
                &self.root,
                &self.initial,
                &self.journals,
                &self.scheduler,
                &self.entries,
                &self.queries,
            )
        }

        fn push(&mut self, event: Event, unit: Option<Unit>, launch: Option<Launch>) {
            self.journals.push(Journal {
                schema: 1,
                profile: PROFILE.into(),
                authority_root: self.root.authority_root.clone(),
                sequence: self.journals.len() as u64 + 1,
                event,
                unit,
                launch,
            });
            self.root.expected_journals = self.journals.len() as u64;
        }

        fn rejected_or_incomplete(&self) {
            assert!(match self.assess() {
                Ok(a) => !a.structural_complete,
                Err(_) => true,
            });
        }

        fn child_failed(&mut self, raw: i32) {
            self.root.outcome = Outcome::Failure;
            self.root.launches[0].outcome = LaunchOutcome::Failure;
            self.root.launches[0].wait_status = Some(raw);
            self.journals[5].launch = Some(self.root.launches[0].clone());
            self.unit_failed();
        }

        fn unit_failed(&mut self) {
            self.root.outcome = Outcome::Failure;
            self.root.units[0].outcome = UnitOutcome::Failure;
            self.journals.last_mut().unwrap().unit = Some(self.root.units[0].clone());
        }
    }

    fn empty() -> Fixture {
        let initial = Manifest {
            schema: 1,
            profile: PROFILE.into(),
            run_id: "run-1".into(),
            authority_root: "compiler-1".into(),
            parent_authority_root: "owner-1".into(),
            scheduler_capture_id: "scheduler-1".into(),
            expected_units: 0,
            expected_launches: 0,
            expected_journals: 0,
            sealed: false,
            interrupted: false,
            lost_events: 0,
            outcome: Outcome::Pending,
            units: vec![],
            launches: vec![],
        };
        let mut root = initial.clone();
        root.sealed = true;
        root.outcome = Outcome::Success;
        Fixture {
            root,
            initial,
            journals: vec![],
            scheduler: SchedulerSummary {
                run_id: "run-1".into(),
                authority_root: "owner-1".into(),
                capture_id: "scheduler-1".into(),
                unit_ids: BTreeSet::new(),
                structural_complete: true,
            },
            entries: vec![],
            queries: vec![],
        }
    }

    fn declare_unit(f: &mut Fixture) -> Unit {
        // Deliberately not a contiguous JobId: this is the exact scheduler unit id.
        let unit = Unit {
            unit_id: 42,
            kind: UnitKind::Rustc,
            disposition: Disposition::Pending,
            outcome: UnitOutcome::Pending,
            terminal: false,
        };
        f.scheduler.unit_ids.insert(unit.unit_id);
        f.root.expected_units = 1;
        f.push(Event::UnitDeclared, Some(unit.clone()), None);
        unit
    }

    fn successful() -> Fixture {
        let mut f = empty();
        let mut unit = declare_unit(&mut f);
        unit.disposition = Disposition::Required;
        f.push(Event::UnitResolution, Some(unit.clone()), None);
        let mut launch = Launch {
            invocation_id: "invocation-1".into(),
            launch_ordinal: 0,
            unit_id: 42,
            parent_task_id: "cargo-unit-42".into(),
            entry_capture_id: "entry-1".into(),
            capture_id: "query-1".into(),
            spawn_attempts: 0,
            spawned: false,
            terminal: false,
            outcome: LaunchOutcome::Pending,
            wait_status: None,
        };
        f.root.expected_launches = 1;
        f.push(Event::LaunchDeclared, None, Some(launch.clone()));
        launch.spawn_attempts = 1;
        f.push(Event::SpawnAttempt, None, Some(launch.clone()));
        launch.spawned = true;
        f.push(Event::SpawnResult, None, Some(launch.clone()));
        launch.terminal = true;
        launch.outcome = LaunchOutcome::Success;
        launch.wait_status = Some(0);
        f.push(Event::WaitResult, None, Some(launch.clone()));
        unit.terminal = true;
        unit.outcome = UnitOutcome::Success;
        f.push(Event::UnitTerminal, Some(unit.clone()), None);
        f.root.units.push(unit);
        f.root.launches.push(launch);
        f.entries.push(Entry {
            schema: 1,
            producer: "rustc-query-admission-entry".into(),
            scope: "compiler-entry-to-query-capture-binding".into(),
            capture_id: "entry-1".into(),
            run_id: "run-1".into(),
            invocation_id: "invocation-1".into(),
            parent_invocation_id: "compiler-1".into(),
            parent_task_id: "cargo-unit-42".into(),
            expected_query_captures: 1,
            query_capture_ids: vec!["query-1".into()],
            query_receipt_schema: 3,
            full_pipeline_complete: false,
            cpu_work_ns: None,
            execution_span_ns: None,
        });
        f.queries.push(QuerySummary {
            schema: 3,
            capture_id: "query-1".into(),
            structural_complete: true,
        });
        f
    }

    #[test]
    fn supported_query_schema_must_equal_its_independent_entry_declaration() {
        for schema in [3, 4] {
            let mut fixture = successful();
            fixture.entries[0].query_receipt_schema = schema;
            fixture.queries[0].schema = schema;
            assert!(fixture.assess().unwrap().structural_complete);
            fixture.queries[0].schema = if schema == 3 { 4 } else { 3 };
            let assessment = fixture.assess().unwrap();
            assert!(!assessment.structural_complete);
            assert!(
                assessment
                    .issues
                    .iter()
                    .any(|issue| issue.contains("exact schema"))
            );
        }
    }

    fn no_child(disposition: Disposition, outcome: UnitOutcome) -> Fixture {
        let mut f = empty();
        let mut unit = declare_unit(&mut f);
        unit.disposition = disposition;
        if disposition != Disposition::Pending {
            f.push(Event::UnitResolution, Some(unit.clone()), None);
        }
        unit.outcome = outcome;
        unit.terminal = true;
        f.push(Event::UnitTerminal, Some(unit.clone()), None);
        f.root.units.push(unit);
        f.root.outcome = if outcome == UnitOutcome::Success {
            Outcome::Success
        } else {
            Outcome::Failure
        };
        f
    }

    fn spawn_failed() -> Fixture {
        let mut f = successful();
        f.entries.clear();
        f.queries.clear();
        f.journals.remove(5); // no wait, only an observed failed spawn result
        let launch = &mut f.root.launches[0];
        launch.spawned = false;
        launch.outcome = LaunchOutcome::SpawnFailed;
        launch.wait_status = None;
        f.journals[4].launch = Some(launch.clone());
        f.journals[5].sequence = 6;
        f.root.expected_journals = 6;
        f.unit_failed();
        f
    }

    #[test]
    fn success_valid_and_no_full_pipeline_or_timing_claims() {
        let f = successful();
        let a = f.assess().unwrap();
        assert!(a.structural_complete, "{:?}", a.issues);
        assert_eq!(a.scope, SCOPE);
        assert_eq!(a.outcome, Outcome::Success);
        assert_eq!(
            (
                a.observed_units,
                a.observed_launches,
                a.observed_entries,
                a.observed_queries
            ),
            (1, 1, 1, 1)
        );
        assert!(!a.full_pipeline_complete);
        assert_eq!((a.cpu_work_ns, a.execution_span_ns), (None, None));
        assert!(empty().assess().unwrap().structural_complete);
    }

    #[test]
    fn typed_wire_roundtrips() {
        let f = successful();
        assert_eq!(
            parse_manifest(&serde_json::to_vec(&f.root).unwrap()).unwrap(),
            f.root
        );
        assert_eq!(
            parse_manifest(&serde_json::to_vec(&f.initial).unwrap()).unwrap(),
            f.initial
        );
        for j in &f.journals {
            assert_eq!(parse_journal(&serde_json::to_vec(j).unwrap()).unwrap(), *j);
        }
        assert_eq!(
            parse_entry(&serde_json::to_vec(&f.entries[0]).unwrap()).unwrap(),
            f.entries[0]
        );
    }

    #[test]
    fn missing_entry_cannot_be_substituted_by_final_query() {
        let mut f = successful();
        f.entries.clear();
        let a = f.assess().unwrap();
        assert!(!a.structural_complete);
        assert_eq!(a.observed_queries, 0);
    }

    #[test]
    fn missing_query_cannot_be_substituted_by_entry() {
        let mut f = successful();
        f.queries.clear();
        assert!(!f.assess().unwrap().structural_complete);
    }

    #[test]
    fn schema_one_or_two_query_substitution_is_incomplete() {
        for schema in [1, 2] {
            let mut f = successful();
            f.queries[0].schema = schema;
            let a = f.assess().unwrap();
            assert!(!a.structural_complete);
            assert_eq!(a.observed_queries, 0);
        }
    }

    #[test]
    fn failed_query_capture_does_not_infer_process_failure() {
        let mut f = successful();
        f.queries[0].structural_complete = false;
        let a = f.assess().unwrap();
        assert!(!a.structural_complete);
        assert_eq!(a.outcome, Outcome::Success);
    }

    #[test]
    fn actual_failed_child_needs_entry_and_complete_query_even_on_failure() {
        let mut f = successful();
        f.child_failed(256);
        let a = f.assess().unwrap();
        assert!(a.structural_complete);
        assert_eq!(a.outcome, Outcome::Failure);
        f.entries.clear();
        assert!(!f.assess().unwrap().structural_complete);
        let mut f = successful();
        f.child_failed(9);
        f.queries.clear();
        assert!(!f.assess().unwrap().structural_complete);
    }

    #[test]
    fn successful_child_may_have_postlaunch_unit_failure() {
        let mut f = successful();
        f.unit_failed();
        assert!(f.assess().unwrap().structural_complete);
        assert_eq!(f.root.launches[0].outcome, LaunchOutcome::Success);
    }

    #[test]
    fn missing_each_journal_is_incomplete() {
        for index in 0..7 {
            let mut f = successful();
            f.journals.remove(index);
            assert!(!f.assess().unwrap().structural_complete);
        }
    }

    #[test]
    fn journals_accept_input_order_but_require_contiguous_sequence() {
        let mut f = successful();
        f.journals.reverse();
        assert!(f.assess().unwrap().structural_complete);
        f.journals[0].sequence = 99;
        assert!(!f.assess().unwrap().structural_complete);
    }

    #[test]
    fn independent_root_counts_survive_record_deletion() {
        let mut f = successful();
        f.root.units.clear();
        f.rejected_or_incomplete();
        let mut f = successful();
        f.root.launches.clear();
        f.rejected_or_incomplete();
        let mut f = successful();
        f.root.expected_launches = 2;
        f.rejected_or_incomplete();
        let mut f = successful();
        f.root.expected_journals = 0;
        f.rejected_or_incomplete();
    }

    #[test]
    fn deleting_root_records_and_counters_cannot_erase_scheduler_units() {
        let mut f = empty();
        f.scheduler.unit_ids.insert(42);
        assert!(!f.assess().unwrap().structural_complete);
    }

    #[test]
    fn successful_required_unit_without_launch_is_incomplete() {
        let f = no_child(Disposition::Required, UnitOutcome::Success);
        assert!(!f.assess().unwrap().structural_complete);
    }

    #[test]
    fn missing_launch_declaration_is_not_reconstructed_from_wait() {
        let mut f = successful();
        f.journals.remove(2);
        for (index, journal) in f.journals.iter_mut().enumerate() {
            journal.sequence = index as u64 + 1;
        }
        f.root.expected_journals = f.journals.len() as u64;
        assert!(f.assess().is_err());
    }

    #[test]
    fn exact_scheduler_mapping_owner_run_capture_and_completion_required() {
        let mut f = successful();
        f.scheduler.unit_ids = BTreeSet::from([0]); // JobId is not the exact unit map
        assert!(!f.assess().unwrap().structural_complete);
        let mut f = successful();
        f.scheduler.capture_id = "other-capture".into();
        assert!(!f.assess().unwrap().structural_complete);
        let mut f = successful();
        f.scheduler.authority_root = "other-owner".into();
        assert!(!f.assess().unwrap().structural_complete);
        let mut f = successful();
        f.scheduler.run_id = "other-run".into();
        assert!(!f.assess().unwrap().structural_complete);
        let mut f = successful();
        f.scheduler.structural_complete = false;
        assert!(!f.assess().unwrap().structural_complete);
    }

    #[test]
    fn fresh_cutoff_not_started_and_prelaunch_failure_expect_no_child() {
        for (disposition, outcome) in [
            (Disposition::Fresh, UnitOutcome::Success),
            (Disposition::Cutoff, UnitOutcome::Success),
            (Disposition::Pending, UnitOutcome::NotStarted),
            (Disposition::Required, UnitOutcome::Failure),
            (Disposition::Required, UnitOutcome::Cancelled),
        ] {
            let mut f = no_child(disposition, outcome);
            assert!(
                f.assess().unwrap().structural_complete,
                "{disposition:?}/{outcome:?}"
            );
            f.queries.push(QuerySummary {
                schema: 3,
                capture_id: "foreign-query".into(),
                structural_complete: true,
            });
            assert!(!f.assess().unwrap().structural_complete);
        }
    }

    #[test]
    fn unsupported_units_never_claim_complete_compiler_coverage() {
        let f = no_child(Disposition::Unsupported, UnitOutcome::Success);
        assert!(!f.assess().unwrap().structural_complete);
        let mut f = no_child(Disposition::Required, UnitOutcome::Failure);
        let mut unsupported = f.journals[1].unit.clone().unwrap();
        unsupported.disposition = Disposition::Unsupported;
        f.journals.insert(
            2,
            Journal {
                schema: 1,
                profile: PROFILE.into(),
                authority_root: f.root.authority_root.clone(),
                sequence: 3,
                event: Event::UnitResolution,
                unit: Some(unsupported),
                launch: None,
            },
        );
        f.root.units[0].disposition = Disposition::Unsupported;
        f.journals[3].unit = Some(f.root.units[0].clone());
        f.journals[3].sequence = 4;
        f.root.expected_journals = 4;
        assert!(!f.assess().unwrap().structural_complete);
    }

    #[test]
    fn spawn_failure_requires_observed_attempt_failure_and_no_child() {
        let mut f = spawn_failed();
        assert!(f.assess().unwrap().structural_complete);
        f.entries.push(successful().entries[0].clone());
        assert!(f.assess().is_err());
        let mut f = spawn_failed();
        f.journals[4].event = Event::WaitResult;
        assert!(f.assess().is_err());
        let mut f = spawn_failed();
        f.root.launches[0].spawn_attempts = 0;
        assert!(f.assess().is_err());
    }

    #[test]
    fn retry_requires_result_between_monotonic_attempts() {
        let mut f = successful();
        let retry = Journal {
            schema: 1,
            profile: PROFILE.into(),
            authority_root: f.root.authority_root.clone(),
            sequence: 5,
            event: Event::SpawnResult,
            unit: None,
            launch: f.journals[3].launch.clone(),
        };
        let mut second_attempt = f.journals[3].clone();
        second_attempt.launch.as_mut().unwrap().spawn_attempts = 2;
        f.journals.insert(4, retry);
        f.journals.insert(5, second_attempt);
        for (i, journal) in f.journals.iter_mut().enumerate() {
            journal.sequence = i as u64 + 1;
            if i >= 6
                && let Some(launch) = &mut journal.launch
            {
                launch.spawn_attempts = 2;
            }
        }
        f.root.launches[0].spawn_attempts = 2;
        f.root.expected_journals = 9;
        assert!(f.assess().unwrap().structural_complete);
        f.journals[4].event = Event::SpawnAttempt;
        assert!(f.assess().is_err());
    }

    #[test]
    fn impossible_wait_and_unit_outcomes_rejected() {
        let mut f = successful();
        f.root.launches[0].wait_status = Some(256);
        assert!(f.assess().is_err());
        let mut f = successful();
        f.root.launches[0].wait_status = None;
        assert!(f.assess().is_err());
        let mut f = successful();
        f.root.launches[0].wait_status = Some(-1);
        assert!(f.assess().is_err());
        let mut f = successful();
        f.child_failed(256);
        f.root.units[0].outcome = UnitOutcome::Success;
        f.journals[6].unit = Some(f.root.units[0].clone());
        assert!(f.assess().is_err());
        let mut f = successful();
        f.root.units[0].outcome = UnitOutcome::Pending;
        assert!(f.assess().is_err());
    }

    #[test]
    fn duplicate_identities_sequences_and_foreign_journals_rejected() {
        let mut f = successful();
        f.root.units.push(f.root.units[0].clone());
        assert!(f.assess().is_err());
        let mut f = successful();
        f.root.launches.push(f.root.launches[0].clone());
        assert!(f.assess().is_err());
        let mut f = successful();
        f.journals.push(f.journals[0].clone());
        assert!(f.assess().is_err());
        let mut f = successful();
        f.entries.push(f.entries[0].clone());
        assert!(f.assess().is_err());
        let mut f = successful();
        f.queries.push(f.queries[0].clone());
        assert!(f.assess().is_err());
        let mut f = successful();
        f.journals[0].authority_root = "foreign-owner".into();
        assert!(f.assess().is_err());
    }

    #[test]
    fn entry_binding_echoes_all_parent_and_predetermined_capture_fields() {
        for field in 0..6 {
            let mut f = successful();
            match field {
                0 => f.entries[0].run_id = "other-run".into(),
                1 => f.entries[0].invocation_id = "other-invocation".into(),
                2 => f.entries[0].parent_invocation_id = "other-parent".into(),
                3 => f.entries[0].parent_task_id = "cargo-unit-43".into(),
                4 => f.entries[0].query_capture_ids[0] = "other-query".into(),
                _ => f.entries[0].capture_id = "other-entry".into(),
            }
            let a = f.assess().unwrap();
            assert!(!a.structural_complete);
            assert_eq!((a.observed_entries, a.observed_queries), (0, 0));
        }
        let mut f = successful();
        f.queries[0].capture_id = "other-query".into();
        assert!(!f.assess().unwrap().structural_complete);
    }

    #[test]
    fn no_unsealed_interrupted_or_lossy_authority_is_complete() {
        let mut f = successful();
        f.root.sealed = false;
        assert!(!f.assess().unwrap().structural_complete);
        let mut f = successful();
        f.root.interrupted = true;
        assert!(!f.assess().unwrap().structural_complete);
        let mut f = successful();
        f.root.lost_events = 1;
        assert!(!f.assess().unwrap().structural_complete);
        let mut f = successful();
        f.initial = f.root.clone();
        assert!(f.assess().is_err());
        let mut f = successful();
        f.initial.authority_root = "wrong-initial".into();
        assert!(f.assess().is_err());
    }

    #[test]
    fn strict_duplicate_unknown_and_missing_fields_at_every_wire_level() {
        let f = successful();
        let root = serde_json::to_string(&f.root).unwrap();
        assert!(
            parse_manifest(
                root.replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1)
                    .as_bytes()
            )
            .is_err()
        );
        assert!(
            parse_manifest(
                root.replacen("\"unit_id\":42", "\"unit_id\":42,\"unit_id\":42", 1)
                    .as_bytes()
            )
            .is_err()
        );
        assert!(
            parse_manifest(
                root.replacen(
                    "\"wait_status\":0",
                    "\"wait_status\":0,\"wait_status\":0",
                    1
                )
                .as_bytes()
            )
            .is_err()
        );
        assert!(parse_manifest(root.replacen(",\"wait_status\":0", "", 1).as_bytes()).is_err());
        assert!(
            parse_manifest(
                root.replacen("\"schema\":1", "\"schema\":1,\"foreign\":null", 1)
                    .as_bytes()
            )
            .is_err()
        );
        let journal = serde_json::to_string(&f.journals[0]).unwrap();
        assert!(parse_journal(journal.replacen(",\"launch\":null", "", 1).as_bytes()).is_err());
        assert!(
            parse_journal(
                journal
                    .replacen("\"launch\":null", "\"launch\":null,\"launch\":null", 1)
                    .as_bytes()
            )
            .is_err()
        );
        let entry = serde_json::to_string(&f.entries[0]).unwrap();
        for key in ["cpu_work_ns", "execution_span_ns"] {
            assert!(
                parse_entry(
                    entry
                        .replacen(&format!(",\"{key}\":null"), "", 1)
                        .as_bytes()
                )
                .is_err()
            );
            assert!(
                parse_entry(
                    entry
                        .replacen(
                            &format!("\"{key}\":null"),
                            &format!("\"{key}\":null,\"{key}\":null"),
                            1
                        )
                        .as_bytes()
                )
                .is_err()
            );
        }
        assert!(
            parse_entry(
                entry
                    .replacen("\"schema\":1", "\"schema\":1,\"schema\":1", 1)
                    .as_bytes()
            )
            .is_err()
        );
        assert!(
            parse_entry(
                entry
                    .replacen(",\"query_receipt_schema\":3", "", 1)
                    .as_bytes()
            )
            .is_err()
        );
        assert!(
            parse_entry(
                entry
                    .replacen(
                        "\"query_receipt_schema\":3",
                        "\"query_receipt_schema\":2",
                        1
                    )
                    .as_bytes()
            )
            .is_err()
        );
    }

    #[test]
    fn unit_success_cannot_precede_observed_child_wait() {
        let mut f = successful();
        f.journals.swap(5, 6);
        for (i, journal) in f.journals.iter_mut().enumerate() {
            journal.sequence = i as u64 + 1;
        }
        assert!(f.assess().is_err());
    }

    #[test]
    fn cross_child_ids_and_non_rustc_required_coverage_rejected() {
        let mut f = successful();
        f.root.launches[0].capture_id = f.root.launches[0].entry_capture_id.clone();
        assert!(f.assess().is_err());
        let mut f = successful();
        f.root.units[0].kind = UnitKind::Rustdoc;
        assert!(f.assess().is_err());
        let mut f = successful();
        f.root.launches[0].parent_task_id = "cargo-unit-0".into();
        assert!(f.assess().is_err());
    }

    #[test]
    fn entry_cannot_inflate_scope_counts_schema_or_timing() {
        for field in 0..6 {
            let mut f = successful();
            match field {
                0 => f.entries[0].full_pipeline_complete = true,
                1 => f.entries[0].cpu_work_ns = Some(0),
                2 => f.entries[0].execution_span_ns = Some(0),
                3 => f.entries[0].expected_query_captures = 0,
                4 => f.entries[0].query_capture_ids.push("query-2".into()),
                _ => f.entries[0].query_receipt_schema = 2,
            }
            assert!(f.assess().is_err());
        }
    }

    #[test]
    fn ids_counts_bytes_and_json_types_bounded() {
        let mut f = successful();
        for invalid in [
            "".to_string(),
            "x".repeat(129),
            "path/to/file".into(),
            "non ascii é".into(),
        ] {
            f.root.run_id = invalid;
            assert!(parse_manifest(&serde_json::to_vec(&f.root).unwrap()).is_err());
        }
        let mut f = successful();
        f.root.expected_units = MAX_RECORDS as u64 + 1;
        assert!(parse_manifest(&serde_json::to_vec(&f.root).unwrap()).is_err());
        let mut f = successful();
        f.root.expected_journals = MAX_JOURNALS as u64 + 1;
        assert!(parse_manifest(&serde_json::to_vec(&f.root).unwrap()).is_err());
        assert!(parse_entry(&vec![b' '; MAX_SMALL_BYTES + 1]).is_err());
        assert!(parse_journal(&vec![b' '; MAX_SMALL_BYTES + 1]).is_err());
        let root = serde_json::to_string(&successful().root).unwrap();
        for invalid in ["-1", "1.5", "\"1\"", "null"] {
            assert!(
                parse_manifest(
                    root.replacen(
                        "\"expected_units\":1",
                        &format!("\"expected_units\":{invalid}"),
                        1
                    )
                    .as_bytes()
                )
                .is_err()
            );
        }
        assert!(parse_manifest(format!("{root} {{}}").as_bytes()).is_err());
    }
}
