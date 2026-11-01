//! Live progress of a recorded run, for `justrust status` and status bars.
//!
//! The recorder asks cargo for its progress bar even when stderr is not a
//! terminal (`CARGO_TERM_PROGRESS_WHEN=always`). A [`SniffReader`] wraps cargo's
//! stderr. It reads `Building [===>  ] 56/66: a, b, c` segments and cargo's
//! status lines, keeps a [`LiveState`], and writes it to `<run>/live.json`
//! as it changes. When the progress bar was only turned on for us,
//! the reader also removes it from the stream, so agents, logs, and pipes see
//! the same output as before.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const FILE: &str = "live.json";

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct LiveState {
    /// Units cargo has finished, including fresh ones (`N` in `N/M`).
    pub done: usize,
    /// Units in this build (`M` in `N/M`). 0 until cargo shows progress.
    pub total: usize,
    /// Crates rustc is working on right now, as cargo lists them.
    pub active: Vec<String>,
    /// "starting", "waiting for lock", "compiling", "testing", "running".
    pub phase: String,
    /// `Fresh` lines (only printed in verbose mode, which agent runs use).
    pub fresh: usize,
    /// `Compiling`/`Checking`/`Documenting` lines: units that really run rustc.
    pub compiling: usize,
    /// `Running` test binaries and doc-test groups started.
    pub test_binaries: usize,
    pub finished_build: bool,
    /// Units already done at the first progress redraw: roughly the fresh ones.
    #[serde(default)]
    pub first_done: Option<usize>,
    /// When each entry of `active` first appeared (same order).
    #[serde(default)]
    pub active_since: Vec<f64>,
    /// When cargo printed `Finished` (the build part ended).
    #[serde(default)]
    pub finished_at: Option<f64>,
    pub updated: f64,
}

impl LiveState {
    /// Apply one segment of cargo's stderr (a line or a `\r`-ended redraw).
    /// Returns true when the segment is a progress-bar redraw.
    pub fn apply(&mut self, segment: &str) -> bool {
        let text = crate::record::strip_ansi(segment);
        let t = text.trim();
        if t.is_empty() {
            // Cargo pads and clears its bar with blank `\r` segments.
            return !segment.is_empty();
        }
        if let Some(rest) = t.strip_prefix("Building [") {
            if let Some((_, after)) = rest.split_once("] ") {
                let (counts, names) = after.split_once(':').unwrap_or((after, ""));
                if let Some((n, m)) = counts.trim().split_once('/')
                    && let (Ok(n), Ok(m)) = (n.trim().parse(), m.trim().parse())
                {
                    self.first_done.get_or_insert(n);
                    self.done = n;
                    self.total = m;
                }
                let names: Vec<String> = names
                    .split(',')
                    .map(|s| s.trim().trim_end_matches('…').trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect();
                let now = paths::now();
                self.active_since = names
                    .iter()
                    .map(|n| {
                        self.active
                            .iter()
                            .position(|a| a == n)
                            .and_then(|i| self.active_since.get(i).copied())
                            .unwrap_or(now)
                    })
                    .collect();
                self.active = names;
                if !self.finished_build {
                    self.phase = "compiling".into();
                }
            }
            return true;
        }
        let word = t.split_whitespace().next().unwrap_or("");
        match word {
            "Fresh" => self.fresh += 1,
            "Compiling" | "Checking" | "Documenting" => {
                self.compiling += 1;
                self.phase = "compiling".into();
            }
            "Blocking" => self.phase = "waiting for lock".into(),
            "Finished" => {
                self.finished_build = true;
                self.finished_at = Some(paths::now());
                self.active.clear();
                self.active_since.clear();
                self.done = self.total;
                self.phase = "finishing".into();
            }
            "Running" if self.finished_build => {
                if t.starts_with("Running `") {
                    self.phase = "running".into();
                } else {
                    self.test_binaries += 1;
                    self.phase = "testing".into();
                }
            }
            "Doc-tests" => {
                self.test_binaries += 1;
                self.phase = "testing".into();
            }
            _ => {}
        }
        false
    }
}

pub fn load(run_dir: &Path) -> Option<LiveState> {
    serde_json::from_slice(&std::fs::read(run_dir.join(FILE)).ok()?).ok()
}

/// Splits a byte stream into segments ended by `\n` or `\r`, updates the live
/// state, and (when `strip` is set) drops progress-bar segments.
pub struct Sniffer {
    path: Option<PathBuf>,
    strip: bool,
    pending: Vec<u8>,
    pub state: LiveState,
    written: Option<LiveState>,
}

impl Sniffer {
    pub fn new(run_dir: Option<&Path>, strip: bool) -> Self {
        let mut state = LiveState {
            phase: "starting".into(),
            ..Default::default()
        };
        state.updated = paths::now();
        let mut s = Sniffer {
            path: run_dir.map(|d| d.join(FILE)),
            strip,
            pending: Vec::new(),
            state,
            written: None,
        };
        s.write(true);
        s
    }

    /// Feed a chunk; returns the bytes to forward downstream.
    pub fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) {
        if !self.strip {
            out.extend_from_slice(chunk);
        }
        self.pending.extend_from_slice(chunk);
        while let Some(pos) = self.pending.iter().position(|&b| b == b'\n' || b == b'\r') {
            let seg: Vec<u8> = self.pending.drain(..=pos).collect();
            let body = String::from_utf8_lossy(&seg[..seg.len() - 1]).into_owned();
            let progress = self.state.apply(&body);
            // A blank `\r` segment is part of a redraw; a blank `\n` is a real
            // empty line.
            let drop = progress && (seg[seg.len() - 1] == b'\r' || !body.trim().is_empty());
            if self.strip && !drop {
                out.extend_from_slice(&seg);
            }
        }
        self.write(false);
    }

    /// End of stream: forward whatever is left.
    pub fn finish(&mut self, out: &mut Vec<u8>) {
        if self.strip && !self.pending.is_empty() {
            out.append(&mut self.pending);
        }
        self.pending.clear();
        self.write(true);
    }

    fn write(&mut self, force: bool) {
        let Some(path) = &self.path else { return };
        let now = paths::now();
        // Write on every real change: cargo redraws at most a few times a
        // second, and a throttle could swallow the last update for good.
        let mut cur = self.state.clone();
        cur.updated = 0.0;
        cur.active_since.clear();
        if !force && self.written.as_ref() == Some(&cur) {
            return;
        }
        self.state.updated = now;
        let tmp = path.with_extension("json.tmp");
        if let Ok(bytes) = serde_json::to_vec(&self.state)
            && std::fs::write(&tmp, bytes).is_ok()
        {
            let _ = std::fs::rename(&tmp, path);
        }
        self.written = Some(cur);
    }
}

/// `Read` adapter that runs everything through a [`Sniffer`].
pub struct SniffReader<R> {
    inner: R,
    sniffer: Sniffer,
    out: VecDeque<u8>,
    eof: bool,
}

impl<R: Read> SniffReader<R> {
    pub fn new(inner: R, sniffer: Sniffer) -> Self {
        SniffReader {
            inner,
            sniffer,
            out: VecDeque::new(),
            eof: false,
        }
    }
}

impl<R: Read> Read for SniffReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut chunk = vec![0u8; 64 * 1024];
        while self.out.is_empty() && !self.eof {
            let n = match self.inner.read(&mut chunk) {
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            let mut fwd = Vec::new();
            if n == 0 {
                self.eof = true;
                self.sniffer.finish(&mut fwd);
            } else {
                self.sniffer.feed(&chunk[..n], &mut fwd);
            }
            self.out.extend(fwd);
        }
        let n = buf.len().min(self.out.len());
        for (i, b) in self.out.drain(..n).enumerate() {
            buf[i] = b;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &[&[u8]], strip: bool) -> (String, LiveState) {
        let mut s = Sniffer::new(None, strip);
        let mut out = Vec::new();
        for c in input {
            s.feed(c, &mut out);
        }
        s.finish(&mut out);
        (String::from_utf8(out).unwrap(), s.state)
    }

    #[test]
    fn parses_progress_and_strips_it() {
        let (out, st) = run(
            &[
                b"       Fresh libc v0.2\n    Building [===>   ] 56/66: equivalent, toml_dat",
                b"etime, winnow, t\xe2\x80\xa6\r    Building [====> ] 57/66: hashbrown, winnow   \r",
                b"    Checking indexmap v2.14.2\nerror: oops\n\n",
            ],
            true,
        );
        assert_eq!(
            out,
            "       Fresh libc v0.2\n    Checking indexmap v2.14.2\nerror: oops\n\n"
        );
        assert_eq!((st.done, st.total), (57, 66));
        assert_eq!(st.active, vec!["hashbrown", "winnow"]);
        assert_eq!((st.fresh, st.compiling), (1, 1));
        assert_eq!(st.phase, "compiling");
    }

    #[test]
    fn passes_everything_through_when_not_stripping() {
        let input: &[u8] = b"    Building [=> ] 1/2: a\r    Compiling a v1\n";
        let (out, st) = run(&[input], false);
        assert_eq!(out.as_bytes(), input);
        assert_eq!((st.done, st.total, st.compiling), (1, 2, 1));
    }

    #[test]
    fn tracks_test_phase_after_build() {
        let (_, st) = run(
            &[b"    Finished `test` profile\n     Running unittests src/lib.rs (x)\n   Doc-tests foo\n"],
            true,
        );
        assert_eq!(st.phase, "testing");
        assert_eq!(st.test_binaries, 2);
        // Verbose `Running \`rustc ...\`` lines during the build are not tests.
        let (_, st) = run(&[b"     Running `rustc --crate-name a`\n"], true);
        assert_eq!(st.test_binaries, 0);
    }

    #[test]
    fn sniff_reader_round_trips() {
        let data: &[u8] = b"    Building [=> ] 1/3: x\r   Compiling x\nhello";
        let mut r = SniffReader::new(data, Sniffer::new(None, true));
        let mut s = String::new();
        r.read_to_string(&mut s).unwrap();
        assert_eq!(s, "   Compiling x\nhello");
    }
}
