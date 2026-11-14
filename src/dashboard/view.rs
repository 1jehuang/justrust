//! Jcode-style cards: `╭─ Title · note ── ● status ─╮` borders, two cells of
//! padding, label/value rows, `▰▱` gauges, and one accent color. Text is
//! clipped by display cell, never wrapped. All server/log text is untrusted.
use super::data::{Currency, RunRow, Versions};
use super::{CompileProfile, Kind, MenuRow, Profile, State};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// The palette of Jcode's usage cards.
const LABEL: Color = Color::Rgb(140, 140, 150);
const VALUE: Color = Color::Rgb(215, 215, 225);
const DIM: Color = Color::Rgb(110, 110, 125);
const GOOD: Color = Color::Rgb(100, 200, 100);
const WARN: Color = Color::Rgb(255, 200, 100);
const BAD: Color = Color::Rgb(255, 100, 100);
/// Failures that are on justrust, not on the project being built.
const OURS: Color = Color::Rgb(255, 140, 60);
const ACCENT: Color = Color::Rgb(140, 180, 255);
const BORDER: Color = Color::Rgb(120, 140, 190);
const TRACK: Color = Color::Rgb(50, 50, 60);
const SELECTED: Color = Color::Rgb(45, 50, 70);
/// Every other run row, so neighbors read as separate entries.
const BAND: Color = Color::Rgb(28, 30, 40);
/// Widest a gauge grows, as in Jcode's cards.
const GAUGE: usize = 16;

fn clean(text: &str) -> String {
    crate::record::strip_ansi(text)
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn clip(text: &str, width: usize) -> String {
    let text = clean(text);
    if UnicodeWidthStr::width(text.as_str()) <= width {
        return text;
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let cells = UnicodeWidthChar::width(c).unwrap_or(0);
        if used + cells > width - 1 {
            break;
        }
        out.push(c);
        used += cells;
    }
    out.push('…');
    out
}

fn span(text: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(color))
}

fn strong(text: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(
        text.into(),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn line(text: impl AsRef<str>, color: Color, width: usize) -> Line<'static> {
    Line::from(span(clip(text.as_ref(), width), color))
}

/// Styled spans cut to `width` display cells, never wrapped.
fn fit(spans: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let mut out = Vec::with_capacity(spans.len());
    let mut used = 0;
    for s in spans {
        let w = s.width();
        if used + w <= width {
            used += w;
            out.push(s);
            continue;
        }
        let cut = clip(&s.content, width - used);
        if !cut.is_empty() {
            out.push(Span::styled(cut, s.style));
        }
        break;
    }
    Line::from(out)
}

fn pad(text: &str, width: usize) -> String {
    let text = clip(text, width);
    let n = UnicodeWidthStr::width(text.as_str());
    format!("{text}{}", " ".repeat(width.saturating_sub(n)))
}

fn pad_left(text: &str, width: usize) -> String {
    let text = clip(text, width);
    let n = UnicodeWidthStr::width(text.as_str());
    format!("{}{text}", " ".repeat(width.saturating_sub(n)))
}

/// `1234567` as `1,234,567`.
fn count(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// How long until an RFC 3339 instant, as `in 17d`, `in 5h` or `in 12m`.
/// Anything unparseable is shown as given.
fn until(iso: &str, now: f64) -> String {
    let Ok(at) = chrono::DateTime::parse_from_rfc3339(iso) else {
        return clean(iso);
    };
    let secs = (at.timestamp() as f64 - now).max(0.0);
    if secs >= 2.0 * 86400.0 {
        format!("in {}d", (secs / 86400.0) as u64)
    } else if secs >= 3600.0 {
        format!("in {}h", (secs / 3600.0) as u64)
    } else if secs >= 60.0 {
        format!("in {}m", (secs / 60.0) as u64)
    } else {
        "now".into()
    }
}

/// A path with the home directory written as `~`.
fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_owned(),
    }
}

/// A Jcode card: `╭─ Title · note ───── ● status ─╮`, two cells of padding,
/// and an optional note on the bottom border.
#[derive(Default)]
struct Card {
    title: String,
    note: String,
    status: Vec<Span<'static>>,
    footer: String,
}

impl Card {
    fn new(title: &str) -> Self {
        Self {
            title: title.into(),
            ..Default::default()
        }
    }

    fn note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }

    fn status(mut self, status: Vec<Span<'static>>) -> Self {
        self.status = status;
        self
    }

    fn footer(mut self, footer: impl Into<String>) -> Self {
        self.footer = footer.into();
        self
    }

    fn block(&self, width: u16) -> Block<'static> {
        let rule = Style::default().fg(BORDER);
        let avail = width.saturating_sub(2) as usize;
        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(rule)
            .padding(Padding::horizontal(if width >= 40 { 2 } else { 0 }));
        let status_width: usize = self.status.iter().map(Span::width).sum();
        // The status needs " status ─" plus one rule cell before it, and the
        // title keeps at least twelve cells. Otherwise the status is dropped.
        let right = if status_width > 0 && status_width + 4 + 12 <= avail {
            status_width + 4
        } else {
            0
        };
        let room = avail.saturating_sub(right + 3);
        if room >= 3 {
            let title = clip(&self.title, room);
            let used = UnicodeWidthStr::width(title.as_str());
            let mut spans = vec![Span::styled("─ ", rule), strong(title, Color::White)];
            if !self.note.is_empty() && used + 6 <= room {
                spans.push(span(clip(&format!(" · {}", self.note), room - used), LABEL));
            }
            spans.push(Span::raw(" "));
            block = block.title_top(Line::from(spans));
        }
        if right > 0 {
            let mut spans = vec![Span::raw(" ")];
            spans.extend(self.status.iter().cloned());
            spans.push(Span::raw(" "));
            spans.push(Span::styled("─", rule));
            block = block.title_top(Line::from(spans).right_aligned());
        }
        if !self.footer.is_empty() && avail >= 8 {
            block = block.title_bottom(Line::from(vec![
                Span::styled("─ ", rule),
                span(clip(&self.footer, avail - 3), DIM),
                Span::raw(" "),
            ]));
        }
        block
    }

    /// Draw the frame and return the padded area inside it.
    fn frame(&self, f: &mut Frame, area: Rect) -> Rect {
        let block = self.block(area.width);
        let inner = block.inner(area);
        f.render_widget(block, area);
        inner
    }

    fn draw(&self, f: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
        let inner = self.frame(f, area);
        f.render_widget(Paragraph::new(lines), inner);
    }
}

/// Cells inside a card of this outer width.
fn inner_width(width: u16) -> usize {
    width.saturating_sub(if width >= 40 { 6 } else { 2 }) as usize
}

/// A `key  value` row with the key in the label color.
fn detail(key: &str, mut value: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let mut spans = vec![span(format!("{key:<9}"), LABEL)];
    spans.append(&mut value);
    fit(spans, width)
}

/// The monthly allowance as a used-filled gauge with the remaining share.
fn usage(used: i64, total: i64, resets: Option<&str>, width: usize) -> Line<'static> {
    if total <= 0 {
        return detail("Builds", vec![span("allowance not reported", LABEL)], width);
    }
    let fraction = (used.max(0) as f64 / total as f64).clamp(0.0, 1.0);
    let left = ((1.0 - fraction) * 100.0).round() as u8;
    let color = if left <= 20 {
        BAD
    } else if left <= 50 {
        WARN
    } else {
        GOOD
    };
    let cells = width.saturating_sub(9 + 10).min(GAUGE);
    let mut value = Vec::new();
    if cells >= 4 {
        let filled = (fraction * cells as f64).round() as usize;
        value.push(span("▰".repeat(filled), color));
        value.push(span("▱".repeat(cells - filled), TRACK));
        value.push(span(format!(" {left:>3}% left"), color));
    } else {
        value.push(span(format!("{left}% left"), color));
    }
    if let Some(resets) = resets {
        value.push(span(
            format!("  resets {}", until(resets, crate::paths::now())),
            DIM,
        ));
    }
    detail("Builds", value, width)
}

fn account_lines(state: &State, width: usize) -> Vec<Line<'static>> {
    let s = &state.snapshot;
    let mut lines = Vec::new();
    if let Some(me) = &s.account {
        lines.push(detail(
            "Account",
            vec![
                span(clean(me.email.as_deref().unwrap_or("signed in")), VALUE),
                span(
                    format!("  {}", clean(me.tier.as_deref().unwrap_or("account"))),
                    LABEL,
                ),
            ],
            width,
        ));
        if let Some(builds) = &me.builds {
            let used = builds.included_used.unwrap_or(0);
            let total = builds.included.unwrap_or(0);
            lines.push(usage(used, total, builds.resets_at.as_deref(), width));
            // The plain count says it all while builds remain. Otherwise the
            // account's own wording explains trial, overage or a block.
            let value = if total > 0 && used < total && builds.can_build {
                vec![
                    span(format!("{} of {}", count(used), count(total)), VALUE),
                    span(" builds this month", LABEL),
                ]
            } else {
                vec![span(
                    clean(&builds.describe()),
                    if builds.can_build { VALUE } else { WARN },
                )]
            };
            lines.push(detail("Used", value, width));
        } else {
            lines.push(detail(
                "Builds",
                vec![span("allowance not reported", LABEL)],
                width,
            ));
        }
    } else {
        lines.push(line(
            "Your local recordings are available without signing in.",
            LABEL,
            width,
        ));
        lines.push(line(
            "For account history and usage: justrust login --no-wait",
            LABEL,
            width,
        ));
    }
    for (what, error) in [("Hosted", &s.hosted_error), ("Local", &s.local_error)] {
        if let Some(error) = error {
            lines.push(fit(
                vec![
                    span("▲ ", WARN),
                    span(format!("{what}: {}", clean(error)), WARN),
                ],
                width,
            ));
        }
    }
    lines
}

fn account(f: &mut Frame, area: Rect, state: &State, lines: Vec<Line<'static>>) {
    let s = &state.snapshot;
    let status = if s.hosted_error.is_some() {
        vec![span("● ", BAD), strong("offline", BAD)]
    } else if state.refreshing {
        vec![span("◌ ", ACCENT), strong("refreshing", ACCENT)]
    } else if s.hosted_at.is_some() {
        vec![span("● ", GOOD), strong("connected", GOOD)]
    } else {
        vec![span("◌ ", ACCENT), strong("connecting", ACCENT)]
    };
    let footer = s
        .hosted_at
        .map(|at| {
            format!(
                "updated {} ago",
                crate::status::dur((crate::paths::now() - at).max(0.0))
            )
        })
        .unwrap_or_default();
    Card::new("Justrust Personal Dashboard")
        .note("Your account")
        .status(status)
        .footer(footer)
        .draw(f, area, lines);
}

/// How long ago an RFC 3339 instant was, `None` when unparseable.
fn since(iso: &str, now: f64) -> Option<String> {
    let at = chrono::DateTime::parse_from_rfc3339(iso).ok()?;
    let secs = (now - at.timestamp() as f64).max(0.0);
    Some(if secs >= 2.0 * 86400.0 {
        format!("{}d ago", (secs / 86400.0) as u64)
    } else if secs >= 3600.0 {
        format!("{}h ago", (secs / 3600.0) as u64)
    } else if secs >= 60.0 {
        format!("{}m ago", (secs / 60.0) as u64)
    } else {
        "just now".into()
    })
}

/// Client, service and compiler, one row each, with whatever disagrees
/// picked out in the warning color.
fn version_lines(v: &Versions, width: usize) -> Vec<Line<'static>> {
    let mut client = vec![
        span(clean(&v.client), VALUE),
        span(
            if v.release_build {
                "  published release"
            } else {
                "  source build"
            },
            if v.release_build { LABEL } else { WARN },
        ),
    ];
    match v.currency() {
        Currency::Current => client.push(span("  ● latest", GOOD)),
        Currency::Behind(latest) => client.push(span(
            format!("  ▲ {} available: justrust update", clean(&latest)),
            WARN,
        )),
        Currency::Ahead => client.push(span("  newer than the published release", ACCENT)),
        Currency::Unknown => {}
    }
    let mut lines = vec![detail("Client", client, width)];

    let mut service = vec![span(clean(&v.service), VALUE)];
    match &v.live {
        Some(live) => {
            let deployed = live
                .deployed_at
                .as_deref()
                .and_then(|at| since(at, crate::paths::now()));
            if let Some(deployed) = deployed {
                service.push(span(format!("  deployed {deployed}"), LABEL));
            }
            if let Some(id) = &live.deployment {
                service.push(span(
                    format!("  {}", clean(id.split('-').next().unwrap_or(id))),
                    DIM,
                ));
            }
            if live.held {
                service.push(span("  rollout held", WARN));
            }
            if !live.auto_update {
                service.push(span("  auto-update off", WARN));
            }
        }
        None if v.error.is_none() => service.push(span("  checking", DIM)),
        None => {}
    }
    if let Some(error) = &v.error {
        service.push(span(format!("  ▲ {}", clean(error)), WARN));
    }
    lines.push(detail("Service", service, width));

    let live = v.live.as_ref();
    let mut compiler = match live.and_then(|l| l.toolchain.as_deref()) {
        Some(toolchain) => vec![span(clean(toolchain), VALUE)],
        None => vec![span("not reported", LABEL)],
    };
    if let Some(protocol) = live.and_then(|l| l.protocol) {
        compiler.push(match v.drives_toolchain() {
            Some(false) => span(format!("  ▲ protocol {protocol} needs a newer client"), BAD),
            _ => span(format!("  protocol {protocol}"), LABEL),
        });
    }
    if live.is_some_and(|l| l.toolchain.is_some() && !l.published) {
        compiler.push(span("  ▲ not on the toolchain store", WARN));
    }
    // The build machine installs whatever the service names on its next
    // build, so it is only worth a mention while it has something else: a
    // rollout in progress or a stuck machine. `v` lists everything on it.
    if let (Some(wanted), Some(has)) = (
        live.and_then(|l| l.toolchain.as_deref()),
        v.machine("toolchain"),
    ) && wanted != has
    {
        compiler.push(span(
            format!("  ▲ build machine still has {}", clean(has)),
            WARN,
        ));
    }
    lines.push(detail("Compiler", compiler, width));
    if let Some(rustc) = v.machine("rustc") {
        lines.push(detail("Rustc", vec![span(clean(rustc), VALUE)], width));
    }
    let inventory = if v.machine.is_empty() {
        v.machine_note
            .as_deref()
            .unwrap_or("waiting for build machine versions")
            .to_owned()
    } else {
        format!("{} components reported", v.machine.len())
    };
    lines.push(detail(
        "Machine",
        vec![span(clean(&inventory), LABEL)],
        width,
    ));
    lines.push(line("v  full component inventory", ACCENT, width));
    lines
}

fn versions_card(v: &Versions) -> Card {
    let status = match (v.currency(), v.drives_toolchain()) {
        (_, Some(false)) => vec![span("▲ ", BAD), strong("update required", BAD)],
        (Currency::Behind(_), _) => vec![span("▲ ", WARN), strong("update available", WARN)],
        (Currency::Current, _) => vec![span("● ", GOOD), strong("up to date", GOOD)],
        (Currency::Ahead, _) => vec![span("● ", ACCENT), strong("ahead of release", ACCENT)],
        (Currency::Unknown, _) if v.error.is_some() => {
            vec![span("● ", BAD), strong("unreachable", BAD)]
        }
        (Currency::Unknown, _) => vec![span("◌ ", ACCENT), strong("checking", ACCENT)],
    };
    let footer =
        v.at.map(|at| {
            format!(
                "checked {} ago",
                crate::status::dur((crate::paths::now() - at).max(0.0))
            )
        })
        .unwrap_or_default();
    Card::new("Versions").status(status).footer(footer)
}

/// The kind filter as a row of chips with the run count behind each choice.
fn kind_bar(state: &State, width: usize) -> Line<'static> {
    let counts = state.kind_counts();
    let mut chips = vec![(None, counts.iter().sum::<usize>())];
    chips.extend(Kind::ALL.iter().map(|k| (Some(*k), counts[*k as usize])));
    let mut spans = Vec::new();
    let mut used = 0;
    for (kind, n) in chips {
        let chosen = kind == state.kind;
        // Empty kinds stay out of the way unless they are the active filter.
        if n == 0 && !chosen && kind.is_some() {
            continue;
        }
        let label = kind.map_or("all", Kind::label);
        let cost = label.len() + n.to_string().len() + 5;
        if used + cost > width {
            break;
        }
        used += cost;
        if chosen {
            spans.push(Span::styled(
                format!(" {label} {n} "),
                Style::default()
                    .fg(Color::Black)
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(span(format!(" {label} "), VALUE));
            spans.push(span(format!("{n} "), DIM));
        }
        spans.push(Span::raw("  "));
    }
    Line::from(spans)
}

/// What happened, in words. The cause comes only from the class the recorder
/// stored. An exit code is shown when nothing better is known.
fn result_label(run: &RunRow) -> (String, Color) {
    use crate::failure::Class;
    let Some(code) = run.exit else {
        return ("not loaded".into(), DIM);
    };
    if code == 0 {
        return ("ok".into(), GOOD);
    }
    match run.failure {
        Some(Class::UserCode) if run.compile_failed => ("compile error".into(), BAD),
        Some(Class::UserCode) if Kind::of(&run.args) == Kind::Test => ("tests failed".into(), BAD),
        Some(Class::UserCode) => ("your code failed".into(), BAD),
        Some(Class::Toolchain) => ("toolchain fault".into(), OURS),
        Some(Class::Infra) => ("infra fault".into(), OURS),
        Some(Class::Interrupted) => ("interrupted".into(), WARN),
        Some(Class::Unknown) => ("unknown cause".into(), OURS),
        None if run.compile_failed => ("compile error".into(), BAD),
        None => (format!("failed, exit {code}"), BAD),
    }
}

/// How long ago a run started. Records without a start time say so.
fn when(start: f64) -> String {
    if start > 0.0 {
        crate::runs::ago(start)
    } else {
        "?".into()
    }
}

/// The active filters for the border, the changed ones picked out.
fn filter_status(state: &State) -> Vec<Span<'static>> {
    let mut parts = vec![
        (
            state
                .project
                .as_deref()
                .map_or("all projects".to_owned(), |p| clean(super::project_name(p))),
            state.project.is_some(),
        ),
        (
            state.outcome.label().to_owned(),
            state.outcome != super::Outcome::Any,
        ),
        (
            state.place.label().to_owned(),
            state.place != super::Place::Any,
        ),
    ];
    if !state.query.is_empty() {
        parts.push((format!("matching \"{}\"", clean(&state.query)), true));
    }
    let mut spans = Vec::new();
    for (i, (text, changed)) in parts.into_iter().enumerate() {
        if i > 0 {
            spans.push(span(" · ", DIM));
        }
        spans.push(if changed {
            strong(text, ACCENT)
        } else {
            span(text, LABEL)
        });
    }
    spans
}

const RESULT_WIDTH: usize = 18;
const WHERE_WIDTH: usize = 7;
/// Builds in progress pinned above the recorded runs, at most.
const ACTIVE_ROWS: usize = 4;

/// How long a wait was, at a glance: quick, ordinary, slow, very slow.
fn wall_color(secs: f64) -> Color {
    match secs {
        s if s < 10.0 => GOOD,
        s if s < 60.0 => VALUE,
        s if s < 300.0 => WARN,
        _ => BAD,
    }
}

/// A run row filled to the card's width on its background.
fn banded(mut l: Line<'static>, width: usize, bg: Option<Color>) -> Line<'static> {
    let Some(bg) = bg else { return l };
    let used = l.width();
    l.spans
        .push(Span::raw(" ".repeat(width.saturating_sub(used))));
    l.style(Style::default().bg(bg))
}

/// A build in progress, in the columns of the recorded runs.
fn active_row(run: &crate::status::Active, wide: bool, command_width: usize) -> Vec<Span<'static>> {
    let mut detail = Vec::new();
    if let Some(e) = &run.estimate {
        detail.push(format!("~{} left", crate::status::dur(e.remaining)));
    }
    let current = run
        .running
        .iter()
        // A hosted build reports the place itself while it has no unit names.
        .filter(|(name, _)| *name != run.place)
        .take(3)
        .map(|(name, _)| clean(name))
        .collect::<Vec<_>>()
        .join(", ");
    if !current.is_empty() {
        detail.push(current);
    }
    let detail = detail.join(" · ");
    let label = clean(&run.label);
    let elapsed = crate::status::dur(run.elapsed);
    let mut spans = vec![span("◌ ", ACCENT)];
    if wide {
        let label = clip(&label, command_width.saturating_sub(2));
        let used = UnicodeWidthStr::width(label.as_str());
        let detail = pad(&detail, command_width.saturating_sub(used + 2));
        spans.extend([
            strong(pad("building", RESULT_WIDTH - 2), ACCENT),
            span(format!(" {}", pad(&run.place, WHERE_WIDTH)), LABEL),
            span(format!(" {elapsed:>7}  "), wall_color(run.elapsed)),
            span(label, VALUE),
            span(format!("  {detail}"), DIM),
            span(pad_left("now", 9), LABEL),
        ]);
    } else {
        spans.extend([
            strong("building", ACCENT),
            span(" · ", LABEL),
            span(elapsed, wall_color(run.elapsed)),
            span(" · ", LABEL),
            span(label, VALUE),
            span(format!("  {detail}"), DIM),
        ]);
    }
    spans
}

fn selected_lines(state: &State, width: usize) -> Vec<Line<'static>> {
    let rows = state.rows();
    let Some(run) = rows.get(state.selected) else {
        return vec![line(
            "No run selected. Move through Runs with ↑/↓.",
            LABEL,
            width,
        )];
    };
    let (result, color) = result_label(run);
    let mut lines = vec![
        fit(
            vec![
                strong(result, color),
                span(
                    format!("  {} wall", crate::status::dur(run.wall)),
                    wall_color(run.wall),
                ),
                span(
                    format!("  {}  {}", clean(&run.place), when(run.start)),
                    LABEL,
                ),
            ],
            width,
        ),
        detail(
            "Command",
            vec![span(
                format!("justrust {}", clean(&run.args.join(" "))),
                VALUE,
            )],
            width,
        ),
        detail("Project", vec![span(tilde(&clean(&run.cwd)), VALUE)], width),
    ];
    if let Some(times) = &run.time {
        let total: f64 = times.iter().sum();
        let mut parts: Vec<_> = super::data::Bucket::ALL
            .iter()
            .map(|b| (b.label(), times[*b as usize]))
            .filter(|(_, secs)| *secs > 0.0)
            .collect();
        parts.sort_by(|a, b| b.1.total_cmp(&a.1));
        let text = parts
            .iter()
            .map(|(name, secs)| {
                format!(
                    "{name} {} ({:.0}%)",
                    crate::status::dur(*secs),
                    if total > 0.0 {
                        secs / total * 100.0
                    } else {
                        0.0
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(" · ");
        lines.push(detail("Timing", vec![span(text, VALUE)], width));
    } else {
        lines.push(detail(
            "Timing",
            vec![span("breakdown not recorded locally", DIM)],
            width,
        ));
    }
    lines
}

fn history(f: &mut Frame, area: Rect, state: &State) {
    let rows = state.rows();
    let active = &state.snapshot.active;
    let note = match active.len() {
        0 => count(rows.len() as i64),
        n => format!("{} · {n} building", count(rows.len() as i64)),
    };
    let inner = Card::new("Runs")
        .note(note)
        .status(filter_status(state))
        .frame(f, area);
    if inner.height == 0 {
        return;
    }
    let width = inner.width as usize;
    let wide = width >= 84;
    // Fixed columns: result, where, wall 7, when 9, plus gaps.
    let command_width = width.saturating_sub(RESULT_WIDTH + WHERE_WIDTH + 7 + 9 + 4);
    let header = if wide {
        format!(
            "  {} {} {:>7}  {}{:>9}",
            pad("RESULT", RESULT_WIDTH - 2),
            pad("WHERE", WHERE_WIDTH),
            "WALL",
            pad("COMMAND", command_width),
            "WHEN"
        )
    } else {
        "  RESULT · WHEN · COMMAND".into()
    };
    let mut lines = vec![
        kind_bar(state, width),
        Line::default(),
        line(header, DIM, width),
    ];
    let body = inner.height.saturating_sub(3) as usize;
    // Builds in progress lead the list and never crowd out the history.
    let pinned = active.len().min(ACTIVE_ROWS).min(body / 2);
    for (i, run) in active.iter().take(pinned).enumerate() {
        let l = fit(active_row(run, wide, command_width), width);
        lines.push(banded(l, width, (i % 2 == 1).then_some(BAND)));
    }
    let visible = body - pinned;
    let start = state.selected.saturating_sub(visible.saturating_sub(1));
    for (i, run) in rows.iter().enumerate().skip(start).take(visible) {
        let (result, color) = result_label(run);
        let (command, project) = if run.args.is_empty() {
            (
                clean(run.machine.as_deref().unwrap_or("account recording")),
                clean(&run.id),
            )
        } else {
            (clean(&run.args.join(" ")), clean(&tilde(&run.cwd)))
        };
        let wall = if run.exit.is_some() {
            crate::status::dur(run.wall)
        } else {
            "?".into()
        };
        let mut spans = vec![span("● ", color)];
        if wide {
            let command = clip(&command, command_width.saturating_sub(2));
            let used = UnicodeWidthStr::width(command.as_str());
            let project = pad(&project, command_width.saturating_sub(used + 2));
            spans.extend([
                span(pad(&result, RESULT_WIDTH - 2), color),
                span(format!(" {}", pad(&run.place, WHERE_WIDTH)), LABEL),
                span(
                    format!(" {wall:>7}  "),
                    if run.exit.is_some() {
                        wall_color(run.wall)
                    } else {
                        DIM
                    },
                ),
                span(command, VALUE),
                span(format!("  {project}"), DIM),
                span(pad_left(&when(run.start), 9), LABEL),
            ]);
        } else {
            spans.extend([
                span(result, color),
                span(format!(" · {} · ", when(run.start)), LABEL),
                span(command, VALUE),
                span(format!("  {project}"), DIM),
            ]);
        }
        let mut l = fit(spans, width);
        if i == state.selected {
            l = banded(l, width, Some(SELECTED));
            l.style = l.style.add_modifier(Modifier::BOLD);
        } else {
            l = banded(l, width, ((pinned + i) % 2 == 1).then_some(BAND));
        }
        lines.push(l);
    }
    if rows.is_empty() && pinned == 0 {
        lines.push(line(
            if state.filters_active() {
                "No runs match these filters. Press x to clear them."
            } else {
                "No recordings yet. Run justrust check, test or build."
            },
            LABEL,
            width,
        ));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

/// Cells of one share gauge.
const SHARE_BAR: usize = 12;
/// Rows the profile card may use.
const PROFILE_ROWS: usize = 5;

/// One `label ▰▰▰▱▱▱  52%  1h08m` gauge per part of the wait, largest first,
/// laid out in as many columns as fit so the card stays short.
fn profile_lines(profile: &Profile, width: usize) -> Vec<Line<'static>> {
    if profile.timed == 0 || profile.total <= 0.0 {
        return vec![line(
            "No timings for these runs. Timings come from runs recorded on this machine.",
            LABEL,
            width,
        )];
    }
    let parts: Vec<_> = profile
        .buckets
        .iter()
        .map(|(bucket, secs)| (bucket.label(), *secs))
        .collect();
    share_lines(&parts, profile.total, width)
}

/// The compile profile: what the compiler's time went to, same gauges.
fn compile_lines(profile: &CompileProfile, width: usize) -> Vec<Line<'static>> {
    if profile.timed == 0 || profile.total <= 0.0 {
        return vec![line(
            "No compiled units for these runs. Units come from runs recorded on this machine.",
            LABEL,
            width,
        )];
    }
    let parts: Vec<_> = profile
        .parts
        .iter()
        .map(|(part, secs)| (part.label(), *secs))
        .collect();
    share_lines(&parts, profile.total, width)
}

fn compile_card(profile: &CompileProfile) -> Card {
    let card = Card::new("Compile profile");
    if profile.timed == 0 {
        return card;
    }
    card.note(format!(
        "{} of compiler time over {} {}",
        crate::status::dur(profile.total),
        profile.timed,
        if profile.timed == 1 { "run" } else { "runs" }
    ))
}

fn share_lines(parts: &[(&'static str, f64)], total: f64, width: usize) -> Vec<Line<'static>> {
    let name = parts
        .iter()
        .map(|(label, _)| label.len())
        .max()
        .unwrap_or(0);
    let gap = 5;
    // label, gap, bar, gap, percent, gap, duration
    let cell = |bar: usize| name + 2 + bar + usize::from(bar > 0) + 4 + 2 + 6;
    let columns_with = |bar: usize| ((width + gap) / (cell(bar) + gap)).clamp(1, 3);
    // A shorter gauge is worth it when it lets a second column fit.
    let bar = if width < cell(SHARE_BAR / 2) {
        0
    } else if width < cell(SHARE_BAR)
        || (columns_with(SHARE_BAR) == 1 && columns_with(SHARE_BAR / 2) > 1)
    {
        SHARE_BAR / 2
    } else {
        SHARE_BAR
    };
    let columns = columns_with(bar);
    let shown = parts.len().min(columns * PROFILE_ROWS);
    let rows = shown.div_ceil(columns);
    let mut lines = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut spans = Vec::new();
        for column in 0..columns {
            // Column-major, so each column reads as a ranked list.
            let Some((label, secs)) = parts[..shown].get(column * rows + row) else {
                continue;
            };
            let share = secs / total;
            let percent = match share * 100.0 {
                p if p < 1.0 => "<1%".to_owned(),
                p => format!("{p:.0}%"),
            };
            if column > 0 {
                spans.push(Span::raw(" ".repeat(gap)));
            }
            spans.push(span(pad(label, name + 2), LABEL));
            if bar > 0 {
                let filled = ((share * bar as f64).round() as usize).min(bar);
                spans.push(span("▰".repeat(filled), ACCENT));
                spans.push(span("▱".repeat(bar - filled), TRACK));
                spans.push(Span::raw(" "));
            }
            spans.push(span(format!("{percent:>4}"), VALUE));
            spans.push(span(
                format!("  {}", pad_left(&crate::status::dur(*secs), 6)),
                DIM,
            ));
        }
        lines.push(fit(spans, width));
    }
    lines
}

fn profile_card(profile: &Profile) -> Card {
    let card = Card::new("Overall profile");
    if profile.timed == 0 {
        return card;
    }
    let card = card.note(format!(
        "{} waited over {} {}",
        crate::status::dur(profile.total),
        profile.timed,
        if profile.timed == 1 { "run" } else { "runs" }
    ));
    if profile.timed < profile.runs {
        card.status(vec![span(
            format!("{} without timings", profile.runs - profile.timed),
            DIM,
        )])
    } else {
        card
    }
}

/// The filter menu, drawn over the dashboard.
fn menu(f: &mut Frame, area: Rect, state: &State, at: usize) {
    let width = area.width.saturating_sub(4).min(60);
    let height = (MenuRow::ALL.len() as u16 + 4).min(area.height);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    let inner_width = inner_width(width);
    let mut lines = vec![Line::default()];
    for (i, row) in MenuRow::ALL.iter().enumerate() {
        let chosen = i == at;
        let value = clean(&state.menu_value(*row));
        let label = format!("{:<8} ", row.label());
        let spans = if chosen && *row == MenuRow::Search {
            vec![
                span("› ", ACCENT),
                span(label, VALUE),
                strong(value, ACCENT),
                span("   (enter to type)", DIM),
            ]
        } else if chosen {
            vec![
                span("› ", ACCENT),
                span(label, VALUE),
                span("‹ ", DIM),
                strong(value, ACCENT),
                span(" ›", DIM),
            ]
        } else {
            vec![Span::raw("  "), span(label, LABEL), span(value, VALUE)]
        };
        let mut l = fit(spans, inner_width);
        if chosen {
            let used = l.width();
            l.spans
                .push(Span::raw(" ".repeat(inner_width.saturating_sub(used))));
            l = l.style(Style::default().bg(SELECTED));
        }
        lines.push(l);
    }
    f.render_widget(Clear, popup);
    Card::new("Filters")
        .footer(format!("{} runs match", count(state.rows().len() as i64)))
        .draw(f, popup, lines);
}

/// Key hints: the key bright, what it does dim.
fn hints(pairs: &[(&str, &str)], width: usize) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (key, what) in pairs {
        spans.push(span(*key, VALUE));
        spans.push(span(format!(" {what}   "), DIM));
    }
    fit(spans, width)
}

pub(super) fn draw(f: &mut Frame, state: &State) {
    let area = f.area();
    if area.height < 8 || area.width < 24 {
        f.render_widget(
            Paragraph::new(line("Enlarge terminal · q quit", WARN, area.width as usize)),
            area,
        );
        return;
    }
    if state.detail.is_some() || state.loading_detail {
        let [body, footer] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
        let text = state.detail.as_deref().unwrap_or("Loading run details…");
        let line_count = text.lines().count();
        let scroll = state
            .detail_scroll
            .min(line_count.saturating_sub(body.height.saturating_sub(2) as usize));
        let width = inner_width(body.width);
        let lines = text
            .lines()
            .skip(scroll)
            .map(|s| line(s, VALUE, width))
            .collect();
        let title = if text.starts_with("Justrust Personal Dashboard · Versions\n") {
            "Versions"
        } else {
            "Run details"
        };
        Card::new(title)
            .note(format!("line {} of {}", scroll + 1, line_count))
            .draw(f, body, lines);
        f.render_widget(
            Paragraph::new(hints(
                &[
                    ("esc", "back"),
                    ("↑↓", "scroll"),
                    ("pgup/pgdn", "page"),
                    ("q", "quit"),
                ],
                footer.width as usize,
            )),
            footer,
        );
        return;
    }
    let width = inner_width(area.width);
    // Cards pair up side by side when there is room, else stack, and short
    // terminals keep the run list instead.
    let beside = area.width >= 110;
    let card_width = if beside {
        inner_width(area.width / 2)
    } else {
        width
    };
    let account_lines = account_lines(state, card_width);
    let version_lines = version_lines(&state.versions, card_width);
    let versions_h = version_lines.len() as u16 + 2;
    let account_h = if beside {
        (account_lines.len() as u16 + 2).max(versions_h)
    } else {
        account_lines.len() as u16 + 2
    };
    // Stacked, the versions take their own rows unless the terminal is short.
    let versions_h = if beside || area.height < 30 {
        0
    } else {
        versions_h
    };
    let profile = state.profile();
    let compile = state.compile_profile();
    let time_lines = profile_lines(&profile, card_width);
    let compile_lines = compile_lines(&compile, card_width);
    let (overall_h, compile_h) = (time_lines.len() as u16 + 2, compile_lines.len() as u16 + 2);
    let (first_h, second_h) = match (beside, area.height) {
        (_, h) if h < 24 => (0, 0),
        (true, _) => (overall_h.max(compile_h), 0),
        (false, h) if h < 40 => (overall_h, 0),
        (false, _) => (overall_h, compile_h),
    };
    let selected = selected_lines(state, width);
    let selected_h = if area.height >= 24 {
        selected.len() as u16 + 2
    } else {
        0
    };
    let [top, versions, first, second, selection, runs, footer] = Layout::vertical([
        Constraint::Length(account_h.min(area.height.saturating_sub(6))),
        Constraint::Length(versions_h),
        Constraint::Length(first_h),
        Constraint::Length(second_h),
        Constraint::Length(selected_h),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);
    if beside {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(top);
        account(f, left, state, account_lines);
        versions_card(&state.versions).draw(f, right, version_lines);
    } else {
        account(f, top, state, account_lines);
        if versions.height > 0 {
            versions_card(&state.versions).draw(f, versions, version_lines);
        }
    }
    if beside && first.height > 0 {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(first);
        profile_card(&profile).draw(f, left, time_lines);
        compile_card(&compile).draw(f, right, compile_lines);
    } else {
        if first.height > 0 {
            profile_card(&profile).draw(f, first, time_lines);
        }
        if second.height > 0 {
            compile_card(&compile).draw(f, second, compile_lines);
        }
    }
    if selection.height > 0 {
        let note = state
            .rows()
            .get(state.selected)
            .map(|r| r.id.clone())
            .unwrap_or_default();
        Card::new("Selected run")
            .note(note)
            .draw(f, selection, selected);
    }
    history(f, runs, state);
    if let Some(at) = state.menu {
        menu(f, area, state, at);
    }
    let footer_width = footer.width as usize;
    let help = if state.searching {
        fit(
            vec![
                span(" search ", LABEL),
                strong(format!("{}▏", clean(&state.query)), ACCENT),
                span(
                    "   enter keep   esc cancel   matches command, project, run ID",
                    DIM,
                ),
            ],
            footer_width,
        )
    } else if state.menu.is_some() {
        hints(
            &[
                ("↑↓", "choose"),
                ("←→", "change"),
                ("/", "search"),
                ("x", "clear all"),
                ("s/esc", "close"),
                ("q", "quit"),
            ],
            footer_width,
        )
    } else {
        hints(
            &[
                ("↑↓", "select"),
                ("enter", "details"),
                ("l", "logs"),
                ("s", "filters"),
                ("/", "search"),
                ("x", "clear"),
                ("a", "account"),
                ("v", "versions"),
                ("r", "refresh"),
                ("q", "quit"),
            ],
            footer_width,
        )
    };
    f.render_widget(Paragraph::new(help), footer);
}

#[cfg(test)]
mod tests {
    use super::super::data::Bucket;
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn chrome_clips_by_unicode_cells_and_removes_terminal_controls() {
        assert_eq!(clip("日本語abc", 5), "日本…");
        assert_eq!(clip("hello", 0), "");
        assert_eq!(clean("\x1b[31mred\x1b[0m\r\u{202e}x"), "red  x");
        for w in 0..20 {
            assert!(UnicodeWidthStr::width(clip("🦀日本 café", w).as_str()) <= w);
        }
    }

    #[test]
    fn usage_fills_used_portion_and_labels_remaining() {
        let text = |l: Line| -> String { l.spans.iter().map(|s| s.content.as_ref()).collect() };
        let soon =
            chrono::DateTime::from_timestamp(crate::paths::now() as i64 + 17 * 86400 + 600, 0)
                .unwrap()
                .to_rfc3339();
        let shown = text(usage(25, 100, Some(&soon), 80));
        assert!(
            shown.contains("▰▰▰▰▱▱▱▱▱▱▱▱▱▱▱▱  75% left  resets in 17d"),
            "{shown}"
        );
        assert!(!text(usage(0, 0, None, 80)).contains('%'));
        // Too narrow for a gauge: the number alone, never wrapped.
        assert_eq!(text(usage(25, 100, None, 20)), "Builds   75% left");
        assert_eq!(count(1234567), "1,234,567");
        assert_eq!(count(21185), "21,185");
        assert_eq!(count(999), "999");
        let at = "2026-12-01T00:00:00.000Z";
        let epoch = 1796083200.0;
        assert_eq!(until(at, epoch - 17.5 * 86400.0), "in 17d");
        assert_eq!(until(at, epoch - 30.0 * 3600.0), "in 30h");
        assert_eq!(until(at, epoch - 5.5 * 3600.0), "in 5h");
        assert_eq!(until(at, epoch - 750.0), "in 12m");
        assert_eq!(until(at, epoch + 5.0), "now");
        assert_eq!(until("not a date", 0.0), "not a date");
    }

    fn versions(client: &str, latest: &str) -> Versions {
        Versions {
            client: client.into(),
            release_build: true,
            service: "build.jcode.sh".into(),
            live: Some(crate::dashboard::data::Live {
                release: Some(latest.into()),
                auto_update: true,
                deployment: Some("f0daab20-248d-4f14".into()),
                deployed_at: chrono::DateTime::from_timestamp(
                    crate::paths::now() as i64 - 3 * 3600 - 60,
                    0,
                )
                .map(|at| at.to_rfc3339()),
                toolchain: Some("jr-ac20de5e7921".into()),
                protocol: Some(1),
                published: true,
                ..Default::default()
            }),
            machine: vec![
                ("toolchain".into(), "jr-ac20de5e7921".into()),
                ("rustc".into(), "rustc 1.99.1 (58cf8f9c3 2026-10-13)".into()),
                ("agent".into(), format!("justrust {client}")),
            ],
            at: Some(crate::paths::now()),
            ..Default::default()
        }
    }

    #[test]
    fn versions_card_says_whether_the_client_is_current() {
        let text = |lines: Vec<Line>| -> String {
            lines
                .iter()
                .map(|l| {
                    l.spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>()
                        + "\n"
                })
                .collect()
        };
        let status = |v: &Versions| -> String {
            versions_card(v)
                .status
                .iter()
                .map(|s| s.content.as_ref())
                .collect()
        };
        let current = versions("0.1.0-alpha.26", "0.1.0-alpha.26");
        let shown = text(version_lines(&current, 120));
        assert!(
            shown.contains("Client   0.1.0-alpha.26  published release  ● latest"),
            "{shown}"
        );
        assert!(
            shown.contains("Service  build.jcode.sh  deployed 3h ago  f0daab20"),
            "{shown}"
        );
        assert!(
            shown.contains("Compiler jr-ac20de5e7921  protocol 1"),
            "{shown}"
        );
        assert!(shown.contains("Rustc    rustc 1.99.1"), "{shown}");
        assert!(shown.contains("Machine  3 components reported"), "{shown}");
        assert!(shown.contains("v  full component inventory"), "{shown}");
        assert_eq!(shown.lines().count(), 6, "{shown}");
        assert!(!shown.contains('▲'), "{shown}");
        assert_eq!(status(&current), "● up to date");

        let mut behind = versions("0.1.0-alpha.25", "0.1.0-alpha.26");
        behind.release_build = false;
        behind.machine[0].1 = "jr-000000000000".into();
        let shown = text(version_lines(&behind, 120));
        assert!(
            shown.contains("source build  ▲ 0.1.0-alpha.26 available: justrust update"),
            "{shown}"
        );
        assert!(
            shown.contains("▲ build machine still has jr-000000000000"),
            "{shown}"
        );
        assert_eq!(status(&behind), "▲ update available");

        // A toolchain this client cannot drive outranks everything else.
        let mut refused = versions("0.1.0-alpha.26", "0.1.0-alpha.26");
        refused.live.as_mut().unwrap().protocol =
            Some(crate::toolchain_policy::SUPPORTED.end() + 1);
        assert!(text(version_lines(&refused, 120)).contains("needs a newer client"));
        assert_eq!(status(&refused), "▲ update required");

        assert_eq!(
            status(&versions("0.1.0-alpha.27", "0.1.0-alpha.26")),
            "● ahead of release"
        );

        // Before the first answer, and when the service cannot be reached.
        let mut unknown = Versions {
            client: "0.1.0-alpha.26".into(),
            service: "build.jcode.sh".into(),
            ..Default::default()
        };
        assert_eq!(status(&unknown), "◌ checking");
        unknown.error = Some("Could not resolve host".into());
        unknown.machine_note = Some("no session open to the build machine".into());
        let shown = text(version_lines(&unknown, 120));
        assert!(shown.contains("▲ Could not resolve host"), "{shown}");
        assert!(shown.contains("Compiler not reported"), "{shown}");
        assert!(shown.contains("no session open"), "{shown}");
        assert_eq!(status(&unknown), "● unreachable");
        // Rows are clipped, never wrapped.
        for l in version_lines(&behind, 30) {
            assert!(l.width() <= 30);
        }
    }

    #[test]
    fn versions_card_sits_beside_the_account_or_below_it() {
        let state = State {
            versions: versions("0.1.0-alpha.26", "0.1.0-alpha.26"),
            ..Default::default()
        };
        let wide = screen(&state, 140, 30);
        let row = wide
            .lines()
            .find(|l| l.contains("Versions"))
            .unwrap_or_else(|| panic!("{wide}"));
        assert!(
            row.contains("Justrust Personal Dashboard") && row.contains("up to date"),
            "{wide}"
        );
        assert!(wide.contains("jr-ac20de5e7921"), "{wide}");
        assert!(wide.contains("v versions"), "{wide}");
        let narrow = screen(&state, 90, 32);
        let rows: Vec<&str> = narrow.lines().collect();
        let account = rows
            .iter()
            .position(|l| l.contains("Your account"))
            .unwrap();
        let card = rows
            .iter()
            .position(|l| l.contains("Versions"))
            .unwrap_or_else(|| panic!("{narrow}"));
        assert!(card > account, "{narrow}");
        // A short terminal keeps the run list.
        assert!(!screen(&state, 90, 26).contains("Versions"));
    }

    #[test]
    fn full_versions_inventory_is_scrollable_and_has_its_own_title() {
        let mut inventory = versions("0.1.0-alpha.26", "0.1.0-alpha.26");
        inventory
            .machine
            .push(("future-component".into(), "future-version".into()));
        let state = State {
            detail: Some(inventory.report()),
            ..Default::default()
        };
        let full = screen(&state, 120, 60);
        assert!(full.contains("╭─ Versions"), "{full}");
        assert!(!full.contains("Run details"), "{full}");
        assert!(full.contains("future-component future-version"), "{full}");
        assert!(full.contains("esc back"), "{full}");
        let narrow = screen(&state, 30, 12);
        assert!(narrow.contains("╭─ Versions"), "{narrow}");
        assert!(narrow.contains('…'), "{narrow}");
    }

    fn screen(state: &State, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| draw(f, state)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<String>()
                    + "\n"
            })
            .collect()
    }

    #[test]
    fn selected_card_follows_highlight_and_sits_above_runs() {
        let mut state = State::default();
        let mut times = [0.0; super::super::data::BUCKETS];
        times[Bucket::Compile as usize] = 8.0;
        times[Bucket::Tests as usize] = 2.0;
        state.snapshot.runs = vec![
            RunRow {
                id: "selected-first".into(),
                start: 2.0,
                args: vec!["test".into()],
                cwd: "/srv/one".into(),
                wall: 10.0,
                exit: Some(0),
                time: Some(times),
                ..Default::default()
            },
            RunRow {
                id: "selected-second".into(),
                start: 1.0,
                args: vec!["check".into()],
                cwd: "/srv/two".into(),
                exit: Some(1),
                ..Default::default()
            },
        ];
        let text = screen(&state, 140, 45);
        assert!(text.contains("compiling 8.0s (80%)"), "{text}");
        assert!(
            text.find("Selected run").unwrap() < text.find("─ Runs").unwrap(),
            "{text}"
        );
        state.selected = 1;
        let lines = selected_lines(&state, 120);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(
            text.contains("justrust check") && text.contains("/srv/two"),
            "{text}"
        );
        assert!(text.contains("breakdown not recorded"), "{text}");
        state.snapshot.runs.clear();
        assert!(
            selected_lines(&state, 100)[0].spans[0]
                .content
                .contains("No run selected")
        );
    }

    #[test]
    fn kind_bar_counts_runs_and_filters_show_in_the_title() {
        let run = |id: &str, args: &[&str]| super::super::data::RunRow {
            id: id.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            exit: Some(0),
            place: "remote".into(),
            origin: "local".into(),
            ..Default::default()
        };
        let mut state = State::default();
        state.snapshot.runs = vec![
            run("r1", &["check"]),
            run("r2", &["test", "--lib"]),
            run("r3", &["test"]),
            run("r4", &["build", "--release"]),
        ];
        let text = screen(&state, 140, 30);
        assert!(text.contains("all 4"), "{text}");
        assert!(text.contains("check 1") && text.contains("test 2"));
        assert!(text.contains("release 1"));
        // Kinds with no runs are not listed.
        assert!(!text.contains("clippy 0"), "{text}");
        assert!(text.contains("anywhere"));

        state.kind = Some(Kind::Clippy);
        state.query = "zzz".into();
        let text = screen(&state, 140, 30);
        assert!(text.contains("clippy 0"), "{text}");
        assert!(text.contains("matching \"zzz\""));
        assert!(text.contains("Press x to clear"));

        state.searching = true;
        assert!(screen(&state, 140, 30).contains("search zzz"));
        // Narrow terminals drop chips instead of wrapping.
        screen(&state, 24, 8);
    }

    #[test]
    fn results_read_as_what_happened_and_runs_show_their_age() {
        use crate::failure::Class;
        let run = |args: &[&str], exit: Option<i32>, failure, compile_failed| RunRow {
            id: "20261113-050528950-2175590".into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            exit,
            failure,
            compile_failed,
            ..Default::default()
        };
        let label = |r: RunRow| result_label(&r).0;
        assert_eq!(label(run(&["check"], Some(0), None, false)), "ok");
        for (args, class, compile_failed, expected) in [
            (&["check"][..], Some(Class::UserCode), true, "compile error"),
            (
                &["test", "--lib"][..],
                Some(Class::UserCode),
                true,
                "compile error",
            ),
            (
                &["test", "--lib"][..],
                Some(Class::UserCode),
                false,
                "tests failed",
            ),
            (
                &["run"][..],
                Some(Class::UserCode),
                false,
                "your code failed",
            ),
            (
                &["build"][..],
                Some(Class::Toolchain),
                true,
                "toolchain fault",
            ),
            (&["build"][..], Some(Class::Infra), false, "infra fault"),
            (
                &["test"][..],
                Some(Class::Interrupted),
                false,
                "interrupted",
            ),
            (&["test"][..], Some(Class::Unknown), true, "unknown cause"),
            (&["check"][..], None, true, "compile error"),
            (&["test"][..], None, false, "failed, exit 101"),
        ] {
            assert_eq!(
                label(run(args, Some(101), class, compile_failed)),
                expected,
                "{args:?} {class:?} {compile_failed}"
            );
        }
        assert_eq!(label(run(&[], None, None, false)), "not loaded");

        assert_eq!(when(0.0), "?");
        let mut state = State::default();
        let mut failed = run(
            &["test", "-p", "ui"],
            Some(101),
            Some(Class::UserCode),
            false,
        );
        failed.start = crate::paths::now() - 300.0;
        failed.place = "remote".into();
        state.snapshot.runs = vec![failed];
        let text = screen(&state, 140, 30);
        assert!(text.contains("tests failed"), "{text}");
        assert!(text.contains("5m ago"), "{text}");
        assert!(text.contains("WHEN"));
        // The wide layout shows the age in place of the run ID.
        let history = text.split("─ Runs").nth(1).unwrap();
        assert!(!history.contains("exit 101") && !history.contains("20261113-050528950"));
    }

    #[test]
    fn profile_card_shows_shares_of_the_wait_and_follows_filters() {
        let timed = |id: &str, args: &[&str], compile: f64, sync: f64| {
            let mut time = [0.0; super::super::data::BUCKETS];
            time[Bucket::Compile as usize] = compile;
            time[Bucket::Sync as usize] = sync;
            RunRow {
                id: id.into(),
                args: args.iter().map(|a| a.to_string()).collect(),
                exit: Some(0),
                time: Some(time),
                ..Default::default()
            }
        };
        let mut state = State::default();
        state.snapshot.runs = vec![
            timed("a", &["check"], 30.0, 10.0),
            timed("b", &["test"], 45.0, 15.0),
        ];
        let text = screen(&state, 140, 40);
        assert!(
            text.contains("Overall profile · 1m40s waited over 2 runs"),
            "{text}"
        );
        // Each part of the wait is a gauge filled to its share, largest first.
        assert!(
            text.contains("compiling    ▰▰▰▰▰▰▰▰▰▱▱▱  75%   1m15s"),
            "{text}"
        );
        assert!(
            text.contains("source sync  ▰▰▰▱▱▱▱▱▱▱▱▱  25%     25s"),
            "{text}"
        );
        // Wide cards put the parts side by side, narrow ones stack them.
        assert_eq!(profile_lines(&state.profile(), 134).len(), 1);
        assert_eq!(profile_lines(&state.profile(), 30).len(), 2);

        state.kind = Some(Kind::Check);
        let text = screen(&state, 140, 40);
        assert!(text.contains("40s waited over 1 run "), "{text}");
        for width in [0, 1, 10, 30] {
            let lines = profile_lines(&state.profile(), width);
            assert!(lines.len() <= PROFILE_ROWS, "{width}");
            assert!(lines.iter().all(|l| l.width() <= width), "{width}");
        }
        state.snapshot.runs.iter_mut().for_each(|r| r.time = None);
        let text = screen(&state, 140, 40);
        assert!(text.contains("No timings for these runs"), "{text}");
        // Short terminals keep the run list and drop the card.
        assert!(!screen(&state, 140, 20).contains("Overall profile"));
    }

    #[test]
    fn compile_profile_has_its_own_card_beside_or_below_the_overall_one() {
        let mut state = State::default();
        state.snapshot.runs = vec![RunRow {
            id: "a".into(),
            args: vec!["test".into()],
            exit: Some(0),
            compile: Some([30.0, 60.0, 0.0, 0.0, 0.0, 10.0]),
            ..Default::default()
        }];
        let text = screen(&state, 160, 40);
        assert!(
            text.contains("Compile profile · 1m40s of compiler time over 1 run "),
            "{text}"
        );
        assert!(text.contains("test targets"), "{text}");
        assert!(text.contains(" 60%"), "{text}");
        assert!(text.contains("workspace crates") && text.contains("cache restores"));
        // Side by side: both titles share a row.
        let titles = text
            .lines()
            .find(|l| l.contains("Overall profile"))
            .unwrap();
        assert!(titles.contains("Compile profile"), "{text}");
        // Narrow and tall: stacked. Narrow and short: the overall one only.
        let text = screen(&state, 100, 40);
        assert!(text.contains("Overall profile") && text.contains("Compile profile"));
        let text = screen(&state, 100, 28);
        assert!(text.contains("Overall profile") && !text.contains("Compile profile"));
        state.snapshot.runs[0].compile = None;
        assert!(screen(&state, 160, 40).contains("No compiled units for these runs"));
    }

    #[test]
    fn builds_in_progress_lead_the_runs_card_and_rows_are_banded() {
        let building = |label: &str, elapsed: f64| crate::status::Active {
            id: label.into(),
            label: label.into(),
            cwd: String::new(),
            args: Vec::new(),
            agent_session: None,
            ncpu: 0,
            cpu_cores: None,
            sampled_at: None,
            place: "hosted".into(),
            reason: None,
            elapsed,
            live: Default::default(),
            units_finished: 0,
            running: vec![("serde".into(), 1.0)],
            estimate: None,
        };
        let done = |id: &str, wall: f64| RunRow {
            id: id.into(),
            args: vec!["check".into()],
            exit: Some(0),
            wall,
            ..Default::default()
        };
        let mut state = State::default();
        state.snapshot.active = vec![building("jcode test", 75.0)];
        state.snapshot.runs = vec![done("a", 2.0), done("b", 30.0), done("c", 400.0)];
        let text = screen(&state, 140, 30);
        assert!(!text.contains("on this client"), "{text}");
        assert!(text.contains("╭─ Runs · 3 · 1 building "), "{text}");
        let row = text.lines().find(|l| l.contains("◌ building")).unwrap();
        assert!(row.contains("hosted") && row.contains("1m15s"), "{row}");
        assert!(row.contains("jcode test  serde") && row.contains("now"));
        state.snapshot.active[0].running = vec![("hosted".into(), 1.0)];
        assert!(!screen(&state, 140, 30).contains("jcode test  hosted"));
        state.snapshot.active[0].running = vec![("serde".into(), 1.0)];
        // The narrow layout keeps it too.
        assert!(screen(&state, 70, 30).contains("building · 1m15s · jcode test"));

        assert_eq!(wall_color(2.0), GOOD);
        assert_eq!(wall_color(30.0), VALUE);
        assert_eq!(wall_color(75.0), WARN);
        assert_eq!(wall_color(400.0), BAD);

        // Row backgrounds alternate, and the selected row has its own.
        let mut terminal = Terminal::new(TestBackend::new(140, 30)).unwrap();
        terminal.draw(|f| draw(f, &state)).unwrap();
        let buffer = terminal.backend().buffer();
        let row_of = |needle: &str| {
            (0..30u16)
                .find(|y| {
                    (0..140u16)
                        .map(|x| buffer[(x, *y)].symbol())
                        .collect::<String>()
                        .contains(needle)
                })
                .unwrap()
        };
        let first = row_of("◌ building");
        let bgs: Vec<_> = (0..4).map(|i| buffer[(60, first + i)].bg).collect();
        assert_eq!(bgs, [Color::Reset, SELECTED, Color::Reset, BAND]);
        // The wall time carries its own color on a recorded row.
        let wall = (0..140u16)
            .find(|x| buffer[(*x, first + 3)].symbol() == "6")
            .unwrap();
        assert_eq!(buffer[(wall, first + 3)].fg, BAD);
    }

    #[test]
    fn filter_menu_lists_every_filter_with_its_current_value() {
        let mut state = State {
            menu: Some(1),
            ..Default::default()
        };
        state.kind = Some(Kind::Test);
        state.project = Some("/home/me/jcode-desktop".into());
        let text = screen(&state, 140, 40);
        assert!(text.contains("Filters"), "{text}");
        assert!(text.contains("Project  jcode-desktop"), "{text}");
        assert!(text.contains("› Command  ‹ test ›"), "{text}");
        assert!(text.contains("Where    anywhere"));
        assert!(text.contains("Result   all results"));
        assert!(text.contains("Search   none"));
        assert!(text.contains("0 runs match"));
        assert!(text.contains("s/esc close"));
        state.menu = Some(4);
        assert!(screen(&state, 140, 40).contains("(enter to type)"));
        // The popup never exceeds a small terminal.
        screen(&state, 24, 8);
    }

    #[test]
    fn every_size_renders_without_wrapped_chrome_or_panics() {
        for (w, h) in [(10, 3), (24, 8), (40, 12), (80, 24), (160, 50)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal.draw(|f| draw(f, &State::default())).unwrap();
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer.area.width, w);
            if w >= 80 {
                let text = format!("{buffer:?}");
                assert!(text.contains("Your account"));
                assert!(text.contains("Runs"));
            }
        }
    }

    #[test]
    fn account_card_has_no_blank_row_and_counts_down_to_the_reset() {
        let mut state = State::default();
        state.snapshot.hosted_at = Some(crate::paths::now());
        let resets =
            chrono::DateTime::from_timestamp(crate::paths::now() as i64 + 5 * 3600 + 600, 0)
                .unwrap()
                .to_rfc3339();
        state.snapshot.account = Some(crate::remote_hosted::Me {
            email: Some("me@example.com".into()),
            tier: Some("flagship".into()),
            status: None,
            build_hosts: true,
            builds: Some(crate::remote_hosted::Builds {
                included: Some(100),
                included_used: Some(25),
                resets_at: Some(resets),
                can_build: true,
                ..Default::default()
            }),
            plan: None,
        });
        // Stacked: beside the taller Versions card the account card pads.
        let text = screen(&state, 100, 45);
        assert!(text.contains("75% left  resets in 5h"), "{text}");
        let rows: Vec<&str> = text.lines().collect();
        let end = rows.iter().position(|r| r.starts_with('╰')).unwrap();
        // Account, Builds, Used, then the bottom border: no empty row.
        assert_eq!(end, 4, "{text}");
        for row in &rows[1..end] {
            assert!(!row.trim_matches(['│', ' ']).is_empty(), "{text}");
        }
    }

    #[test]
    fn cards_have_jcode_borders_and_no_row_reaches_past_the_frame() {
        let mut state = State::default();
        state.snapshot.hosted_at = Some(crate::paths::now());
        state.snapshot.runs = vec![RunRow {
            id: "r1".into(),
            args: vec!["check".into(), "-p".into(), "x".repeat(300)],
            cwd: "/srv/project".into(),
            exit: Some(0),
            place: "remote".into(),
            ..Default::default()
        }];
        for (w, h) in [(40, 12), (60, 30), (100, 30), (150, 45)] {
            let text = screen(&state, w, h);
            for row in text.lines().take(h as usize - 1) {
                let first = row.chars().next().unwrap();
                let last = row.chars().last().unwrap();
                assert!("╭│╰".contains(first), "{w}x{h}: {row}");
                assert!("╮│╯".contains(last), "{w}x{h}: {row}");
            }
            assert!(text.contains("╭─ Justrust Personal"), "{text}");
            assert!(text.contains("╭─ Runs · 1 "), "{text}");
        }
        let text = screen(&state, 150, 45);
        assert!(text.contains("● connected ─╮"), "{text}");
        assert!(text.contains("╰─ updated "), "{text}");
        assert!(text.contains("all projects · all results · anywhere ─╮"));
        // The command reads first and its project follows, dimmer.
        assert!(text.contains("● ok"), "{text}");
        state.snapshot.runs[0].args = vec!["check".into()];
        assert!(screen(&state, 150, 45).contains("check  /srv/project"));
    }
}
