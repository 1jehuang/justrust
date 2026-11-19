//! A single selected run, sampled at the midpoint of each terminal cell.
//! Adjacent short spans never look concurrent merely because they share a cell.
use super::*;
use crate::dashboard::data::{Bucket, Timeline};

const COLORS: [Color; 9] = [
    ACCENT,
    GOOD,
    Color::Cyan,
    Color::Magenta,
    WARN,
    Color::LightBlue,
    OURS,
    Color::LightMagenta,
    LABEL,
];

pub(super) fn lines(trace: Option<&Timeline>, width: usize) -> Vec<Line<'static>> {
    let Some(t) = trace.filter(|t| t.wall.is_finite() && t.wall > 0. && !t.spans.is_empty()) else {
        return vec![line("Total elapsed: timeline unavailable", DIM, width)];
    };
    let mut result = single(t, width);
    if let Some(cargo) = t.cargo.as_deref() {
        result.extend(single(cargo, width));
    }
    result
}

fn single(t: &Timeline, width: usize) -> Vec<Line<'static>> {
    let prefix = if t.end_to_end {
        "Total elapsed  0s "
    } else {
        "Cargo elapsed  0s "
    };
    let suffix = format!(" {}", crate::status::dur(t.wall));
    let n = width.saturating_sub(prefix.len() + suffix.len()).min(160);
    let mut masks = vec![0u16; n];
    let boundary = |s: f64| ((s / t.wall * n as f64 - 0.5).ceil() as usize).min(n);
    for &(lo, hi, cat) in &t.spans {
        if lo.is_finite() && hi.is_finite() && lo < hi {
            for mask in &mut masks[boundary(lo)..boundary(hi)] {
                *mask |= 1 << cat as usize;
            }
        }
    }
    let mut bar = vec![span(prefix, LABEL)];
    let mut seen = 0;
    let mut gaps = false;
    let mut overlap = false;
    for (i, mask) in masks.into_iter().enumerate() {
        seen |= mask;
        if mask == 0 {
            gaps = true;
            bar.push(span("·", DIM));
            continue;
        }
        let mut colors = Bucket::ALL
            .iter()
            .filter(|b| mask & (1 << **b as usize) != 0)
            .map(|b| COLORS[*b as usize])
            .cycle()
            .skip(i % mask.count_ones() as usize);
        let fg = colors.next().unwrap();
        if mask.is_power_of_two() {
            bar.push(span("█", fg));
        } else {
            overlap = true;
            bar.push(Span::styled(
                "▚",
                Style::default().fg(fg).bg(colors.next().unwrap()),
            ));
        }
    }
    bar.push(span(suffix, LABEL));
    let mut key = Vec::new();
    for cat in Bucket::ALL {
        if seen & (1 << cat as usize) != 0 {
            key.push(span(format!("■ {}  ", cat.label()), COLORS[cat as usize]));
        }
    }
    if overlap {
        key.push(span("▚ overlap  ", VALUE));
    }
    if gaps {
        key.push(span("· untraced", DIM));
    }
    let mut rows = vec![fit(bar, width)];
    let mut labels = Vec::new();
    let mut used = 0;
    for label in key {
        if used + label.width() > width && !labels.is_empty() {
            rows.push(fit(std::mem::take(&mut labels), width));
            used = 0;
        }
        used += label.width();
        labels.push(label);
    }
    if !labels.is_empty() {
        rows.push(fit(labels, width));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn elapsed_cells_show_only_timestamped_concurrency() {
        let t = Timeline {
            end_to_end: true,
            cargo: None,
            wall: 10.,
            spans: vec![(0., 8., Bucket::Compile), (5., 9., Bucket::Tests)],
        };
        let w = "Total elapsed  0s ".len() + 10 + format!(" {}", crate::status::dur(t.wall)).len();
        let row = lines(Some(&t), w)[0].to_string();
        assert!(row.contains("█████▚▚▚█·"), "{row}");
        let cells = &lines(Some(&t), w)[0].spans;
        assert_eq!(cells[1].style.fg, Some(COLORS[Bucket::Compile as usize]));
        assert!(cells[6].style.bg.is_some());
        assert_eq!(cells[9].style.fg, Some(COLORS[Bucket::Tests as usize]));
        let sequential = Timeline {
            spans: vec![(0., 4.8, Bucket::Compile), (4.8, 9., Bucket::Tests)],
            ..t
        };
        assert!(!lines(Some(&sequential), w)[0].to_string().contains('▚'));
        assert!(lines(None, 60)[0].to_string().contains("unavailable"));
    }
}
