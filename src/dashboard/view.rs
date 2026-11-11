//! Jcode-style chrome: one row per label, display-cell clipping, used-filled
//! usage bars and remaining percentages. All server/log text is untrusted.
use super::State;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const MUTED: Color = Color::Rgb(140, 140, 140);
const FAINT: Color = Color::Rgb(120, 120, 120);
const GOOD: Color = Color::Rgb(100, 200, 100);
const WARN: Color = Color::Rgb(255, 200, 100);
const BAD: Color = Color::Rgb(255, 100, 100);
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

fn history(f: &mut Frame, area: Rect, state: &State) {
    let rows = state.rows();
    let title = format!(
        "Runs · {} · {} · {}",
        rows.len(),
        if state.here {
            "this project"
        } else {
            "all projects"
        },
        if state.failed {
            "failed only"
        } else {
            "all results"
        }
    );
    let border = block(&title, area.width);
    let inner = border.inner(area);
    f.render_widget(border, area);
    if inner.height == 0 {
        return;
    }
    let width = inner.width as usize;
    let header = if width >= 88 {
        "RESULT       WHERE       WALL    COMMAND / PROJECT                                      RUN"
    } else {
        "RESULT · WHERE · COMMAND / RUN"
    };
    let mut lines = vec![line(header, FAINT, width)];
    let visible = inner.height.saturating_sub(1) as usize;
    let start = state.selected.saturating_sub(visible.saturating_sub(1));
    for (i, run) in rows.iter().enumerate().skip(start).take(visible) {
        let (result, color) = match run.exit {
            Some(0) => ("ok".into(), GOOD),
            Some(code) => (format!("exit {code}"), BAD),
            None => ("record".into(), MUTED),
        };
        let command = if run.args.is_empty() {
            format!(
                "{} · {}",
                run.machine.as_deref().unwrap_or("account recording"),
                run.id
            )
        } else {
            format!("{} · {}", run.args.join(" "), run.cwd)
        };
        let text = if width >= 88 {
            format!(
                "{result:<12} {:<10} {:>6}  {}  {}",
                clip(&run.place, 10),
                if run.exit.is_some() {
                    crate::status::dur(run.wall)
                } else {
                    "?".into()
                },
                pad(&command, width.saturating_sub(63)),
                run.id
            )
        } else {
            format!("{result} · {} · {command}", run.place)
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
            "No matching recordings yet. Filters: f failures · h this project",
            MUTED,
            width,
        ));
    }
    f.render_widget(Paragraph::new(lines), inner);
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
    let [top, running, runs, footer] = Layout::vertical([
        Constraint::Length(account_h.min(area.height.saturating_sub(6))),
        Constraint::Length(if area.height >= 18 { active_h } else { 0 }),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);
    account(f, top, state);
    if running.height > 0 {
        active(f, running, state);
    }
    history(f, runs, state);
    f.render_widget(
        Paragraph::new(line(
            "↑↓ select · enter details · l logs · a account · f failures · h project · r refresh · q quit",
            FAINT,
            footer.width as usize,
        )),
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
