//! Human-only dashboard. No build, billing, cancellation or configuration mutations.
//! Network and disk work stay off the terminal thread. Existing commands never enter it.
mod data;
mod view;

use anyhow::{Result, bail};
use data::{RunRow, Snapshot};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::io::IsTerminal;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

/// Operation categories, matching the command axis of `bench/synth` so the
/// dashboard and the benchmark workload describe a run the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Check,
    Test,
    Build,
    Release,
    Clippy,
    Run,
    Other,
}

impl Kind {
    pub(crate) const ALL: [Kind; 7] = [
        Kind::Check,
        Kind::Test,
        Kind::Build,
        Kind::Release,
        Kind::Clippy,
        Kind::Run,
        Kind::Other,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Kind::Check => "check",
            Kind::Test => "test",
            Kind::Build => "build",
            Kind::Release => "release",
            Kind::Clippy => "clippy",
            Kind::Run => "run",
            Kind::Other => "other",
        }
    }

    /// Classify recorded cargo arguments. A record without arguments (account
    /// metadata that has not been opened) is `Other`, never a guessed command.
    pub(crate) fn of(args: &[String]) -> Kind {
        let cargo: Vec<&str> = args
            .iter()
            .map(String::as_str)
            .skip_while(|a| a.starts_with('+'))
            .take_while(|a| *a != "--")
            .collect();
        let release = cargo.iter().enumerate().any(|(i, arg)| {
            matches!(*arg, "--release" | "-r" | "--profile=release")
                || (*arg == "--profile" && cargo.get(i + 1) == Some(&"release"))
        });
        match cargo.first().copied() {
            Some("check" | "c") => Kind::Check,
            Some("test" | "t" | "nextest") => Kind::Test,
            Some("clippy") => Kind::Clippy,
            Some("run" | "r") => Kind::Run,
            Some("build" | "b") if release => Kind::Release,
            Some("build" | "b") => Kind::Build,
            _ => Kind::Other,
        }
    }
}

/// Where compilation ran. Records that do not say are in neither subset.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    #[default]
    Any,
    Remote,
    Local,
}

impl Place {
    fn next(self) -> Place {
        match self {
            Place::Any => Place::Remote,
            Place::Remote => Place::Local,
            Place::Local => Place::Any,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Place::Any => "anywhere",
            Place::Remote => "remote only",
            Place::Local => "local only",
        }
    }

    fn matches(self, place: &str) -> bool {
        match self {
            Place::Any => true,
            Place::Remote => matches!(place, "remote" | "hosted"),
            Place::Local => place == "local",
        }
    }
}

/// Which results to list. The "whose fault" choices use only the class the
/// recorder stored, so an unclassified failure is in neither.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    #[default]
    Any,
    Failed,
    Passed,
    YourCode,
    Ours,
}

impl Outcome {
    const ALL: [Outcome; 5] = [
        Outcome::Any,
        Outcome::Failed,
        Outcome::Passed,
        Outcome::YourCode,
        Outcome::Ours,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Outcome::Any => "all results",
            Outcome::Failed => "failed only",
            Outcome::Passed => "passed only",
            Outcome::YourCode => "failed: your code",
            Outcome::Ours => "failed: toolchain or infra",
        }
    }

    fn matches(self, row: &RunRow) -> bool {
        let failed = row.exit.is_some_and(|code| code != 0);
        match self {
            Outcome::Any => true,
            Outcome::Failed => failed,
            Outcome::Passed => row.exit == Some(0),
            Outcome::YourCode => failed && row.failure == Some(crate::failure::Class::UserCode),
            Outcome::Ours => failed && row.failure.is_some_and(|class| class.is_ours()),
        }
    }
}

/// Rows of the filter menu, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuRow {
    Project,
    Kind,
    Place,
    Outcome,
    Search,
}

impl MenuRow {
    pub(crate) const ALL: [MenuRow; 5] = [
        MenuRow::Project,
        MenuRow::Kind,
        MenuRow::Place,
        MenuRow::Outcome,
        MenuRow::Search,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            MenuRow::Project => "Project",
            MenuRow::Kind => "Command",
            MenuRow::Place => "Where",
            MenuRow::Outcome => "Result",
            MenuRow::Search => "Search",
        }
    }
}

/// Where the wait went across the listed runs.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct Profile {
    /// Listed runs, and how many of them have timings on this client.
    pub runs: usize,
    pub timed: usize,
    /// Seconds waited by the timed runs. The buckets sum to it.
    pub total: f64,
    /// Largest first, empty buckets left out.
    pub buckets: Vec<(data::Bucket, f64)>,
}

/// What the compiler's time went to across the listed runs.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct CompileProfile {
    /// Listed runs with units recorded on this client.
    pub timed: usize,
    /// Unit seconds of those runs. Units overlap, so this exceeds the wait.
    pub total: f64,
    /// Largest first, empty parts left out.
    pub parts: Vec<(data::Part, f64)>,
}

/// Last path component, the way a person names a project.
pub(crate) fn project_name(path: &str) -> &str {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}

#[derive(Default)]
struct State {
    snapshot: Snapshot,
    hosted_runs: Vec<RunRow>,
    selected: usize,
    selected_id: Option<String>,
    /// Directory whose runs (and those below it) are listed.
    project: Option<String>,
    outcome: Outcome,
    /// Selected row of the filter menu while it is open.
    menu: Option<usize>,
    kind: Option<Kind>,
    place: Place,
    /// Case-insensitive substring over command, project path and run ID.
    query: String,
    searching: bool,
    detail: Option<String>,
    detail_scroll: usize,
    loading_detail: bool,
    detail_request: u64,
    refreshing: bool,
    cwd: String,
}

impl State {
    fn rows(&self) -> Vec<&RunRow> {
        self.filtered(true)
    }

    /// Runs per kind under every other filter, so the kind bar shows what
    /// choosing each kind would list.
    fn kind_counts(&self) -> [usize; Kind::ALL.len()] {
        let mut counts = [0; Kind::ALL.len()];
        for row in self.filtered(false) {
            counts[Kind::of(&row.args) as usize] += 1;
        }
        counts
    }

    fn filters_active(&self) -> bool {
        self.project.is_some()
            || self.outcome != Outcome::Any
            || self.kind.is_some()
            || self.place != Place::Any
            || !self.query.is_empty()
    }

    /// Projects with recorded runs, most runs first, ignoring every filter.
    fn projects(&self) -> Vec<String> {
        let mut counts = std::collections::BTreeMap::<&str, usize>::new();
        for row in self.snapshot.runs.iter().chain(&self.hosted_runs) {
            if !row.cwd.is_empty() {
                *counts.entry(row.cwd.as_str()).or_default() += 1;
            }
        }
        let mut projects: Vec<_> = counts.into_iter().collect();
        projects.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        projects
            .into_iter()
            .take(30)
            .map(|(path, _)| path.to_owned())
            .collect()
    }

    fn cycle_project(&mut self, forward: bool) {
        let projects = self.projects();
        let n = projects.len() + 1;
        // A project that no longer has runs counts as "all" for stepping.
        let at = self
            .project
            .as_ref()
            .and_then(|p| projects.iter().position(|q| q == p))
            .map_or(0, |i| i + 1);
        let next = if forward { at + 1 } else { at + n - 1 } % n;
        self.project = next.checked_sub(1).map(|i| projects[i].clone());
        self.selected_id = None;
    }

    fn cycle_outcome(&mut self, forward: bool) {
        let n = Outcome::ALL.len();
        let at = Outcome::ALL
            .iter()
            .position(|o| *o == self.outcome)
            .unwrap_or(0);
        self.outcome = Outcome::ALL[if forward { at + 1 } else { at + n - 1 } % n];
        self.selected_id = None;
    }

    /// Step the value of a menu row. Search has no values to step through.
    fn cycle(&mut self, row: MenuRow, forward: bool) {
        match row {
            MenuRow::Project => self.cycle_project(forward),
            MenuRow::Kind => self.cycle_kind(forward),
            MenuRow::Place => {
                self.place = if forward {
                    self.place.next()
                } else {
                    self.place.next().next()
                };
                self.selected_id = None;
            }
            MenuRow::Outcome => self.cycle_outcome(forward),
            MenuRow::Search => {}
        }
    }

    /// The current value of a menu row, as shown in the menu.
    fn menu_value(&self, row: MenuRow) -> String {
        match row {
            MenuRow::Project => self
                .project
                .as_deref()
                .map_or("all projects".into(), |p| project_name(p).to_owned()),
            MenuRow::Kind => self.kind.map_or("all commands", Kind::label).into(),
            MenuRow::Place => self.place.label().into(),
            MenuRow::Outcome => self.outcome.label().into(),
            MenuRow::Search if self.query.is_empty() => "none".into(),
            MenuRow::Search => format!("\"{}\"", self.query),
        }
    }

    /// Where the wait went for the listed runs, so it follows the filters.
    fn profile(&self) -> Profile {
        let rows = self.rows();
        let mut sums = [0.0; data::BUCKETS];
        let mut timed = 0;
        for time in rows.iter().filter_map(|row| row.time) {
            timed += 1;
            for (sum, secs) in sums.iter_mut().zip(time) {
                *sum += secs;
            }
        }
        let mut buckets: Vec<_> = data::Bucket::ALL
            .into_iter()
            .zip(sums)
            .filter(|(_, secs)| *secs > 0.0)
            .collect();
        buckets.sort_by(|a, b| b.1.total_cmp(&a.1));
        Profile {
            runs: rows.len(),
            timed,
            total: sums.iter().sum(),
            buckets,
        }
    }

    /// What the compiler's time went to for the listed runs.
    fn compile_profile(&self) -> CompileProfile {
        let mut sums = [0.0; data::PARTS];
        let mut timed = 0;
        for time in self.rows().iter().filter_map(|row| row.compile) {
            timed += 1;
            for (sum, secs) in sums.iter_mut().zip(time) {
                *sum += secs;
            }
        }
        let mut parts: Vec<_> = data::Part::ALL
            .into_iter()
            .zip(sums)
            .filter(|(_, secs)| *secs > 0.0)
            .collect();
        parts.sort_by(|a, b| b.1.total_cmp(&a.1));
        CompileProfile {
            timed,
            total: sums.iter().sum(),
            parts,
        }
    }

    fn clear_filters(&mut self) {
        self.project = None;
        self.outcome = Outcome::Any;
        self.kind = None;
        self.place = Place::Any;
        self.query.clear();
        self.searching = false;
        self.selected_id = None;
    }

    fn cycle_kind(&mut self, forward: bool) {
        let n = Kind::ALL.len() + 1;
        let at = self.kind.map_or(0, |k| k as usize + 1);
        let next = if forward { at + 1 } else { at + n - 1 } % n;
        self.kind = next.checked_sub(1).map(|i| Kind::ALL[i]);
        self.selected_id = None;
    }

    fn filtered(&self, by_kind: bool) -> Vec<&RunRow> {
        let query = self.query.to_lowercase();
        // Deduplicate before sorting: a cloud upload timestamp can be later than
        // the actual run start, but must not replace the richer local record.
        let local_ids: std::collections::HashSet<_> =
            self.snapshot.runs.iter().map(|r| r.id.as_str()).collect();
        let mut rows: Vec<_> = self
            .snapshot
            .runs
            .iter()
            .chain(
                self.hosted_runs
                    .iter()
                    .filter(|r| !local_ids.contains(r.id.as_str())),
            )
            .filter(|r| {
                self.project
                    .as_ref()
                    .is_none_or(|p| std::path::Path::new(&r.cwd).starts_with(p))
            })
            .filter(|r| self.outcome.matches(r))
            .filter(|r| self.place.matches(&r.place))
            .filter(|r| !by_kind || self.kind.is_none_or(|k| Kind::of(&r.args) == k))
            .filter(|r| {
                query.is_empty()
                    || r.args.join(" ").to_lowercase().contains(&query)
                    || r.cwd.to_lowercase().contains(&query)
                    || r.id.to_lowercase().contains(&query)
            })
            .collect();
        rows.sort_by(|a, b| b.start.total_cmp(&a.start).then_with(|| a.id.cmp(&b.id)));
        // Local recordings have richer information than cloud list metadata.
        let mut seen = std::collections::HashSet::new();
        rows.retain(|r| seen.insert(r.id.as_str()));
        rows
    }

    fn reconcile_selection(&mut self) {
        let rows = self.rows();
        let selected = self
            .selected_id
            .as_ref()
            .and_then(|id| rows.iter().position(|r| &r.id == id))
            .unwrap_or_else(|| self.selected.min(rows.len().saturating_sub(1)));
        let id = rows.get(selected).map(|r| r.id.clone());
        self.selected = selected;
        self.selected_id = id;
    }
}

enum Update {
    Local(Snapshot),
    Hosted(Result<data::HostedSnapshot>),
    Detail(u64, Result<String>),
}

// Drop is also run on a terminal read/draw error. ratatui::init installs the
// equivalent panic hook, so unwinding does not leave the terminal in raw mode.
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

fn hosted_worker(tx: mpsc::Sender<Update>) -> SyncSender<()> {
    let (refresh, requests) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        loop {
            let result = data::fetch_hosted();
            if tx.send(Update::Hosted(result)).is_err() {
                break;
            }
            match requests.recv_timeout(Duration::from_secs(30)) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    refresh
}

fn apply_updates(state: &mut State, updates: &Receiver<Update>) {
    for update in updates.try_iter() {
        match update {
            Update::Local(local) => {
                state.snapshot.active = local.active;
                state.snapshot.runs = local.runs;
                state.snapshot.local_error = local.local_error;
            }
            Update::Hosted(Ok(hosted)) => {
                state.snapshot.account = Some(hosted.account);
                if hosted.history_error.is_none() {
                    state.hosted_runs = hosted.runs;
                    state.snapshot.hosted_at = Some(crate::paths::now());
                }
                state.snapshot.hosted_error = hosted.history_error;
                state.refreshing = false;
            }
            Update::Hosted(Err(error)) => {
                state.snapshot.hosted_error = Some(error.to_string());
                state.refreshing = false;
            }
            Update::Detail(request, result) if request == state.detail_request => {
                state.detail =
                    Some(result.unwrap_or_else(|e| format!("Could not load this run: {e}")));
                state.loading_detail = false;
            }
            Update::Detail(_, _) => {}
        }
    }
    state.reconcile_selection();
}

/// Refuse redirection before creating workers or entering raw mode.
pub(crate) fn run() -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!(
            "`justrust interactive dashboard` requires a terminal on stdin and stdout. For agents use `justrust status --json`, `justrust runs --json`, or `justrust show ID --json`."
        );
    }
    let (tx, updates) = mpsc::channel();
    let refresh = hosted_worker(tx.clone());
    let local_tx = tx.clone();
    std::thread::spawn(move || {
        let mut times = data::TimeCache::new();
        loop {
            if local_tx
                .send(Update::Local(data::local_snapshot(&mut times)))
                .is_err()
            {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
    let mut state = State {
        cwd: std::env::current_dir()?.to_string_lossy().into_owned(),
        refreshing: true,
        ..Default::default()
    };
    let mut terminal = ratatui::try_init()?;
    let _guard = TerminalGuard;
    let mut last_refresh = Instant::now();
    loop {
        apply_updates(&mut state, &updates);
        terminal.draw(|f| view::draw(f, &state))?;
        if !event::poll(Duration::from_millis(150))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            break;
        }
        // Typing a search owns every printable key, including q.
        if state.searching {
            match key.code {
                KeyCode::Enter => state.searching = false,
                KeyCode::Esc => {
                    state.searching = false;
                    state.query.clear();
                }
                KeyCode::Backspace => {
                    state.query.pop();
                }
                KeyCode::Char(c) if state.query.chars().count() < 64 => state.query.push(c),
                _ => {}
            }
            state.selected_id = None;
            continue;
        }
        if key.code == KeyCode::Char('q') {
            break;
        }
        if let Some(at) = state.menu {
            let row = MenuRow::ALL[at.min(MenuRow::ALL.len() - 1)];
            match key.code {
                KeyCode::Esc | KeyCode::Char('s') => state.menu = None,
                KeyCode::Down | KeyCode::Char('j') => {
                    state.menu = Some((at + 1) % MenuRow::ALL.len())
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    state.menu = Some((at + MenuRow::ALL.len() - 1) % MenuRow::ALL.len())
                }
                KeyCode::Enter | KeyCode::Char(' ' | '/') if row == MenuRow::Search => {
                    state.searching = true
                }
                KeyCode::Char('/') => {
                    state.menu = Some(MenuRow::Search as usize);
                    state.searching = true;
                }
                KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ' | 'l') => {
                    state.cycle(row, true)
                }
                KeyCode::Left | KeyCode::Char('h') => state.cycle(row, false),
                KeyCode::Char('x') => state.clear_filters(),
                _ => {}
            }
            continue;
        }
        if state.detail.is_some() || state.loading_detail {
            match key.code {
                KeyCode::Esc => {
                    state.detail = None;
                    state.loading_detail = false;
                    state.detail_request += 1;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    state.detail_scroll = state.detail_scroll.saturating_add(1)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    state.detail_scroll = state.detail_scroll.saturating_sub(1)
                }
                KeyCode::PageDown => state.detail_scroll = state.detail_scroll.saturating_add(10),
                KeyCode::PageUp => state.detail_scroll = state.detail_scroll.saturating_sub(10),
                KeyCode::Home => state.detail_scroll = 0,
                _ => {}
            }
            continue;
        }
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                state.selected = (state.selected + 1).min(state.rows().len().saturating_sub(1));
                state.selected_id = state.rows().get(state.selected).map(|r| r.id.clone());
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.selected = state.selected.saturating_sub(1);
                state.selected_id = state.rows().get(state.selected).map(|r| r.id.clone());
            }
            KeyCode::Char('s') => state.menu = Some(0),
            KeyCode::Char('f') => {
                state.outcome = if state.outcome == Outcome::Failed {
                    Outcome::Any
                } else {
                    Outcome::Failed
                };
                state.selected_id = None;
            }
            KeyCode::Char('h') => {
                state.project = if state.project.as_deref() == Some(state.cwd.as_str()) {
                    None
                } else {
                    Some(state.cwd.clone())
                };
                state.selected_id = None;
            }
            KeyCode::Char('c') | KeyCode::Tab => state.cycle_kind(true),
            KeyCode::Char('C') | KeyCode::BackTab => state.cycle_kind(false),
            KeyCode::Char('w') => {
                state.place = state.place.next();
                state.selected_id = None;
            }
            KeyCode::Char('/') => state.searching = true,
            KeyCode::Char('x') | KeyCode::Esc => state.clear_filters(),
            KeyCode::Char('r') if last_refresh.elapsed() >= Duration::from_secs(1) => {
                if refresh.try_send(()).is_ok() {
                    state.refreshing = true;
                }
                last_refresh = Instant::now();
            }
            KeyCode::Char('a') => {
                state.detail_request += 1;
                state.detail_scroll = 0;
                state.detail = Some(format!(
                    "Account status\n\n{}\n\nHosted error: {}\nLocal error: {}\n\nThis view is read-only. Account changes use justrust login/upgrade separately.",
                    state
                        .snapshot
                        .account
                        .as_ref()
                        .and_then(|m| serde_json::to_string_pretty(m).ok())
                        .unwrap_or_else(|| "Not signed in or account not yet loaded".into()),
                    state.snapshot.hosted_error.as_deref().unwrap_or("none"),
                    state.snapshot.local_error.as_deref().unwrap_or("none")
                ));
            }
            KeyCode::Enter | KeyCode::Char('l') => {
                if let Some(row) = state.rows().get(state.selected).copied().cloned() {
                    let logs = key.code == KeyCode::Char('l');
                    state.detail_request += 1;
                    let request = state.detail_request;
                    state.loading_detail = true;
                    state.detail_scroll = 0;
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        let _ = tx.send(Update::Detail(request, data::fetch_detail(&row, logs)));
                    });
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, start: f64) -> RunRow {
        RunRow {
            id: id.into(),
            start,
            cwd: "/work/project".into(),
            args: vec!["check".into()],
            exit: Some(0),
            wall: 1.0,
            place: "local".into(),
            origin: "local".into(),
            ..Default::default()
        }
    }

    #[test]
    fn selection_tracks_id_across_refresh_and_filters_use_path_boundaries() {
        let mut s = State::default();
        s.snapshot.runs = vec![row("older", 1.0), row("newer", 2.0)];
        s.selected_id = Some("older".into());
        s.reconcile_selection();
        assert_eq!(s.selected, 1);
        s.snapshot.runs.push(row("newest", 3.0));
        s.reconcile_selection();
        assert_eq!(s.selected, 2);
        s.project = Some("/work/pro".into());
        assert!(s.rows().is_empty());
        s.project = Some("/work".into());
        assert_eq!(s.rows().len(), 3);
    }

    #[test]
    fn result_filter_uses_the_recorded_failure_class() {
        use crate::failure::Class;
        let failed = |id: &str, class: Option<Class>| RunRow {
            failure: class,
            ..run_with(id, &["test"], "local", 101)
        };
        let mut s = State::default();
        s.snapshot.runs = vec![
            run_with("a", &["check"], "local", 0),
            failed("b", Some(Class::UserCode)),
            failed("c", Some(Class::Infra)),
            failed("d", Some(Class::Unknown)),
            failed("e", Some(Class::Interrupted)),
            failed("f", None),
            RunRow {
                exit: None,
                ..row("g", 1.0)
            },
        ];
        let ids = |s: &State| {
            let mut ids: Vec<_> = s.rows().iter().map(|r| r.id.clone()).collect();
            ids.sort();
            ids.join("")
        };
        for (outcome, expected) in [
            (Outcome::Any, "abcdefg"),
            (Outcome::Failed, "bcdef"),
            (Outcome::Passed, "a"),
            (Outcome::YourCode, "b"),
            (Outcome::Ours, "cd"),
        ] {
            s.outcome = outcome;
            assert_eq!(ids(&s), expected, "{outcome:?}");
        }
        // Stepping visits every choice and returns to the start.
        s.outcome = Outcome::Any;
        for _ in 0..Outcome::ALL.len() {
            s.cycle_outcome(true);
        }
        assert_eq!(s.outcome, Outcome::Any);
        s.cycle_outcome(false);
        assert_eq!(s.outcome, Outcome::Ours);
    }

    #[test]
    fn project_choices_come_from_recorded_runs_most_used_first() {
        let at = |id: &str, cwd: &str| RunRow {
            cwd: cwd.into(),
            ..row(id, 1.0)
        };
        let mut s = State::default();
        s.snapshot.runs = vec![at("a", "/w/big"), at("b", "/w/big"), at("c", "/w/small")];
        s.hosted_runs = vec![at("d", ""), at("e", "/w/big")];
        assert_eq!(s.projects(), ["/w/big", "/w/small"]);
        assert_eq!(s.menu_value(MenuRow::Project), "all projects");
        s.cycle(MenuRow::Project, true);
        assert_eq!(s.project.as_deref(), Some("/w/big"));
        assert_eq!(s.menu_value(MenuRow::Project), "big");
        assert_eq!(s.rows().len(), 3);
        s.cycle(MenuRow::Project, true);
        assert_eq!(s.rows().len(), 1);
        s.cycle(MenuRow::Project, true);
        assert_eq!(s.project, None);
        s.cycle(MenuRow::Project, false);
        assert_eq!(s.project.as_deref(), Some("/w/small"));
        // A project whose runs aged out of the window steps as "all".
        s.project = Some("/gone".into());
        s.cycle(MenuRow::Project, true);
        assert_eq!(s.project.as_deref(), Some("/w/big"));

        s.cycle(MenuRow::Place, false);
        assert_eq!(s.place, Place::Local);
        s.cycle(MenuRow::Kind, true);
        assert_eq!(s.menu_value(MenuRow::Kind), "check");
        s.cycle(MenuRow::Search, true);
        assert_eq!(s.menu_value(MenuRow::Search), "none");
        assert_eq!(project_name("/"), "/");
    }

    #[test]
    fn profile_sums_timed_runs_and_follows_filters() {
        use data::Bucket;
        let timed = |id: &str, args: &[&str], compile: f64, sync: f64| {
            let mut time = [0.0; data::BUCKETS];
            time[Bucket::Compile as usize] = compile;
            time[Bucket::Sync as usize] = sync;
            RunRow {
                time: Some(time),
                ..run_with(id, args, "remote", 0)
            }
        };
        let mut s = State::default();
        s.snapshot.runs = vec![
            timed("a", &["check"], 2.0, 6.0),
            timed("b", &["test"], 10.0, 1.0),
            run_with("c", &["test"], "remote", 0),
        ];
        let p = s.profile();
        assert_eq!((p.runs, p.timed, p.total), (3, 2, 19.0));
        assert_eq!(p.buckets, [(Bucket::Compile, 12.0), (Bucket::Sync, 7.0)]);
        s.kind = Some(Kind::Check);
        let p = s.profile();
        assert_eq!((p.runs, p.timed, p.total), (1, 1, 8.0));
        assert_eq!(p.buckets[0], (Bucket::Sync, 6.0));
        s.query = "nothing matches".into();
        assert_eq!(s.profile(), Profile::default());
        assert_eq!(s.compile_profile(), CompileProfile::default());

        use data::Part;
        let mut s = State::default();
        let mut a = run_with("a", &["check"], "remote", 0);
        let mut b = run_with("b", &["test"], "remote", 0);
        a.compile = Some([4.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
        b.compile = Some([2.0, 9.0, 0.0, 0.0, 0.0, 0.0]);
        s.snapshot.runs = vec![a, b, run_with("c", &["test"], "remote", 0)];
        let p = s.compile_profile();
        assert_eq!((p.timed, p.total), (2, 16.0));
        assert_eq!(
            p.parts,
            [
                (Part::Tests, 9.0),
                (Part::Workspace, 6.0),
                (Part::Dependencies, 1.0)
            ]
        );
        s.kind = Some(Kind::Check);
        assert_eq!(s.compile_profile().parts[0], (Part::Workspace, 4.0));
    }

    fn run_with(id: &str, args: &[&str], place: &str, exit: i32) -> RunRow {
        RunRow {
            args: args.iter().map(|a| a.to_string()).collect(),
            place: place.into(),
            exit: Some(exit),
            ..row(id, 1.0)
        }
    }

    #[test]
    fn kinds_follow_the_cargo_command_and_never_guess() {
        let kind =
            |args: &[&str]| Kind::of(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        assert_eq!(kind(&["check", "-p", "a"]), Kind::Check);
        assert_eq!(kind(&["test", "--lib", "--", "name"]), Kind::Test);
        assert_eq!(kind(&["+nightly", "nextest", "run"]), Kind::Test);
        assert_eq!(kind(&["clippy", "--all-targets"]), Kind::Clippy);
        assert_eq!(kind(&["run", "--release"]), Kind::Run);
        assert_eq!(kind(&["build", "-p", "a"]), Kind::Build);
        assert_eq!(kind(&["build", "--profile", "selfdev"]), Kind::Build);
        for release in [
            &["build", "--release"][..],
            &["b", "-r"],
            &["build", "--profile", "release"],
            &["build", "--profile=release"],
        ] {
            assert_eq!(kind(release), Kind::Release, "{release:?}");
        }
        // Flags after `--` belong to the program, and a test stays a test.
        assert_eq!(kind(&["build", "--", "--release"]), Kind::Build);
        assert_eq!(kind(&["test", "--release"]), Kind::Test);
        assert_eq!(kind(&["doc"]), Kind::Other);
        assert_eq!(kind(&[]), Kind::Other);
    }

    #[test]
    fn kind_place_and_search_filters_combine_and_clear() {
        let mut s = State::default();
        s.snapshot.runs = vec![
            run_with("a", &["check", "-p", "ui"], "remote", 0),
            run_with("b", &["test", "-p", "ui", "--lib"], "hosted", 101),
            run_with("c", &["build", "--release"], "local", 0),
            run_with("d", &["test", "-p", "core"], "local", 0),
            run_with("e", &[], "unknown", 0),
        ];
        let ids = |s: &State| {
            let mut ids: Vec<_> = s.rows().iter().map(|r| r.id.clone()).collect();
            ids.sort();
            ids.join("")
        };
        assert!(!s.filters_active());
        assert_eq!(ids(&s), "abcde");
        assert_eq!(s.kind_counts(), [1, 2, 0, 1, 0, 0, 1]);

        s.kind = Some(Kind::Test);
        assert_eq!(ids(&s), "bd");
        s.place = Place::Remote;
        assert_eq!(ids(&s), "b");
        // Counts ignore the kind filter but respect the others.
        assert_eq!(s.kind_counts(), [1, 1, 0, 0, 0, 0, 0]);
        s.place = Place::Local;
        assert_eq!(ids(&s), "d");
        s.place = Place::Any;
        s.kind = None;
        // Case-insensitive substring of the command ("ui" alone is also in "build").
        s.query = "-P UI".into();
        assert_eq!(ids(&s), "ab");
        s.outcome = Outcome::Failed;
        assert_eq!(ids(&s), "b");
        assert!(s.filters_active());
        s.clear_filters();
        assert!(!s.filters_active());
        assert_eq!(ids(&s), "abcde");
    }

    #[test]
    fn kind_cycles_through_all_and_back() {
        let mut s = State::default();
        let mut seen = Vec::new();
        for _ in 0..=Kind::ALL.len() {
            s.cycle_kind(true);
            seen.push(s.kind);
        }
        assert_eq!(seen[0], Some(Kind::Check));
        assert_eq!(seen[Kind::ALL.len() - 1], Some(Kind::Other));
        assert_eq!(seen[Kind::ALL.len()], None);
        s.cycle_kind(false);
        assert_eq!(s.kind, Some(Kind::Other));
        assert_eq!(Place::Any.next().next().next(), Place::Any);
    }

    #[test]
    fn interactive_ui_requires_explicit_namespace() {
        use clap::Parser;
        assert!(matches!(
            crate::Cli::try_parse_from(["justrust", "interactive", "dashboard"])
                .unwrap()
                .command,
            crate::Command::Interactive {
                command: crate::InteractiveCommand::Dashboard
            }
        ));
        for args in [
            vec!["justrust", "dashboard"],
            vec!["justrust", "interactive"],
            vec!["justrust", "--interactive", "dashboard"],
        ] {
            assert!(crate::Cli::try_parse_from(args).is_err());
        }
        assert!(matches!(
            crate::Cli::try_parse_from(["justrust", "status", "--json"])
                .unwrap()
                .command,
            crate::Command::Status { json: true, .. }
        ));
    }

    #[test]
    fn local_record_wins_over_later_cloud_upload_and_partial_history_failure() {
        let mut s = State::default();
        s.snapshot.runs.push(row("same", 1.0));
        let mut uploaded = row("same", 10.0);
        uploaded.origin = "hosted".into();
        uploaded.args.clear();
        s.hosted_runs.push(uploaded);
        assert_eq!(s.rows().len(), 1);
        assert_eq!(s.rows()[0].start, 1.0);
        let (tx, rx) = mpsc::channel();
        tx.send(Update::Hosted(Ok(data::HostedSnapshot {
            history_error: Some("records offline".into()),
            ..Default::default()
        })))
        .unwrap();
        apply_updates(&mut s, &rx);
        assert!(s.snapshot.account.is_some());
        assert_eq!(s.hosted_runs.len(), 1);
        assert!(s.snapshot.hosted_at.is_none());
    }

    #[test]
    fn stale_detail_and_hosted_errors_preserve_last_good_data() {
        let (tx, rx) = mpsc::channel();
        let mut s = State {
            detail_request: 2,
            detail: Some("current".into()),
            ..Default::default()
        };
        s.hosted_runs.push(row("cloud", 1.0));
        tx.send(Update::Detail(1, Ok("stale".into()))).unwrap();
        tx.send(Update::Hosted(Err(anyhow::anyhow!("offline"))))
            .unwrap();
        apply_updates(&mut s, &rx);
        assert_eq!(s.detail.as_deref(), Some("current"));
        assert_eq!(s.hosted_runs.len(), 1);
        assert_eq!(s.snapshot.hosted_error.as_deref(), Some("offline"));
    }
}
