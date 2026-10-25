//! Receiver authority is carried by existing immutable source and scope facts.
use justrust_engine::compiler;
use std::collections::BTreeMap;

type GraphKey = (u32, Vec<u64>);

// Mirror the nominal coordinate grammar, not numeric value heuristics. All
// declaration indices, tags, lengths, literal coordinates and multiplicities
// remain exact. Only SnapshotId/world issuer words are alpha-renamed.
fn coordinate(words: &[u64], at: &mut usize, owners: &mut Vec<usize>) {
    let tag = words[*at];
    *at += 1;
    match tag {
        1 | 2 => *at += 1,
        3..=7 => {}
        8 | 9 | 11 => {
            *at += 1;
            coordinate(words, at, owners);
        }
        10 => {
            let count = words[*at];
            *at += 1;
            for _ in 0..count {
                coordinate(words, at, owners);
            }
        }
        12 => coordinate(words, at, owners),
        13 | 14 => {
            owners.push(*at);
            let count = words[*at + 2];
            *at += 3;
            for _ in 0..count {
                coordinate(words, at, owners);
            }
            if tag == 14 {
                let assigned = words[*at];
                *at += 1;
                for _ in 0..assigned {
                    owners.push(*at);
                    *at += 2;
                    coordinate(words, at, owners);
                }
            }
        }
        _ => panic!("unknown nominal coordinate tag {tag}"),
    }
    assert!(*at <= words.len(), "truncated nominal coordinate");
}

fn authority_positions(kind: u32, words: &[u64]) -> Vec<usize> {
    match kind {
        // Associated constant, nominal declaration and impl declaration IDs.
        134 | 138 | 145 => {
            assert_eq!(words.len(), 2);
            vec![0]
        }
        139 => {
            assert_eq!(words[0], 1, "nominal realization encoding version");
            match words[1] {
                0 => {
                    assert_eq!(words.len(), 4);
                    vec![2]
                }
                1 => {
                    let mut owners = vec![2, 3]; // world and declaration snapshot
                    let mut at = 6;
                    for _ in 0..words[5] {
                        let length = words[at] as usize;
                        at += 1;
                        let end = at.checked_add(length).unwrap();
                        assert!(end <= words.len());
                        coordinate(&words[..end], &mut at, &mut owners);
                        assert_eq!(at, end);
                    }
                    assert_eq!(at, words.len());
                    owners
                }
                tag => panic!("unknown nominal realization domain {tag}"),
            }
        }
        _ => vec![],
    }
}

fn normalize_authorities(mut keys: Vec<GraphKey>) -> Vec<GraphKey> {
    // An authority's typed occurrence signature determines its canonical name,
    // independently of evaluation order or the issuer's numeric allocation.
    let mut signatures: BTreeMap<u64, Vec<(GraphKey, usize)>> = BTreeMap::new();
    for (kind, words) in &keys {
        let positions = authority_positions(*kind, words);
        let mut shape = (*kind, words.clone());
        for &position in &positions {
            shape.1[position] = 0;
        }
        for position in positions {
            signatures
                .entry(words[position])
                .or_default()
                .push((shape.clone(), position));
        }
    }
    let mut authorities = signatures.into_iter().collect::<Vec<_>>();
    for (_, signature) in &mut authorities {
        signature.sort();
    }
    authorities.sort_by(|left, right| left.1.cmp(&right.1));
    let names = authorities
        .into_iter()
        .enumerate()
        .map(|(index, (owner, _))| (owner, index as u64 + 1))
        .collect::<BTreeMap<_, _>>();
    for (kind, words) in &mut keys {
        for position in authority_positions(*kind, words) {
            words[position] = names[&words[position]];
        }
    }
    keys.sort();
    keys
}

#[test]
fn authority_normalization_preserves_declarations_aliases_and_multiplicity() {
    let keys = vec![
        (138, vec![900, 7]),
        (145, vec![900, 3]),
        (134, vec![100, 7]),
        (139, vec![1, 0, 900, 7]),
        (139, vec![1, 1, 100, 900, 7, 1, 4, 13, 900, 8, 0]),
        (128, vec![900]),
    ]; // a callable index is NOT an authority
    let mut renamed = keys.clone();
    for (kind, words) in &mut renamed {
        for position in authority_positions(*kind, words) {
            words[position] = if words[position] == 900 { 2 } else { 9999 };
        }
    }
    renamed.reverse();
    assert_eq!(
        normalize_authorities(keys.clone()),
        normalize_authorities(renamed.clone())
    );
    renamed
        .iter_mut()
        .find(|(kind, words)| *kind == 139 && words[1] == 0)
        .unwrap()
        .1[3] += 1; // source-schema declaration index
    assert_ne!(
        normalize_authorities(keys.clone()),
        normalize_authorities(renamed)
    );
    let mut unaliased = keys.clone();
    unaliased[1].1[0] = 101;
    assert_ne!(
        normalize_authorities(keys.clone()),
        normalize_authorities(unaliased)
    );
    let mut duplicated = keys.clone();
    duplicated.push(keys[0].clone());
    assert_ne!(
        normalize_authorities(keys),
        normalize_authorities(duplicated)
    );
}

#[test]
fn receiver_macros_check_without_codegen_or_new_graph_barriers_at_workers_1_8() {
    let sources = [
        "struct S{v:i32}impl S{fn get(&self)->i32{self.v}}fn main(){let s=S{v:42};assert_eq!(s.get(),42);}",
        "struct S{v:i32}impl S{fn get(&self)->i32{macro_rules! get{()=>{self.v};}get!()}}fn main(){let s=S{v:42};assert_eq!(s.get(),42);}",
    ];
    for workers in [1, 8] {
        let mut baseline = None;
        for source in sources {
            let checked = compiler::compile_observed(source.into(), workers, true).unwrap();
            assert!(checked.executable.is_none());
            assert!(checked.report.capture_complete);
            assert_eq!(checked.report.events_dropped, 0);
            assert!(
                checked
                    .report
                    .node_evaluations
                    .iter()
                    .all(|(key, count)| *count == 1 && !matches!(key.kind, 103 | 105 | 130..=133))
            );
            let keys = checked
                .report
                .node_evaluations
                .iter()
                .map(|(key, _)| (key.kind, key.params.clone()))
                .collect::<Vec<_>>();
            let keys = normalize_authorities(keys);
            if let Some(expected) = &baseline {
                assert_eq!(&keys, expected);
            } else {
                baseline = Some(keys);
            }
        }
    }
}
