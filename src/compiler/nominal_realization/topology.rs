//! Node-local topology evidence. Coordinates are not successful read receipts.
use super::{Error, ir::PlanBudget};
mod allocation;
use allocation::Allocations;

/// Temporary values are private and drop before their live-storage leases.
/// No method transfers the unguarded result vector out of this owner.
pub(super) struct ScopedComponents<'a> {
    values: Vec<Component>,
    _allocations: Allocations<'a>,
}
impl std::ops::Deref for ScopedComponents<'_> {
    type Target = [Component];
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ApplicationWords(pub(super) Vec<u64>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Edge {
    pub(super) target: usize,
    /// Field storage propagates through tuples, arrays and nominal fields,
    /// but not through pointers or through nominal type arguments alone.
    pub(super) by_value: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Application {
    pub(super) key: ApplicationWords,
    pub(super) edges: Vec<Edge>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Component {
    /// Local application indices in full canonical key order.
    pub(super) members: Vec<usize>,
    pub(super) external: Vec<usize>,
    /// Dependency-first order, independently checked for finite size.
    pub(super) layout_order: Vec<usize>,
}

pub(super) fn storage<T>(count: usize, budget: &PlanBudget) -> Result<Vec<T>, Error> {
    budget
        .retain_bytes(
            count
                .checked_mul(std::mem::size_of::<T>())
                .ok_or_else(|| Error::new("nominal component storage overflow"))?,
        )
        .map_err(Error::new)?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(count)
        .map_err(|_| Error::new("nominal component allocation failed"))?;
    Ok(result)
}

fn finishing(
    edges: &[Vec<usize>],
    budget: &PlanBudget,
    allocations: &mut Allocations<'_>,
) -> Result<Vec<usize>, Error> {
    let mut seen = allocations.storage(edges.len())?;
    seen.resize(edges.len(), false);
    let mut order = allocations.storage(edges.len())?;
    let mut stack = allocations.storage(edges.len())?;
    for start in 0..edges.len() {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push((start, 0usize));
        while let Some((node, next)) = stack.last_mut() {
            budget.charge_work(1).map_err(Error::new)?;
            if let Some(&target) = edges[*node].get(*next) {
                *next += 1;
                if !seen[target] {
                    seen[target] = true;
                    stack.push((target, 0));
                }
            } else {
                order.push(*node);
                stack.pop();
            }
        }
    }
    Ok(order)
}

pub(super) fn components(
    apps: &[Application],
    budget: &PlanBudget,
) -> Result<Vec<Component>, Error> {
    components_in(apps, budget, &mut Allocations::retained(budget))
}

pub(super) fn scoped_components<'a>(
    apps: &[Application],
    budget: &'a PlanBudget,
) -> Result<ScopedComponents<'a>, Error> {
    let mut allocations = Allocations::scoped(budget);
    let values = components_in(apps, budget, &mut allocations)?;
    Ok(ScopedComponents {
        values,
        _allocations: allocations,
    })
}

fn components_in(
    apps: &[Application],
    budget: &PlanBudget,
    allocations: &mut Allocations<'_>,
) -> Result<Vec<Component>, Error> {
    let count = apps.len();
    let mut canonical = allocations.storage(count)?;
    canonical.extend(0..count);
    // Charge comparisons by their maximum full-word cost, not just node count.
    let words = apps
        .iter()
        .try_fold(0usize, |n, app| n.checked_add(app.key.0.len()))
        .ok_or_else(|| Error::new("nominal component comparison overflow"))?;
    budget
        .charge_work(words.saturating_mul(count.max(1).ilog2() as usize + 1))
        .map_err(Error::new)?;
    canonical.sort_unstable_by(|&a, &b| apps[a].key.0.cmp(&apps[b].key.0));
    for pair in canonical.windows(2) {
        if apps[pair[0]].key == apps[pair[1]].key {
            return Err(Error::new(
                "duplicate nominal application in component inventory",
            ));
        }
    }
    let mut forward = allocations.storage(count)?;
    let mut reverse = allocations.storage(count)?;
    let mut incoming = allocations.storage(count)?;
    incoming.resize(count, 0usize);
    for app in apps {
        for edge in &app.edges {
            budget.charge_work(1).map_err(Error::new)?;
            let count = incoming.get_mut(edge.target).ok_or_else(|| {
                Error::new("nominal edge leaves discovered application inventory")
            })?;
            *count = count
                .checked_add(1)
                .ok_or_else(|| Error::new("nominal reverse edge count overflow"))?;
        }
    }
    for count in incoming {
        reverse.push(allocations.storage(count)?);
    }
    for _ in 0..count {
        forward.push(Vec::new());
    }
    for (source, app) in apps.iter().enumerate() {
        forward[source] = allocations.storage(app.edges.len())?;
        for edge in &app.edges {
            budget.charge_work(1).map_err(Error::new)?;
            if edge.target >= count {
                return Err(Error::new(
                    "nominal edge leaves discovered application inventory",
                ));
            }
            forward[source].push(edge.target);
            reverse[edge.target].push(source);
        }
    }
    let order = finishing(&forward, budget, allocations)?;
    let mut owner = allocations.storage(count)?;
    owner.resize(count, usize::MAX);
    let mut groups: Vec<Vec<usize>> = allocations.storage(count)?;
    let mut stack = allocations.storage(count)?;
    for &start in order.iter().rev() {
        if owner[start] != usize::MAX {
            continue;
        }
        let component = groups.len();
        let mut members = Vec::new();
        owner[start] = component;
        stack.push(start);
        while let Some(node) = stack.pop() {
            budget.charge_work(1).map_err(Error::new)?;
            allocations.grow(&mut members)?;
            members.push(node);
            for &target in &reverse[node] {
                if owner[target] == usize::MAX {
                    owner[target] = component;
                    stack.push(target);
                }
            }
        }
        members.sort_unstable_by(|&a, &b| apps[a].key.0.cmp(&apps[b].key.0));
        groups.push(members);
    }
    // Size cycles are independent of identity SCCs. Validate the complete
    // reachable size graph once, rather than allocating N scratch rows per SCC.
    let mut layout = allocations.storage(count)?;
    for app in apps {
        let mut edges = allocations.storage(app.edges.len())?;
        edges.extend(app.edges.iter().filter(|e| e.by_value).map(|e| e.target));
        layout.push(edges);
    }
    let order = finishing(&layout, budget, allocations)?;
    let mut position = allocations.storage(count)?;
    position.resize(count, 0);
    for (ordinal, &node) in order.iter().enumerate() {
        position[node] = ordinal;
    }
    for (node, targets) in layout.iter().enumerate() {
        for &target in targets {
            budget.charge_work(1).map_err(Error::new)?;
            if position[target] >= position[node] {
                return Err(Error::new(
                    "invalid: recursive nominal has infinite by-value layout",
                ));
            }
        }
    }
    let mut layouts = allocations.storage(groups.len())?;
    for group in &groups {
        layouts.push(allocations.storage(group.len())?);
    }
    for node in order {
        layouts[owner[node]].push(node);
    }
    let mut result = allocations.storage(groups.len())?;
    for (component, (members, layout_order)) in groups.into_iter().zip(layouts).enumerate() {
        let mut external = Vec::new();
        for &node in &members {
            for edge in &apps[node].edges {
                budget.charge_work(1).map_err(Error::new)?;
                if owner[edge.target] != component {
                    allocations.grow(&mut external)?;
                    external.push(edge.target);
                }
            }
        }
        external.sort_unstable_by(|&a, &b| apps[a].key.0.cmp(&apps[b].key.0));
        external.dedup();
        result.push(Component {
            members,
            external,
            layout_order,
        });
    }
    result.sort_unstable_by(|a, b| apps[a.members[0]].key.0.cmp(&apps[b.members[0]].key.0));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(key: u64, edges: &[(usize, bool)]) -> Application {
        Application {
            key: ApplicationWords(vec![1, 1, 7, key]),
            edges: edges
                .iter()
                .map(|&(target, by_value)| Edge { target, by_value })
                .collect(),
        }
    }

    #[test]
    fn scoped_topology_storage_preserves_checks_and_live_ownership() {
        for apps in [
            vec![],
            vec![app(1, &[])],
            vec![
                app(9, &[(1, true)]),
                app(3, &[(0, false), (2, false)]),
                app(20, &[]),
            ],
            vec![
                app(1, &[(1, false), (2, false)]),
                app(2, &[(2, false)]),
                app(3, &[]),
            ],
        ] {
            let retained_budget = PlanBudget::default();
            let retained = components(&apps, &retained_budget).unwrap();
            let retained_work = retained_budget.usage_snapshot().work;
            let budget = PlanBudget::default();
            let baseline = budget.usage_snapshot().bytes;
            let mut previous_work = 0;
            for _ in 0..64 {
                let scoped = scoped_components(&apps, &budget).unwrap();
                assert_eq!(&*scoped, retained.as_slice());
                assert!(budget.usage_snapshot().bytes > baseline);
                assert_eq!(budget.usage_snapshot().work - previous_work, retained_work);
                previous_work = budget.usage_snapshot().work;
                drop(scoped);
                assert_eq!(budget.usage_snapshot().bytes, baseline);
            }
            let first = scoped_components(&apps, &budget).unwrap();
            let live = budget.usage_snapshot().bytes;
            let second = scoped_components(&apps, &budget).unwrap();
            assert!(budget.usage_snapshot().bytes > live);
            drop(second);
            assert_eq!(budget.usage_snapshot().bytes, live);
            drop(first);
            assert_eq!(budget.usage_snapshot().bytes, baseline);
        }
        let apps = [app(1, &[(1, false)]), app(2, &[(0, false)])];
        let poisoned = PlanBudget::default();
        poisoned.poison_ledger_for_tests();
        assert!(scoped_components(&apps, &poisoned).is_err());
        assert!(poisoned.usage_snapshot().poisoned);
        for limits in [
            crate::compiler::ir::PlanLimits {
                bytes: 0,
                ..Default::default()
            },
            crate::compiler::ir::PlanLimits {
                bytes: 64,
                ..Default::default()
            },
            crate::compiler::ir::PlanLimits {
                work: 0,
                ..Default::default()
            },
        ] {
            let budget = PlanBudget::new(limits);
            assert!(scoped_components(&apps, &budget).is_err());
            assert_eq!(budget.usage_snapshot().bytes, 0);
            assert!(budget.first_resource_failure().is_some());
        }
        for invalid in [
            vec![app(1, &[(1, false)])],
            vec![app(1, &[]), app(1, &[])],
            vec![app(1, &[(0, true)])],
            vec![app(1, &[(1, true)]), app(2, &[(0, true)])],
        ] {
            let budget = PlanBudget::default();
            let retained = components(&invalid, &PlanBudget::default()).unwrap_err();
            let scoped = match scoped_components(&invalid, &budget) {
                Ok(_) => panic!("forged or infinitely sized topology accepted"),
                Err(error) => error,
            };
            assert_eq!(scoped.to_string(), retained.to_string());
            assert_eq!(budget.usage_snapshot().bytes, 0);
            assert!(budget.first_resource_failure().is_none());
        }
    }

    #[test]
    fn pointer_cycle_has_canonical_members_and_dependency_first_layout() {
        let apps = [
            app(9, &[(1, true)]),
            app(3, &[(0, false), (2, false)]),
            app(20, &[]),
        ];
        let components = components(&apps, &PlanBudget::default()).unwrap();
        assert_eq!(
            components[0],
            Component {
                members: vec![1, 0],
                external: vec![2],
                layout_order: vec![1, 0],
            }
        );
        assert_eq!(components[1].members, vec![2]);
    }

    #[test]
    fn direct_and_mutual_by_value_cycles_reject() {
        assert!(components(&[app(1, &[(0, true)])], &PlanBudget::default()).is_err());
        assert!(
            components(
                &[app(1, &[(1, true)]), app(2, &[(0, true)])],
                &PlanBudget::default()
            )
            .is_err()
        );
    }

    #[test]
    fn forged_edges_and_duplicate_complete_keys_reject() {
        assert!(components(&[app(1, &[(1, false)])], &PlanBudget::default()).is_err());
        assert!(components(&[app(1, &[]), app(1, &[])], &PlanBudget::default()).is_err());
    }
}
