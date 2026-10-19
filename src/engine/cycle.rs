//! Quiescence diagnostics from live unresolved engine waits, not event capture.
//! Allocation is engine infrastructure: input-scaled checked counts and fallible
//! Vec reservations. Compiler interpretation pays its own diagnostic budget.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencyCycleEdge {
    consumer: Key,
    producer: Key,
    output: usize,
}
impl DependencyCycleEdge {
    pub fn consumer(&self) -> &Key {
        &self.consumer
    }
    pub fn producer(&self) -> &Key {
        &self.producer
    }
    pub fn output(&self) -> usize {
        self.output
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencyCycle {
    edges: Vec<DependencyCycleEdge>,
}
impl DependencyCycle {
    /// Complete closed ordered cycle. The final producer is the first consumer.
    pub fn edges(&self) -> &[DependencyCycleEdge] {
        &self.edges
    }
}
#[derive(Clone, Copy)]
struct Wait {
    from: usize,
    to: usize,
    output: usize,
}
struct Frame {
    node: usize,
    next: usize,
    end: usize,
}
fn vector<T>(count: usize, admit: &mut impl FnMut(usize) -> bool) -> Option<Vec<T>> {
    let bytes = count.checked_mul(std::mem::size_of::<T>())?;
    if bytes > isize::MAX as usize || !admit(bytes) {
        return None;
    }
    let mut values = Vec::new();
    values.try_reserve_exact(count).ok()?;
    Some(values)
}
fn copy_key(key: &Key, admit: &mut impl FnMut(usize) -> bool) -> Option<Key> {
    let mut params = vector(key.params.len(), admit)?;
    params.extend_from_slice(&key.params);
    Some(Key {
        kind: key.kind,
        version: key.version,
        params,
    })
}
// Nodes are borrowed from Shared, sorted by monotonic node ID below. Scratch
// never escapes. A failed reservation or incomplete/external wait yields None.
fn extract(
    pending: &[PendingWaits],
    nodes: &[Arc<Node>],
    admit: &mut impl FnMut(usize) -> bool,
) -> Option<DependencyCycle> {
    if pending.is_empty() || pending.iter().any(|(_, _, waits)| waits.is_empty()) {
        return None;
    }
    let count = nodes.len();
    let edge_count = pending.iter().try_fold(0usize, |count, (_, _, waits)| {
        count.checked_add(waits.len())
    })?;
    let mut ordered = vector::<&Arc<Node>>(count, admit)?;
    ordered.extend(nodes.iter());
    ordered.sort_unstable_by_key(|node| node.id);
    let position = |id| ordered.binary_search_by_key(&id, |node| node.id).ok();
    let mut edges = vector::<Wait>(edge_count, admit)?;
    for (_, consumer, waits) in pending {
        let from = position(*consumer)?;
        for (producer, output) in waits {
            let to = position(*producer)?;
            if lock(ordered[to].slots.get(*output)?).result.is_some() {
                return None;
            }
            edges.push(Wait {
                from,
                to,
                output: *output,
            });
        }
    }
    edges.sort_unstable_by_key(|edge| (edge.from, edge.to, edge.output));
    edges.dedup_by_key(|edge| (edge.from, edge.to, edge.output));
    let mut colors = vector::<u8>(count, admit)?;
    colors.resize(count, 0);
    let mut parent = vector::<Option<usize>>(count, admit)?;
    parent.resize(count, None);
    let mut stack = vector::<Frame>(count, admit)?;
    let bounds = |node| {
        let start = edges.partition_point(|edge| edge.from < node);
        let end = edges.partition_point(|edge| edge.from <= node);
        Frame {
            node,
            next: start,
            end,
        }
    };
    for start in 0..count {
        if colors[start] != 0 {
            continue;
        }
        colors[start] = 1;
        stack.push(bounds(start));
        while let Some(frame) = stack.last_mut() {
            if frame.next == frame.end {
                colors[frame.node] = 2;
                stack.pop();
                continue;
            }
            let edge_index = frame.next;
            frame.next += 1;
            let edge = edges[edge_index];
            if colors[edge.to] == 0 {
                parent[edge.to] = Some(edge_index);
                colors[edge.to] = 1;
                stack.push(bounds(edge.to));
            } else if colors[edge.to] == 1 {
                let mut path = vector::<usize>(count, admit)?;
                let mut cursor = edge.from;
                while cursor != edge.to {
                    if path.len() >= count {
                        return None;
                    }
                    let incoming = parent[cursor]?;
                    path.push(incoming);
                    cursor = edges[incoming].from;
                }
                path.reverse();
                path.push(edge_index);
                let mut witness = vector::<DependencyCycleEdge>(path.len(), admit)?;
                for index in path {
                    let edge = edges[index];
                    witness.push(DependencyCycleEdge {
                        consumer: copy_key(&ordered[edge.from].key, admit)?,
                        producer: copy_key(&ordered[edge.to].key, admit)?,
                        output: edge.output,
                    });
                }
                if witness.is_empty()
                    || witness.iter().enumerate().any(|(index, edge)| {
                        edge.producer != witness[(index + 1) % witness.len()].consumer
                    })
                {
                    return None;
                }
                return Some(DependencyCycle { edges: witness });
            }
        }
    }
    None
}
impl Shared {
    pub(super) fn capture_dependency_cycle(&self, pending: &[PendingWaits]) {
        let nodes = lock(&self.nodes);
        let witness = extract(pending, &nodes, &mut |_| true);
        drop(nodes);
        if let Some(witness) = witness {
            lock(&self.accounting).report.dependency_cycle = Some(witness);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Empty;
    impl Provider for Empty {
        fn outputs(&self, _: &Key) -> usize {
            2
        }
        fn start(&self, _: Context, _: Key) -> TaskFuture {
            Box::pin(std::future::pending())
        }
    }
    #[test]
    fn witness_allocation_failure_never_mints_partial_cycle() {
        let engine = Engine::new(Empty);
        let _a = engine.request(Key::new(119, vec![0]));
        let _b = engine.request(Key::new(119, vec![1]));
        let nodes = lock(&engine.shared.nodes);
        let pending = vec![
            (1, nodes[0].id, HashSet::from([(nodes[1].id, 1)])),
            (2, nodes[1].id, HashSet::from([(nodes[0].id, 1)])),
        ];
        let mut allocations = 0;
        assert!(
            extract(&pending, &nodes, &mut |_| {
                allocations += 1;
                true
            })
            .is_some()
        );
        for fail_at in 0..allocations {
            let mut current = 0;
            assert!(
                extract(&pending, &nodes, &mut |_| {
                    let admitted = current != fail_at;
                    current += 1;
                    admitted
                })
                .is_none()
            );
        }
    }
    #[test]
    fn witness_allocations_are_fallible_and_overflow_checked() {
        assert!(vector::<u128>(usize::MAX, &mut |_| true).is_none());
        assert!(vector::<u64>(1, &mut |_| false).is_none());
        assert!(copy_key(&Key::new(119, vec![1]), &mut |_| false).is_none());
    }
}
