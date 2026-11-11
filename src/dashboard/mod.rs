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

#[derive(Default)]
struct State {
    snapshot: Snapshot,
    hosted_runs: Vec<RunRow>,
    selected: usize,
    selected_id: Option<String>,
    here: bool,
    failed: bool,
    detail: Option<String>,
    detail_scroll: usize,
    loading_detail: bool,
    detail_request: u64,
    refreshing: bool,
    cwd: String,
}

impl State {
    fn rows(&self) -> Vec<&RunRow> {
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
            .filter(|r| !self.here || std::path::Path::new(&r.cwd).starts_with(&self.cwd))
            .filter(|r| !self.failed || r.exit.is_some_and(|code| code != 0))
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
        loop {
            if local_tx
                .send(Update::Local(data::local_snapshot()))
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
        if key.code == KeyCode::Char('q')
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            break;
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
            KeyCode::Char('f') => {
                state.failed = !state.failed;
                state.selected_id = None;
            }
            KeyCode::Char('h') => {
                state.here = !state.here;
                state.selected_id = None;
            }
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
            machine: None,
            origin: "local".into(),
        }
    }

    #[test]
    fn selection_tracks_id_across_refresh_and_filters_use_path_boundaries() {
        let mut s = State {
            cwd: "/work/pro".into(),
            ..Default::default()
        };
        s.snapshot.runs = vec![row("older", 1.0), row("newer", 2.0)];
        s.selected_id = Some("older".into());
        s.reconcile_selection();
        assert_eq!(s.selected, 1);
        s.snapshot.runs.push(row("newest", 3.0));
        s.reconcile_selection();
        assert_eq!(s.selected, 2);
        s.here = true;
        assert!(s.rows().is_empty());
        s.cwd = "/work".into();
        assert_eq!(s.rows().len(), 3);
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
