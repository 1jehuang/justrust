//! Follow atomic replacements of the executable that launched this dashboard.
//! Never look up a replacement on PATH or choose a download here.
use super::*;
use std::{
    os::unix::{fs::MetadataExt, process::CommandExt},
    path::PathBuf,
    process::Command,
};

const NAV: &str = "JUSTRUST_DASHBOARD_RESUME_V1";
type Navigation = (
    Option<String>,
    usize,
    Option<String>,
    Option<Kind>,
    Place,
    Outcome,
    String,
);

pub(super) fn resume(s: &mut State) {
    if let Ok(value) = std::env::var(NAV)
        && let Ok(nav) = serde_json::from_str::<Navigation>(&value)
    {
        (
            s.selected_id,
            s.selected,
            s.project,
            s.kind,
            s.place,
            s.outcome,
            s.query,
        ) = nav;
    }
}

pub(super) struct Watch {
    path: PathBuf,
    identity: (u64, u64),
    checked: Instant,
}
impl Watch {
    pub(super) fn current() -> Option<Self> {
        let path = std::env::current_exe().ok()?;
        let m = std::fs::metadata(&path).ok()?;
        Some(Self {
            path,
            identity: (m.dev(), m.ino()),
            checked: Instant::now(),
        })
    }
    pub(super) fn replacement(&mut self) -> Option<&std::path::Path> {
        if self.checked.elapsed() < Duration::from_secs(2) {
            return None;
        }
        self.checked = Instant::now();
        let m = std::fs::metadata(&self.path).ok()?;
        (m.is_file() && (m.dev(), m.ino()) != self.identity).then_some(&self.path)
    }
}

pub(super) fn restart(path: &std::path::Path, s: &State) -> std::io::Error {
    let nav = (
        &s.selected_id,
        s.selected,
        &s.project,
        s.kind,
        s.place,
        s.outcome,
        &s.query,
    );
    Command::new(path)
        .args(["interactive", "dashboard"])
        .env(NAV, serde_json::to_string(&nav).unwrap_or_default())
        .exec()
}
