//! A single selected run, sampled at the midpoint of each terminal cell.
//! Adjacent short spans never look concurrent merely because they share a cell.
use super::*;
use crate::dashboard::data::{Bucket, Timeline};

pub(super) const COLORS: [Color; 9] = [
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

/// Exact uncovered wall time, independent of terminal cell sampling.
pub(super) fn gap(t: &Timeline) -> Option<f64> {
    if !t.wall.is_finite() || t.wall <= 0. || t.spans.is_empty() {
        return None;
    }
    let mut spans: Vec<_> = t
        .spans
        .iter()
        .filter_map(|&(lo, hi, _)| {
            (lo.is_finite() && hi.is_finite() && lo < hi && hi > 0. && lo < t.wall)
                .then(|| (lo.max(0.), hi.min(t.wall)))
        })
        .collect();
    spans.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let mut end: f64 = 0.;
    let mut gap = 0.;
    for (lo, hi) in spans {
        gap += (lo - end).max(0.);
        end = end.max(hi);
    }
    Some(gap + (t.wall - end))
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
    for (i, mask) in masks.into_iter().enumerate() {
        if mask == 0 {
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
            bar.push(Span::styled(
                "▚",
                Style::default().fg(fg).bg(colors.next().unwrap()),
            ));
        }
    }
    bar.push(span(suffix, LABEL));
    vec![fit(bar, width)]
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
        assert_eq!(gap(&t), Some(1.));
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
