//! The "a ... with a similar name exists" suggestions of cargo's messages:
//! `src/util/edit_distance.rs` of cargo 1.99.0 (5f94df4789).

use std::{cmp, mem};

/// The Damerau-Levenshtein distance of `a` and `b` lowercased, or `None` if it is over `limit`.
pub fn edit_distance(a: &str, b: &str, limit: usize) -> Option<usize> {
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

/// The element of `iter` closest to `choice`, within a distance of 3; the first of the
/// closest.
pub fn closest<'a, T>(
    choice: &str,
    iter: impl Iterator<Item = T>,
    key: impl Fn(&T) -> &'a str,
) -> Option<T> {
    iter.filter_map(|e| Some((edit_distance(choice, key(&e), 3)?, e)))
        .min_by_key(|t| t.0)
        .map(|t| t.1)
}

/// `\n\nhelp: a {kind} with a similar name exists: `x`` or nothing.
pub fn closest_msg<'a, T>(
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

#[cfg(test)]
mod tests {
    use super::*;

    // From cargo's `test_edit_distance`.
    #[test]
    fn distances() {
        let a = "\nMäry häd ä little lämb\n\nLittle lämb\n";
        let b = "\nMary häd ä little lämb\n\nLittle lämb\n";
        let c = "Mary häd ä little lämb\n\nLittle lämb\n";
        assert_eq!(edit_distance(a, b, usize::MAX), Some(1));
        assert_eq!(edit_distance(b, a, usize::MAX), Some(1));
        assert_eq!(edit_distance(a, c, usize::MAX), Some(2));
        assert_eq!(edit_distance(c, a, usize::MAX), Some(2));
        assert_eq!(edit_distance(b, c, usize::MAX), Some(1));
        assert_eq!(edit_distance(c, b, usize::MAX), Some(1));
        assert_eq!(edit_distance("ab", "ba", usize::MAX), Some(1));
        assert_eq!(edit_distance("CHECK", "check", 0), Some(0));
        assert_eq!(edit_distance("abcdefgh", "x", 3), None);
    }

    #[test]
    fn suggestion() {
        let names = ["default", "std", "serde"];
        assert_eq!(
            closest_msg("serd", names.iter(), |n| n, "feature"),
            "\n\nhelp: a feature with a similar name exists: `serde`"
        );
        assert_eq!(closest_msg("tokio-full", names.iter(), |n| n, "feature"), "");
    }
}
