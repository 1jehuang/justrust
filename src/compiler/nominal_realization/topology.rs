//! Node-local topology evidence. Coordinates are not successful read receipts.
use super::{Error, ir::PlanBudget};

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

fn finishing(edges: &[Vec<usize>], budget: &PlanBudget) -> Result<Vec<usize>, Error> {
    let mut seen = storage(edges.len(), budget)?;
    seen.resize(edges.len(), false);
    let mut order = storage(edges.len(), budget)?;
    let mut stack = storage(edges.len(), budget)?;
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
    let count = apps.len();
    let mut canonical = storage(count, budget)?;
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
    let mut forward = storage(count, budget)?;
    let mut reverse = storage(count, budget)?;
    let mut incoming = storage(count, budget)?;
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
        reverse.push(storage(count, budget)?);
    }
    for _ in 0..count {
        forward.push(Vec::new());
    }
    for (source, app) in apps.iter().enumerate() {
        forward[source] = storage(app.edges.len(), budget)?;
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
    let order = finishing(&forward, budget)?;
    let mut owner = storage(count, budget)?;
    owner.resize(count, usize::MAX);
    let mut groups: Vec<Vec<usize>> = storage(count, budget)?;
    let mut stack = storage(count, budget)?;
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
            budget
                .retain_bytes(std::mem::size_of::<usize>())
                .map_err(Error::new)?;
            members
                .try_reserve_exact(1)
                .map_err(|_| Error::new("nominal member allocation failed"))?;
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
    let mut layout = storage(count, budget)?;
    for app in apps {
        let mut edges = storage(app.edges.len(), budget)?;
        edges.extend(app.edges.iter().filter(|e| e.by_value).map(|e| e.target));
        layout.push(edges);
    }
    let order = finishing(&layout, budget)?;
    let mut position = storage(count, budget)?;
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
    let mut layouts = storage(groups.len(), budget)?;
    for group in &groups {
        layouts.push(storage(group.len(), budget)?);
    }
    for node in order {
        layouts[owner[node]].push(node);
    }
    let mut result = storage(groups.len(), budget)?;
    for (component, (members, layout_order)) in groups.into_iter().zip(layouts).enumerate() {
        let mut external = Vec::new();
        for &node in &members {
            for edge in &apps[node].edges {
                budget.charge_work(1).map_err(Error::new)?;
                if owner[edge.target] != component {
                    budget
                        .retain_bytes(std::mem::size_of::<usize>())
                        .map_err(Error::new)?;
                    external
                        .try_reserve_exact(1)
                        .map_err(|_| Error::new("nominal outbound allocation failed"))?;
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
