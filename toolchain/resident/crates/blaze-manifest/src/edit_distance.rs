//! cargo's "did you mean" suggestions (`src/util/edit_distance.rs` there), which its messages
//! end with.

use std::{cmp, mem};

/// The edit distance of `a` and `b` ignoring case, or `None` past `limit`.
pub(crate) fn edit_distance(a: &str, b: &str, limit: usize) -> Option<usize> {
    let a = a.to_lowercase();
    let b = b.to_lowercase();

    let mut a = &a.chars().collect::<Vec<_>>()[..];
    let mut b = &b.chars().collect::<Vec<_>>()[..];

    if a.len() < b.len() {
        mem::swap(&mut a, &mut b);
    }

    let min_dist = a.len() - b.len();
    if min_dist > limit {
        return None;
    }

    while let Some(((b_char, b_rest), (a_char, a_rest))) = b.split_first().zip(a.split_first()) {
        if a_char != b_char {
            break;
        }
        a = a_rest;
        b = b_rest;
    }
    while let Some(((b_char, b_rest), (a_char, a_rest))) = b.split_last().zip(a.split_last()) {
        if a_char != b_char {
            break;
        }
        a = a_rest;
        b = b_rest;
    }

    if b.is_empty() {
        return Some(min_dist);
    }

    let mut prev_prev = vec![usize::MAX; b.len() + 1];
    let mut prev = (0..=b.len()).collect::<Vec<_>>();
    let mut current = vec![0; b.len() + 1];

    for i in 1..=a.len() {
        current[0] = i;
        let a_idx = i - 1;
        for j in 1..=b.len() {
            let b_idx = j - 1;
            let substitution_cost = if a[a_idx] == b[b_idx] { 0 } else { 1 };
            current[j] = cmp::min(
                prev[j] + 1,
                cmp::min(current[j - 1] + 1, prev[j - 1] + substitution_cost),
            );
            if (i > 1) && (j > 1) && (a[a_idx] == b[b_idx - 1]) && (a[a_idx - 1] == b[b_idx]) {
                current[j] = cmp::min(current[j], prev_prev[j - 2] + 1);
            }
        }
        [prev_prev, prev, current] = [prev, current, prev_prev];
    }

    let distance = prev[b.len()];
    (distance <= limit).then_some(distance)
}

/// The element of `iter` nearest to `choice`, within an edit distance of 3.
pub(crate) fn closest<'a, T>(
    choice: &str,
    iter: impl Iterator<Item = T>,
    key: impl Fn(&T) -> &'a str,
) -> Option<T> {
    iter.filter_map(|e| Some((edit_distance(choice, key(&e), 3)?, e)))
        .min_by_key(|t| t.0)
        .map(|t| t.1)
}

/// `closest` as the "help: a ... with a similar name exists" line cargo appends.
pub(crate) fn closest_msg<'a, T>(
    choice: &str,
    iter: impl Iterator<Item = T>,
    key: impl Fn(&T) -> &'a str,
    kind: &str,
) -> String {
    match closest(choice, iter, &key) {
        Some(e) => format!("\n\nhelp: a {kind} with a similar name exists: `{}`", key(&e)),
        None => String::new(),
    }
}
