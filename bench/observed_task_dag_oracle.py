#!/usr/bin/env python3
"""Original-record oracle for explicitly instrumented fork/join task DAGs.

This is acceptance arithmetic, not a replacement for the public validator.
Inputs must already pass the public lifecycle and timing authority checks.
No worker chronology, query ancestry, or availability relation becomes an edge.
"""
from collections import Counter, defaultdict
import heapq


def native_task_oracle(pool):
    """Reconstruct explicit task edges and measured weights from original bytes.

    Unknown CPU is rejected for positive acceptance, never replaced with zero.
    Query-qualified rows additionally require the caller to reconcile original
    query admissions before accepting this arithmetic as an observed DAG check.
    """
    timing = pool['timing']
    if (not pool['sealed'] or pool['interface_outcome'] != 'returned'
            or pool['dropped_events'] != 0 or not timing['records_complete']
            or not timing['sealed'] or timing['lost_segments'] != 0
            or timing['active_segments'] != 0
            or len(timing['segments']) != timing['expected_segments']):
        raise ValueError('positive oracle requires complete original task/timing inventories')
    tasks, roots, edges, edge_kinds = set(), set(), set(), {}
    starts, ends, definitions = {}, {}, {}
    for event in pool['events']:
        kind = event['kind']
        if kind == 'root':
            task = event['task']
            if task in definitions:
                raise ValueError('duplicate original task definition')
            definitions[task] = 'root'
            tasks.add(task)
            roots.add(task)
        elif kind == 'fork':
            parent, child, continuation = (event[name] for name in
                                           ('parent', 'child', 'continuation'))
            if child in definitions:
                raise ValueError('duplicate original child definition')
            definitions[child] = 'child'
            if continuation in definitions and definitions[continuation] != ('continuation', parent):
                raise ValueError('foreign original continuation definition')
            definitions[continuation] = ('continuation', parent)
            tasks.update((parent, child, continuation))
            edges.add((parent, child))
            edge_kinds[parent, child] = 'fork'
        elif kind == 'join':
            pair = event['child'], event['continuation']
            edges.add(pair)
            edge_kinds[pair] = 'join'
        elif kind in ('start', 'finish'):
            target = starts if kind == 'start' else ends
            if event['task'] in target:
                raise ValueError('duplicate original task lifecycle')
            target[event['task']] = event['at_ns']
        elif kind != 'resume':
            raise ValueError('unknown original task event')
    if tasks != set(definitions) or tasks != set(starts) or tasks != set(ends):
        raise ValueError('incomplete original task lifecycle')
    if len(tasks) != pool['expected_tasks']:
        raise ValueError('independent original task inventory mismatch')
    for left, right in edges:
        if left not in tasks or right not in tasks or ends[left] > starts[right]:
            raise ValueError('invalid original causal edge endpoint/order')
    weights = Counter({task: 0 for task in tasks})
    excluded = Counter()
    for row in pool['timing']['segments']:
        if row['status'] != 'measured' or type(row['cpu_ns']) is not int or row['cpu_ns'] < 0:
            raise ValueError('positive oracle requires measured original segments')
        task = row['task']
        if task is None:
            owner = row.get('owner_kind')
            if owner is None and timing.get('schema') == 1:
                owner = 'opaque'
            if owner is None:
                raise ValueError('missing original CPU owner category')
            excluded[owner] += row['cpu_ns']
            continue
        if task not in tasks or not starts[task] <= row['start_ns'] <= row['end_ns'] <= ends[task]:
            raise ValueError('original measured CPU outside actual task')
        weights[task] += row['cpu_ns']
    return dict(tasks=sorted(tasks), roots=sorted(roots), edges=sorted(edges),
                edge_kinds=edge_kinds, weights=dict(weights), excluded=dict(excluded),
                components=components_and_chains(tasks, edges, weights))


def components_and_chains(tasks, edges, weights):
    """Longest retained known CPU chain, with deterministic lexical tie breaking.

    This does not compute a full execution span, work/span ratio, or ceiling.
    """
    tasks = set(tasks)
    outgoing, neighbors = defaultdict(set), defaultdict(set)
    indegrees = {task: 0 for task in tasks}
    for left, right in edges:
        if left not in tasks or right not in tasks:
            raise ValueError('missing task endpoint')
        if right not in outgoing[left]:
            outgoing[left].add(right)
            indegrees[right] += 1
        neighbors[left].add(right)
        neighbors[right].add(left)
    ready = [task for task, degree in indegrees.items() if degree == 0]
    heapq.heapify(ready)
    paths, costs, visited = {}, {}, set()
    while ready:
        task = heapq.heappop(ready)
        visited.add(task)
        paths.setdefault(task, (task,))
        costs.setdefault(task, weights[task])
        for child in sorted(outgoing[task]):
            cost, path = costs[task] + weights[child], paths[task] + (child,)
            if child not in costs or cost > costs[child] or (cost == costs[child] and path < paths[child]):
                costs[child], paths[child] = cost, path
            indegrees[child] -= 1
            if indegrees[child] == 0:
                heapq.heappush(ready, child)
    if visited != tasks:
        raise ValueError('cyclic original task DAG')
    unseen, components = set(tasks), []
    while unseen:
        seed = min(unseen)
        pending, members = [seed], set()
        while pending:
            task = pending.pop()
            if task in members:
                continue
            members.add(task)
            pending.extend(neighbors[task] - members)
        unseen.difference_update(members)
        terminal = min(members, key=lambda task: (-costs[task], paths[task]))
        components.append(dict(component=min(members), tasks=sorted(members),
                               observed_task_cpu_ns=sum(weights[task] for task in members),
                               known_cpu_chain_ns=costs[terminal],
                               known_cpu_chain_tasks=list(paths[terminal])))
    return components
