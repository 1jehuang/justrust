//! Jcode-style chrome: one row per label, display-cell clipping, used-filled
//! usage bars and remaining percentages. All server/log text is untrusted.
use super::data::{Bucket, RunRow};
use super::{Kind, MenuRow, Profile, State};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const MUTED: Color = Color::Rgb(140, 140, 140);
const FAINT: Color = Color::Rgb(120, 120, 120);
const GOOD: Color = Color::Rgb(100, 200, 100);
const WARN: Color = Color::Rgb(255, 200, 100);
const BAD: Color = Color::Rgb(255, 100, 100);
/// Failures that are on justrust, not on the project being built.
const OURS: Color = Color::Rgb(255, 140, 60);
const ACCENT: Color = Color::Rgb(186, 139, 255);
const BORDER: Color = Color::Rgb(100, 100, 110);

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

fn line(text: impl AsRef<str>, color: Color, width: usize) -> Line<'static> {
    Line::from(Span::styled(
        clip(text.as_ref(), width),
        Style::default().fg(color),
    ))
}

fn block(title: &str, width: u16) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Line::from(Span::styled(
            format!(" {} ", clip(title, width.saturating_sub(4) as usize)),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )))
}

fn card(f: &mut Frame, area: Rect, title: &str, lines: Vec<Line<'static>>) {
    let border = block(title, area.width);
    let inner = border.inner(area);
    f.render_widget(border, area);
    f.render_widget(Paragraph::new(lines), inner);
}

fn usage(used: i64, total: i64, width: usize) -> Line<'static> {
    if total <= 0 {
        return line("Build allowance not reported", MUTED, width);
    }
    let used = used.max(0);
    let fraction = (used as f64 / total as f64).clamp(0.0, 1.0);
    let left = ((1.0 - fraction) * 100.0).round() as u8;
    let color = if left < 10 {
        BAD
    } else if left < 25 {
        WARN
    } else {
        GOOD
    };
    let label = format!("Monthly builds {used}/{total} ");
    let suffix = format!(" {left}% left");
    let cells = width.saturating_sub(label.len() + suffix.len()).min(12);
    if cells < 3 {
        return line(format!("{label}{left}% left"), color, width);
    }
    let filled = (fraction * cells as f64).round() as usize;
    Line::from(vec![
        Span::styled(label, Style::default().fg(MUTED)),
        Span::styled("▰".repeat(filled), Style::default().fg(color)),
        Span::styled(
            "▱".repeat(cells - filled),
            Style::default().fg(Color::Rgb(50, 50, 60)),
        ),
        Span::styled(suffix, Style::default().fg(color)),
    ])
}

fn account(f: &mut Frame, area: Rect, state: &State) {
    let s = &state.snapshot;
    let width = area.width.saturating_sub(2) as usize;
    let freshness = s
        .hosted_at
        .map(|at| {
            format!(
                "updated {} ago",
                crate::status::dur((crate::paths::now() - at).max(0.0))
            )
        })
        .unwrap_or_else(|| "connecting".into());
    let status = if s.hosted_error.is_some() {
        "● offline"
    } else if state.refreshing {
        "refreshing"
    } else {
        "● connected"
    };
    let title = format!("justrust · Your account · {status} · {freshness}");
    let mut lines = Vec::new();
    if let Some(me) = &s.account {
        lines.push(line(
            format!(
                "{} · {}",
                me.email.as_deref().unwrap_or("Signed in"),
                me.tier.as_deref().unwrap_or("account")
            ),
            Color::Reset,
            width,
        ));
        if let Some(builds) = &me.builds {
            lines.push(usage(
                builds.included_used.unwrap_or(0),
                builds.included.unwrap_or(0),
                width,
            ));
            lines.push(line(
                builds.describe(),
                if builds.can_build { GOOD } else { WARN },
                width,
            ));
            if let Some(reset) = &builds.resets_at {
                lines.push(line(format!("Resets {reset}"), MUTED, width));
            }
        } else {
            lines.push(line(
                "Account did not report a build allowance",
                MUTED,
                width,
            ));
        }
    } else {
        lines.push(line(
            "Your local recordings are available without signing in.",
            MUTED,
            width,
        ));
        lines.push(line(
            "For account history and usage: justrust login --no-wait",
            MUTED,
            width,
        ));
    }
    if let Some(error) = &s.hosted_error {
        lines.push(line(format!("Hosted: {error}"), WARN, width));
    }
    if let Some(error) = &s.local_error {
        lines.push(line(format!("Local: {error}"), WARN, width));
    }
    card(f, area, &title, lines);
}

fn active(f: &mut Frame, area: Rect, state: &State) {
    let width = area.width.saturating_sub(2) as usize;
    let mut lines = Vec::new();
    for run in &state.snapshot.active {
        let progress = run
            .estimate
            .as_ref()
            .map(|e| format!(" · ~{} left", crate::status::dur(e.remaining)))
            .unwrap_or_default();
        let current = run
            .running
            .iter()
            .take(3)
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(line(
            format!(
                "● {} · {} · {}{progress} · {current}",
                run.label,
                run.place,
                crate::status::dur(run.elapsed)
            ),
            GOOD,
            width,
        ));
    }
    if lines.is_empty() {
        lines.push(line("No active builds observed by this client. Run justrust check, test or build in another terminal.", MUTED, width));
    }
    card(
        f,
        area,
        &format!(
            "Active on this client · {} builds",
            state.snapshot.active.len()
        ),
        lines,
    );
}

/// The kind filter as a row of chips with the run count behind each choice.
fn kind_bar(state: &State, width: usize) -> Line<'static> {
    let counts = state.kind_counts();
    let mut chips = vec![(None, counts.iter().sum::<usize>())];
    chips.extend(Kind::ALL.iter().map(|k| (Some(*k), counts[*k as usize])));
    let mut spans = vec![Span::styled("kind ", Style::default().fg(FAINT))];
    let mut used = 5;
    for (kind, count) in chips {
        let chosen = kind == state.kind;
        // Empty kinds stay out of the way unless they are the active filter.
        if count == 0 && !chosen && kind.is_some() {
            continue;
        }
        let text = format!(" {} {count} ", kind.map_or("all", Kind::label));
        used += text.len() + 1;
        if used > width {
            break;
        }
        let style = if chosen {
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(MUTED)
        };
        spans.push(Span::styled(text, style));
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

/// What happened, in words. The cause comes only from the class the recorder
/// stored. An exit code is shown when nothing better is known.
fn result_label(run: &RunRow) -> (String, Color) {
    use crate::failure::Class;
    let Some(code) = run.exit else {
        return ("not loaded".into(), MUTED);
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

fn history(f: &mut Frame, area: Rect, state: &State) {
    let rows = state.rows();
    let mut title = format!(
        "Runs · {} · {} · {} · {}",
        rows.len(),
        state
            .project
            .as_deref()
            .map_or("all projects", super::project_name),
        state.outcome.label(),
        state.place.label()
    );
    if !state.query.is_empty() {
        title.push_str(&format!(" · matching \"{}\"", state.query));
    }
    let border = block(&title, area.width);
    let inner = border.inner(area);
    f.render_widget(border, area);
    if inner.height == 0 {
        return;
    }
    let width = inner.width as usize;
    let wide = width >= 88;
    // Fixed columns: result 16, where 8, wall 6, when 8, plus separators.
    let command_width = width.saturating_sub(44);
    let header = if wide {
        format!(
            "{:<16} {:<8} {:>6}  {}  {:>8}",
            "RESULT",
            "WHERE",
            "WALL",
            pad("COMMAND / PROJECT", command_width),
            "WHEN"
        )
    } else {
        "RESULT · WHEN · COMMAND".into()
    };
    let mut lines = vec![kind_bar(state, width), line(header, FAINT, width)];
    let visible = inner.height.saturating_sub(2) as usize;
    let start = state.selected.saturating_sub(visible.saturating_sub(1));
    for (i, run) in rows.iter().enumerate().skip(start).take(visible) {
        let (result, color) = result_label(run);
        let command = if run.args.is_empty() {
            format!(
                "{} · {}",
                run.machine.as_deref().unwrap_or("account recording"),
                run.id
            )
        } else {
            format!("{} · {}", run.args.join(" "), run.cwd)
        };
        let text = if wide {
            format!(
                "{} {} {:>6}  {}  {:>8}",
                pad(&result, 16),
                pad(&run.place, 8),
                if run.exit.is_some() {
                    crate::status::dur(run.wall)
                } else {
                    "?".into()
                },
                pad(&command, command_width),
                when(run.start)
            )
        } else {
            format!("{result} · {} · {command}", when(run.start))
        };
        let mut l = line(text, color, width);
        if i == state.selected {
            l = l.style(
                Style::default()
                    .bg(Color::Rgb(60, 60, 80))
                    .add_modifier(Modifier::BOLD),
            );
        }
        lines.push(l);
    }
    if rows.is_empty() {
        lines.push(line(
            if state.filters_active() {
                "No runs match these filters. Press x to clear them."
            } else {
                "No recordings yet. Run justrust check, test or build."
            },
            MUTED,
            width,
        ));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn bucket_color(bucket: Bucket) -> Color {
    match bucket {
        Bucket::Compile => ACCENT,
        Bucket::Tests => GOOD,
        Bucket::Sync => Color::Rgb(100, 160, 255),
        Bucket::Artifacts => WARN,
        Bucket::Hosted => Color::Rgb(90, 200, 200),
        Bucket::Startup => Color::Rgb(230, 130, 180),
        Bucket::Lock => BAD,
        Bucket::Client => Color::Rgb(170, 170, 190),
        Bucket::Other => FAINT,
    }
}

/// One stacked bar across the card, then a legend that flows onto as many
/// lines as it needs (at most three).
fn profile_lines(profile: &Profile, width: usize) -> Vec<Line<'static>> {
    if profile.timed == 0 || profile.total <= 0.0 {
        return vec![line(
            "No timings for these runs. Timings come from runs recorded on this machine.",
            MUTED,
            width,
        )];
    }
    let share = |secs: f64| secs / profile.total;
    let mut cells: Vec<usize> = profile
        .buckets
        .iter()
        .map(|(_, secs)| (share(*secs) * width as f64).floor() as usize)
        .collect();
    // Rounding leftovers go to the largest bucket so the bar spans the card.
    let spare = width.saturating_sub(cells.iter().sum());
    if let Some(first) = cells.first_mut() {
        *first += spare;
    }
    let bar = profile
        .buckets
        .iter()
        .zip(&cells)
        .map(|((bucket, _), n)| {
            Span::styled("█".repeat(*n), Style::default().fg(bucket_color(*bucket)))
        })
        .collect::<Vec<_>>();
    let mut lines = vec![Line::from(bar), Line::default()];
    let mut used = 0;
    for (bucket, secs) in &profile.buckets {
        let percent = match share(*secs) * 100.0 {
            p if p < 1.0 => "<1%".to_owned(),
            p => format!("{p:.0}%"),
        };
        let text = format!("{} {percent} {}", bucket.label(), crate::status::dur(*secs));
        let cost = text.len() + 5;
        if used + cost > width && used > 0 {
            if lines.len() == 4 {
                break;
            }
            lines.push(Line::default());
            used = 0;
        }
        used += cost;
        let current = lines.last_mut().expect("legend line exists");
        current.spans.push(Span::styled(
            "■ ",
            Style::default().fg(bucket_color(*bucket)),
        ));
        current
            .spans
            .push(Span::styled(text, Style::default().fg(MUTED)));
        current.spans.push(Span::raw("   "));
    }
    lines
}

fn profile_title(profile: &Profile) -> String {
    if profile.timed == 0 {
        return "Where the time went".into();
    }
    let mut title = format!(
        "Where the time went · {} waited over {} {}",
        crate::status::dur(profile.total),
        profile.timed,
        if profile.timed == 1 { "run" } else { "runs" }
    );
    if profile.timed < profile.runs {
        title.push_str(&format!(
            " · {} more without timings",
            profile.runs - profile.timed
        ));
    }
    title
}

/// The filter menu, drawn over the dashboard.
fn menu(f: &mut Frame, area: Rect, state: &State, at: usize) {
    let width = area.width.saturating_sub(4).min(64);
    let height = (MenuRow::ALL.len() as u16 + 4).min(area.height);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    let inner_width = width.saturating_sub(2) as usize;
    let mut lines = Vec::new();
    for (i, row) in MenuRow::ALL.iter().enumerate() {
        let chosen = i == at;
        let value = state.menu_value(*row);
        let text = if chosen && *row == MenuRow::Search {
            format!("› {:<8} {value}   (enter to type)", row.label())
        } else if chosen {
            format!("› {:<8} ‹ {value} ›", row.label())
        } else {
            format!("  {:<8} {value}", row.label())
        };
        let mut l = line(text, if chosen { Color::Reset } else { MUTED }, inner_width);
        if chosen {
            l = l.style(
                Style::default()
                    .bg(Color::Rgb(60, 60, 80))
                    .add_modifier(Modifier::BOLD),
            );
        }
        lines.push(l);
    }
    lines.push(Line::default());
    lines.push(line(
        format!("{} runs match", state.rows().len()),
        FAINT,
        inner_width,
    ));
    f.render_widget(Clear, popup);
    card(f, popup, "Filters", lines);
}

fn pad(text: &str, width: usize) -> String {
    let text = clip(text, width);
    let n = UnicodeWidthStr::width(text.as_str());
    format!("{text}{}", " ".repeat(width.saturating_sub(n)))
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
        let lines = text
            .lines()
            .skip(scroll)
            .map(|s| line(s, Color::Reset, body.width.saturating_sub(2) as usize))
            .collect();
        card(
            f,
            body,
            &format!("Run details · line {} of {}", scroll + 1, line_count),
            lines,
        );
        f.render_widget(
            Paragraph::new(line(
                "esc back · ↑↓ scroll · pgup/pgdn page · q quit",
                FAINT,
                footer.width as usize,
            )),
            footer,
        );
        return;
    }
    let account_h = if state.snapshot.hosted_error.is_some() {
        8
    } else {
        7
    };
    let active_h = (state.snapshot.active.len() as u16 + 2).clamp(3, 6);
    let profile = state.profile();
    let time_lines = profile_lines(&profile, area.width.saturating_sub(2) as usize);
    let profile_h = if area.height >= 24 {
        time_lines.len() as u16 + 2
    } else {
        0
    };
    let [top, running, time, runs, footer] = Layout::vertical([
        Constraint::Length(account_h.min(area.height.saturating_sub(6))),
        Constraint::Length(if area.height >= 18 { active_h } else { 0 }),
        Constraint::Length(profile_h),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);
    account(f, top, state);
    if running.height > 0 {
        active(f, running, state);
    }
    if time.height > 0 {
        card(f, time, &profile_title(&profile), time_lines);
    }
    history(f, runs, state);
    if let Some(at) = state.menu {
        menu(f, area, state, at);
    }
    let (help, color) = if state.searching {
        (
            format!(
                "search: {}▏ · enter keep · esc cancel · matches command, project, run ID",
                state.query
            ),
            ACCENT,
        )
    } else if state.menu.is_some() {
        (
            "↑↓ choose filter · ←→ change · / search · x clear all · s or esc close · q quit"
                .into(),
            ACCENT,
        )
    } else {
        (
            "↑↓ select · enter details · l logs · s filters · / search · x clear · a account · r refresh · q quit".into(),
            FAINT,
        )
    };
    f.render_widget(
        Paragraph::new(line(help, color, footer.width as usize)),
        footer,
    );
}

#[cfg(test)]
mod tests {
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
        let l = usage(25, 100, 80);
        let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("▰▰▰▱▱▱▱▱▱▱▱▱ 75% left"), "{text}");
        assert!(
            !usage(0, 0, 80)
                .spans
                .iter()
                .any(|s| s.content.contains('%'))
        );
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
        assert!(screen(&state, 140, 30).contains("search: zzz"));
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
        assert!(!text.contains("exit 101") && !text.contains("20261113-050528950"));
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
            text.contains("Where the time went · 1m40s waited over 2 runs"),
            "{text}"
        );
        assert!(text.contains("compiling 75% 1m15s"), "{text}");
        assert!(text.contains("source sync 25% 25s"), "{text}");
        // The bar spans the card and splits in proportion to the wait.
        let lines = profile_lines(&state.profile(), 100);
        let bar: Vec<usize> = lines[0]
            .spans
            .iter()
            .map(|s| s.content.chars().count())
            .collect();
        assert_eq!(bar, [75, 25]);

        state.kind = Some(Kind::Check);
        let text = screen(&state, 140, 40);
        assert!(text.contains("40s waited over 1 run "), "{text}");
        for width in [0, 1, 10, 30] {
            assert!(profile_lines(&state.profile(), width).len() <= 4, "{width}");
        }
        state.snapshot.runs.iter_mut().for_each(|r| r.time = None);
        let text = screen(&state, 140, 40);
        assert!(text.contains("No timings for these runs"), "{text}");
        // Short terminals keep the run list and drop the card.
        assert!(!screen(&state, 140, 20).contains("Where the time went"));
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
        assert!(text.contains("s or esc close"));
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
}
