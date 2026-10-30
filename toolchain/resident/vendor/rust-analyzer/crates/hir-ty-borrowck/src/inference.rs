//! Region inference: what each region holds, from where liveness says it is live and the
//! outlives constraints between regions, as rustc's `RegionInferenceContext` computes it on
//! the strongly connected components of the constraint graph.

use smallvec::SmallVec;

use super::{
    bitset::BitSet,
    intervals::IntervalSet,
    regions::{Regions, Rv},
};

/// The points each region holds at, and the universal regions it outlives: one value for each
/// strongly connected component of the outlives graph, whose regions hold the same.
pub(super) struct RegionValues {
    scc: Vec<u32>,
    points: Vec<IntervalSet>,
    universals: Vec<BitSet>,
}

impl RegionValues {
    pub(super) fn contains_point(&self, rv: Rv, point: usize) -> bool {
        self.points[self.scc[rv] as usize].contains(point)
    }

    pub(super) fn points_of(&self, rv: Rv) -> &IntervalSet {
        &self.points[self.scc[rv] as usize]
    }

    pub(super) fn universals_of(&self, rv: Rv) -> impl Iterator<Item = usize> + '_ {
        self.universals[self.scc[rv] as usize].iter()
    }
}

/// Region inference: the values liveness gives grown along the outlives constraints until
/// each region holds wherever a region it outlives does. As rustc's, it works on the strongly
/// connected components of the graph, each once, those it outlives first.
pub(super) fn infer(
    regions: &Regions<'_>,
    count: usize,
    points: usize,
    live: Vec<IntervalSet>,
) -> RegionValues {
    // `sup: sub` makes `sup` hold what `sub` does: an edge from `sup` to `sub`.
    let mut edges: Vec<SmallVec<[u32; 4]>> = vec![SmallVec::new(); count];
    for constraint in &regions.outlives {
        if constraint.sup < count && constraint.sub < count {
            edges[constraint.sup].push(constraint.sub as u32);
        }
    }
    let (scc, sccs) = strongly_connected(&edges);
    // Tarjan's algorithm numbers a component after every component it reaches.
    let mut values = vec![IntervalSet::default(); sccs];
    let mut universals = vec![BitSet::new(regions.universals.len()); sccs];
    for (rv, it) in live.into_iter().enumerate() {
        values[scc[rv] as usize].union(&it);
    }
    for (index, &(rv, _)) in regions.universals.iter().enumerate() {
        if rv >= count {
            continue;
        }
        let at = scc[rv] as usize;
        if points > 0 {
            values[at].insert_range(0, points - 1);
        }
        universals[at].insert(index);
    }
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); sccs];
    for rv in 0..count {
        members[scc[rv] as usize].push(rv as u32);
    }
    let mut seen = vec![u32::MAX; sccs];
    for at in 0..sccs {
        for &rv in &members[at] {
            for &sub in &edges[rv as usize] {
                let from = scc[sub as usize] as usize;
                if from == at || seen[from] == at as u32 {
                    continue;
                }
                seen[from] = at as u32;
                debug_assert!(from < at);
                let (done, rest) = values.split_at_mut(at);
                rest[0].union(&done[from]);
                let (done, rest) = universals.split_at_mut(at);
                rest[0].union(&done[from]);
            }
        }
    }
    RegionValues { scc, points: values, universals }
}

/// The strongly connected components of a graph, by Tarjan's algorithm without recursion: the
/// component of each node, numbered so that every component a component reaches has a
/// smaller number, and how many there are.
fn strongly_connected(edges: &[SmallVec<[u32; 4]>]) -> (Vec<u32>, usize) {
    let count = edges.len();
    const NONE: u32 = u32::MAX;
    let mut index = vec![NONE; count];
    let mut low = vec![0u32; count];
    let mut on_stack = vec![false; count];
    let mut scc = vec![NONE; count];
    let mut stack: Vec<u32> = Vec::new();
    let mut next_index = 0u32;
    let mut sccs = 0u32;
    // (node, the next of its edges to follow)
    let mut calls: Vec<(u32, usize)> = Vec::new();
    for root in 0..count as u32 {
        if index[root as usize] != NONE {
            continue;
        }
        calls.push((root, 0));
        index[root as usize] = next_index;
        low[root as usize] = next_index;
        next_index += 1;
        stack.push(root);
        on_stack[root as usize] = true;
        while let Some(&mut (node, ref mut next)) = calls.last_mut() {
            let n = node as usize;
            if let Some(&to) = edges[n].get(*next) {
                *next += 1;
                let t = to as usize;
                if index[t] == NONE {
                    index[t] = next_index;
                    low[t] = next_index;
                    next_index += 1;
                    stack.push(to);
                    on_stack[t] = true;
                    calls.push((to, 0));
                } else if on_stack[t] {
                    low[n] = low[n].min(index[t]);
                }
                continue;
            }
            calls.pop();
            if let Some(&(parent, _)) = calls.last() {
                low[parent as usize] = low[parent as usize].min(low[n]);
            }
            if low[n] == index[n] {
                loop {
                    let member = stack.pop().unwrap();
                    on_stack[member as usize] = false;
                    scc[member as usize] = sccs;
                    if member == node {
                        break;
                    }
                }
                sccs += 1;
            }
        }
    }
    (scc, sccs as usize)
}
