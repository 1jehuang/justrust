//! The control flow graph of a body as the borrow checker walks it: every edge, the imaginary
//! ones of `FalseEdge` and `FalseUnwind` included, and a number for each location.

use la_arena::RawIdx;
use smallvec::SmallVec;

use hir_ty::mir::{BasicBlockId, MirBody, TerminatorKind};

/// A statement of a block, or its terminator when `statement` is the number of statements.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub(super) struct Location {
    pub(super) block: BasicBlockId,
    pub(super) statement: usize,
}

pub(super) struct Cfg {
    /// The number of the first location of each block, by its index.
    starts: Vec<usize>,
    pub(super) points: usize,
    pub(super) successors: Vec<SmallVec<[BasicBlockId; 2]>>,
    pub(super) predecessors: Vec<SmallVec<[BasicBlockId; 4]>>,
    /// The blocks reachable from the start, in reverse postorder.
    pub(super) order: Vec<BasicBlockId>,
    pub(super) reachable: Vec<bool>,
}

pub(super) fn index(block: BasicBlockId) -> usize {
    block.into_raw().into_u32() as usize
}

pub(super) fn block(index: usize) -> BasicBlockId {
    BasicBlockId::from_raw(RawIdx::from_u32(index as u32))
}

/// Where control may go after `kind`, unwinding and the edges that never run included.
pub(super) fn successors(kind: &TerminatorKind) -> SmallVec<[BasicBlockId; 2]> {
    match kind {
        TerminatorKind::FalseEdge { real_target, imaginary_target } => {
            smallvec::smallvec![*real_target, *imaginary_target]
        }
        _ => {
            let mut all = kind.successors();
            all.extend(kind.unwind());
            all
        }
    }
}

impl Cfg {
    pub(super) fn new(body: &MirBody<'_>) -> Self {
        let count = body.basic_blocks.len();
        let mut starts = Vec::with_capacity(count);
        let mut points = 0;
        let mut successors = vec![SmallVec::new(); count];
        let mut predecessors = vec![SmallVec::new(); count];
        for (id, data) in body.basic_blocks.iter() {
            starts.push(points);
            points += data.statements.len() + 1;
            if let Some(terminator) = &data.terminator {
                for target in self::successors(&terminator.kind) {
                    if !successors[index(id)].contains(&target) {
                        successors[index(id)].push(target);
                        predecessors[index(target)].push(id);
                    }
                }
            }
        }
        for &(from, _, cleanup) in &body.panic_unwinds {
            if !successors[index(from)].contains(&cleanup) {
                successors[index(from)].push(cleanup);
                predecessors[index(cleanup)].push(from);
            }
        }
        // rustc's `Postorder` takes each block's successors from the last, so its reverse
        // postorder, the order errors are found in, has a loop's exit before its body.
        let mut reachable = vec![false; count];
        let mut postorder = Vec::with_capacity(count);
        let mut stack = vec![(body.start_block, 0usize)];
        reachable[index(body.start_block)] = true;
        while let Some((block, next)) = stack.last_mut() {
            let block = *block;
            let all = &successors[index(block)];
            if let Some(&target) = all.len().checked_sub(*next + 1).map(|it| &all[it]) {
                *next += 1;
                if !reachable[index(target)] {
                    reachable[index(target)] = true;
                    stack.push((target, 0));
                }
            } else {
                postorder.push(block);
                stack.pop();
            }
        }
        postorder.reverse();
        Cfg { starts, points, successors, predecessors, order: postorder, reachable }
    }

    pub(super) fn point(&self, location: Location) -> usize {
        self.starts[index(location.block)] + location.statement
    }

    pub(super) fn location(&self, point: usize) -> Location {
        let block = self.starts.partition_point(|&start| start <= point) - 1;
        Location { block: self::block(block), statement: point - self.starts[block] }
    }

    pub(super) fn block_start(&self, block: BasicBlockId) -> usize {
        self.starts[index(block)]
    }

    /// The last location of `block`, its terminator.
    pub(super) fn block_end(&self, body: &MirBody<'_>, block: BasicBlockId) -> usize {
        self.starts[index(block)] + body.basic_blocks[block].statements.len()
    }

    pub(super) fn is_reachable(&self, block: BasicBlockId) -> bool {
        self.reachable[index(block)]
    }

    /// The locations control may come to `location` from.
    pub(super) fn predecessor_locations(
        &self,
        body: &MirBody<'_>,
        location: Location,
    ) -> SmallVec<[Location; 4]> {
        if location.statement > 0 {
            return smallvec::smallvec![Location { statement: location.statement - 1, ..location }];
        }
        self.predecessors[index(location.block)]
            .iter()
            .filter(|it| self.is_reachable(**it))
            .map(|&block| Location { block, statement: body.basic_blocks[block].statements.len() })
            .collect()
    }
}

/// The dominator tree of the reachable blocks, as rustc's: immediate dominators by semi-NCA
/// (Lengauer and Tarjan's semidominators, then nearest common ancestors), and each block's
/// entry and exit in a walk of the tree, so that whether one dominates another is two
/// comparisons. The iterative algorithm walked dominator chains for every predecessor, which a
/// long body of calls, each unwinding to one cleanup, made quadratic.
pub(super) struct Dominators {
    /// Each block's entry and exit in the walk of the tree; `usize::MAX` when unreachable.
    enter: Vec<usize>,
    exit: Vec<usize>,
}

const NONE: usize = usize::MAX;

impl Dominators {
    pub(super) fn new(cfg: &Cfg) -> Self {
        let start = cfg.order.first().map(|&it| index(it));
        Self::of_graph(cfg.reachable.len(), start, &cfg.successors, &cfg.predecessors)
    }

    fn of_graph<S: AsRef<[BasicBlockId]>, P: AsRef<[BasicBlockId]>>(
        count: usize,
        start: Option<usize>,
        successors: &[S],
        predecessors: &[P],
    ) -> Self {
        let mut enter = vec![NONE; count];
        let mut exit = vec![NONE; count];
        let Some(start) = start else { return Dominators { enter, exit } };
        // Depth-first preorder: `pre` of each block, `vertex` and `parent` by that number.
        let mut pre = vec![NONE; count];
        let mut vertex = Vec::new();
        let mut parent = Vec::new();
        let mut stack = vec![(start, NONE)];
        while let Some((block, from)) = stack.pop() {
            if pre[block] != NONE {
                continue;
            }
            pre[block] = vertex.len();
            vertex.push(block);
            parent.push(from);
            for &next in successors[block].as_ref().iter().rev() {
                if pre[index(next)] == NONE {
                    stack.push((index(next), pre[block]));
                }
            }
        }
        let n = vertex.len();
        let mut semi: Vec<usize> = (0..n).collect();
        let mut label: Vec<usize> = (0..n).collect();
        let mut ancestor = vec![NONE; n];
        let mut path = Vec::new();
        for w in (1..n).rev() {
            for &pred in predecessors[vertex[w]].as_ref() {
                let v = pre[index(pred)];
                if v == NONE {
                    continue;
                }
                // The least semidominator on the forest's path from `v` up, compressed.
                let u = if ancestor[v] == NONE {
                    v
                } else {
                    let mut x = v;
                    while ancestor[ancestor[x]] != NONE {
                        path.push(x);
                        x = ancestor[x];
                    }
                    while let Some(x) = path.pop() {
                        let up = ancestor[x];
                        if semi[label[up]] < semi[label[x]] {
                            label[x] = label[up];
                        }
                        ancestor[x] = ancestor[up];
                    }
                    label[v]
                };
                semi[w] = semi[w].min(semi[u]);
            }
            ancestor[w] = parent[w];
        }
        let mut idom = parent;
        for w in 1..n {
            let mut x = idom[w];
            while x > semi[w] {
                x = idom[x];
            }
            idom[w] = x;
        }
        let mut children = vec![Vec::new(); n];
        for w in 1..n {
            children[idom[w]].push(w);
        }
        let mut time = 0;
        let mut stack = vec![(0, false)];
        while let Some((w, done)) = stack.pop() {
            if done {
                exit[vertex[w]] = time;
                time += 1;
                continue;
            }
            enter[vertex[w]] = time;
            time += 1;
            stack.push((w, true));
            stack.extend(children[w].iter().map(|&it| (it, false)));
        }
        Dominators { enter, exit }
    }

    pub(super) fn dominates(&self, a: BasicBlockId, b: BasicBlockId) -> bool {
        let (a, b) = (index(a), index(b));
        if self.enter[b] == NONE {
            return false;
        }
        a == b
            || (self.enter[a] != NONE
                && self.enter[a] <= self.enter[b]
                && self.exit[b] <= self.exit[a])
    }

    pub(super) fn location_dominates(&self, a: Location, b: Location) -> bool {
        if a.block == b.block {
            a.statement <= b.statement
        } else {
            self.dominates(a.block, b.block)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Dominators, block, index};
    use hir_ty::mir::BasicBlockId;

    /// Whether `a` dominates `b`, from each block's set of dominators found to a fixed point.
    fn naive(count: usize, preds: &[Vec<BasicBlockId>], reachable: &[bool]) -> Vec<Vec<bool>> {
        let mut dom = vec![vec![true; count]; count];
        dom[0] = (0..count).map(|it| it == 0).collect();
        let mut changed = true;
        while changed {
            changed = false;
            for b in 1..count {
                if !reachable[b] {
                    continue;
                }
                let mut new = vec![true; count];
                for &p in &preds[b] {
                    if reachable[index(p)] {
                        for (it, d) in new.iter_mut().zip(&dom[index(p)]) {
                            *it &= d;
                        }
                    }
                }
                new[b] = true;
                if new != dom[b] {
                    dom[b] = new;
                    changed = true;
                }
            }
        }
        dom
    }

    #[test]
    fn match_the_dominator_sets() {
        let mut seed = 0x2545f4914f6cdd1du64;
        let mut next = |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound as u64) as usize
        };
        for round in 0..500 {
            let count = 1 + next(24);
            let edges = next(count * 3 + 1);
            let mut succs = vec![Vec::new(); count];
            let mut preds = vec![Vec::new(); count];
            // A chain from the start keeps most blocks reachable, as a body's are.
            for b in 1..count {
                if next(4) != 0 {
                    let from = next(b);
                    succs[from].push(block(b));
                    preds[b].push(block(from));
                }
            }
            for _ in 0..edges {
                let (from, to) = (next(count), next(count));
                if !succs[from].contains(&block(to)) {
                    succs[from].push(block(to));
                    preds[to].push(block(from));
                }
            }
            let mut reachable = vec![false; count];
            let mut stack = vec![0];
            reachable[0] = true;
            while let Some(b) = stack.pop() {
                for &s in &succs[b] {
                    if !reachable[index(s)] {
                        reachable[index(s)] = true;
                        stack.push(index(s));
                    }
                }
            }
            let fast = Dominators::of_graph(count, Some(0), &succs, &preds);
            let want = naive(count, &preds, &reachable);
            for a in 0..count {
                for b in 0..count {
                    let want = reachable[b] && reachable[a] && want[b][a];
                    assert_eq!(
                        fast.dominates(block(a), block(b)),
                        want,
                        "round {round}: {a} dom {b}"
                    );
                }
            }
        }
    }
}
