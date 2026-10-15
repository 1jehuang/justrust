#!/usr/bin/env python3
"""Independent strict-positive oracle for original pool5 execution-flow facts.

No consumer assessment is an input. Dicts must be original decoded receipts.
Paths and bytes are also accepted and parsed with duplicate-key rejection.
Cuts are unknown ancestry barriers, never zero-cost work. No full span or W/S
is computed. Numeric IDs break ties only, never establish causal relations.
"""
from collections import Counter, defaultdict, deque
import json
from pathlib import Path
import re

try:  # Package import from the repo root and standalone PYTHONPATH=bench.
    from .observed_task_dag_oracle import native_task_oracle
except ImportError:
    from observed_task_dag_oracle import native_task_oracle

U64 = (1 << 64) - 1
SCOPE = 'observed-query-and-join-control-flow'
NODE_KEYS = {'id', 'kind', 'at_ns', 'segment_id', 'scope_id', 'task', 'outcome', 'gap_reason'}
KINDS = {'cpu_segment', 'scope_enter', 'scope_exit', 'fork', 'task_enter', 'task_exit', 'join', 'cut'}
EDGE_KINDS = {'strand', 'sync_call', 'sync_return', 'fork_branch', 'join_required', 'continuation'}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def uint(value):
    require(type(value) is int and 0 <= value <= U64, 'expected unsigned 64-bit integer')
    return value


def add(left, right):
    return uint(uint(left) + uint(right))


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, 'duplicate original JSON key')
        result[key] = value
    return result


def _read(value):
    if isinstance(value, Path):
        value = value.read_bytes()
    if isinstance(value, (bytes, str)):
        value = json.loads(value, object_pairs_hook=_pairs,
                           parse_constant=lambda _: require(False, 'nonfinite JSON value'))
    require(type(value) is dict, 'original receipt must be an object')
    return value


def _inventory(parent, singular, plural):
    rows = parent[plural]
    require(type(rows) is list and len(rows) <= 4_000_000, 'invalid flow inventory budget')
    require(uint(parent[singular + '_allocator_baseline']) == 1, 'invalid flow allocator baseline')
    high = uint(parent[singular + '_allocator_high_water'])
    expected = uint(parent['expected_' + plural])
    require(high > 0 and high - 1 == expected == len(rows), 'incomplete flow allocator inventory')
    result = {}
    for row in rows:
        ident = uint(row['id'])
        require(0 < ident < high and ident not in result, 'duplicate or out-of-range flow identity')
        result[ident] = row
    return result


def _query_lifetimes(query):
    require(query['schema'] in (5, 6) and query['producer'] == 'rustc-query-admission',
            'original reciprocal query5/6 required')
    require(query['scope'] == 'query-job-admission-lifecycle'
            and query['capture_complete'] is True and query['lost_events'] == 0
            and query['interface_outcome'] == 'returned', 'incomplete original query admission')
    require(query['full_pipeline_complete'] is False and query['cpu_work_ns'] is None
            and query['execution_span_ns'] is None, 'query promoted full metrics')
    contexts = {}
    for context in query['contexts']:
        ident = uint(context['context_id'])
        high = uint(context['allocator_high_water'])
        require(ident > 0 and ident not in contexts and uint(context['allocator_baseline']) == 1
                and high > 0 and high - 1 == uint(context['expected_jobs']),
                'invalid original query context inventory')
        contexts[ident] = high
    require(len(contexts) == query['expected_contexts'], 'missing original query contexts')
    starts, ends, parents = {}, {}, {}
    for index, event in enumerate(query['events']):
        key = uint(event['context_id']), uint(event['job_id'])
        require(key[0] in contexts and 0 < key[1] < contexts[key[0]], 'foreign original query job')
        at = uint(event['at_ns'])
        kind = event['kind']
        require(kind in ('admitted', 'completed', 'poisoned'), 'unknown query event kind')
        target = starts if kind == 'admitted' else ends
        require(key not in target, 'duplicate original query lifecycle endpoint')
        target[key] = index, at
        if kind == 'admitted':
            parents[key] = event['parent_job_id']
    require(starts.keys() == ends.keys(), 'query lacks actual original terminal')
    require(len(starts) == sum(high - 1 for high in contexts.values()), 'missing allocated query jobs')
    for key, (index, start) in starts.items():
        require(ends[key][0] > index and start <= ends[key][1], 'invalid original query lifetime')
        parent = parents[key]
        if parent is not None:
            require((key[0], uint(parent)) in starts and parent != key[1], 'foreign query parent')
            require(starts[key[0], parent][0] < index, 'query parent not previously admitted')
    return {key: (start, ends[key][1]) for key, (_, start) in starts.items()}


def _measurements(query, pool, lifetimes):
    timing = pool['timing']
    require(timing['schema'] == 2 and timing['scope'] == 'observed-exclusive-thread-segments'
            and timing['cpu_clock'] == 'linux-thread-cpu', 'timing2 original CPU measurements required')
    for flag in ('sealed', 'records_complete'):
        require(timing[flag] is True, 'incomplete original CPU measurements')
    for flag in ('identity_exhausted', 'loss_overflowed'):
        require(timing[flag] is False, 'unknown original CPU identity')
    require(all(timing[key] == 0 for key in ('lost_segments', 'active_segments', 'clock_failures')),
            'positive oracle rejects lost, active or failed CPU measurements')
    bridge = timing['clock_bridge']
    require(set(bridge) == {'query_capture_id', 'pool_capture_id', 'pool_observer', 'pool_epoch_in_query_ns'},
            'invalid reciprocal clock fields')
    require(query['execution_clock'] == bridge and bridge['query_capture_id'] == query['capture_id']
            and bridge['pool_capture_id'] == pool['capture_id']
            and bridge['pool_observer'] == pool['observer']
            and uint(bridge['pool_epoch_in_query_ns']) == 0, 'nonreciprocal original clock bridge')
    high = uint(timing['segment_allocator_high_water'])
    threads = uint(timing['thread_allocator_high_water'])
    require(timing['segment_allocator_baseline'] == 1 and high > 0 and threads > 0
            and high - 1 == uint(timing['expected_segments']) == len(timing['segments']),
            'incomplete original segment allocator')
    segments, names = {}, {}
    intervals = defaultdict(list)
    for row in timing['segments']:
        ident = uint(row['segment_id'])
        require(0 < ident < high and ident not in segments, 'duplicate or foreign timing segment')
        thread = uint(row['thread_id'])
        require(0 < thread < threads, 'foreign original thread identity')
        require(row['status'] == 'measured', 'positive oracle rejects unknown CPU measurements')
        start, end, cpu_start, cpu_end, cpu = [uint(row[key]) for key in
            ('start_ns', 'end_ns', 'cpu_start_ns', 'cpu_end_ns', 'cpu_ns')]
        require(start <= end and cpu_start <= cpu_end and cpu_end - cpu_start == cpu
                and cpu <= end - start, 'invalid original CPU endpoints/delta')
        owner = row['owner_kind']
        task, context, job, name = [row[key] for key in ('task', 'query_context', 'query_job', 'query_name')]
        require(owner in ('query_execution', 'task', 'opaque'), 'unknown original CPU category')
        if task is not None:
            require(uint(task) > 0 and owner != 'opaque', 'invalid original CPU task owner')
            intervals['task', task].append((start, end))
        else:
            require(owner != 'task', 'task CPU lacks actual task')
        if owner == 'query_execution':
            key = uint(context), uint(job)
            require(key in lifetimes and type(name) is str
                    and re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]{0,127}', name), 'unadmitted CPU query/category')
            require(names.setdefault(key, name) == name, 'inconsistent original query category')
            require(lifetimes[key][0] <= start <= end <= lifetimes[key][1],
                    'CPU outside original query lifetime')
            intervals['query', key].append((start, end))
        else:
            require(context is job is name is None, 'nonquery CPU carries query endpoints')
        intervals['thread', thread].append((start, end))
        segments[ident] = row
    for values in intervals.values():
        values.sort()
        require(all(a[1] <= b[0] for a, b in zip(values, values[1:])), 'overlapping original CPU intervals')
    require(segments and any(row['owner_kind'] == 'query_execution' for row in segments.values()),
            'positive flow acceptance requires original query CPU')
    return segments, names


def _known_chain(nodes, edges, costs):
    """Kahn traversal, explicit edges only. None costs erase ancestral chains."""
    outgoing, predecessors = defaultdict(list), defaultdict(list)
    degree = dict.fromkeys(nodes, 0)
    for left, right in sorted(edges):
        outgoing[left].append(right)
        predecessors[right].append(left)
        degree[right] += 1
    ready = deque(sorted(node for node in nodes if degree[node] == 0))
    best, visited = {}, 0
    while ready:
        node = ready.popleft()
        visited += 1
        if costs[node] is not None:
            choices = [(best[p][0], best[p][2], -p, p) for p in predecessors[node] if p in best]
            previous = max(choices)[3] if choices else None
            inherited = best[previous] if previous is not None else (0, None, False)
            best[node] = (add(inherited[0], costs[node]), previous,
                          inherited[2] or nodes[node]['kind'] == 'cpu_segment')
        for child in outgoing[node]:
            degree[child] -= 1
            if degree[child] == 0:
                ready.append(child)
    require(visited == len(nodes), 'cyclic original execution flow')
    measured = [node for node in best if best[node][2]]
    if not measured:
        return None, []
    terminal = max(measured, key=lambda node: (best[node][0], -node))
    path, cursor = [], terminal
    while cursor is not None:
        path.append(cursor)
        cursor = best[cursor][1]
    return best[terminal][0], path[::-1]


def _task_authority(pool):
    """Check original two-branch joins, including nested continuation terminals."""
    facts = native_task_oracle(pool)
    definitions, starts, ends, resumes = {}, {}, {}, {}
    groups, successors = {}, {}
    for event in pool['events']:
        kind, at = event['kind'], uint(event['at_ns'])
        if kind == 'root':
            definitions[event['task']] = ('root', None, at)
        elif kind == 'fork':
            parent, child, continuation = (uint(event[k]) for k in ('parent', 'child', 'continuation'))
            require(len({parent, child, continuation}) == 3, 'aliased original fork')
            group = groups.setdefault(continuation, {'parent': parent, 'branches': set(), 'joins': {}})
            require(group['parent'] == parent and child not in group['branches'], 'foreign original fork group')
            group['branches'].add(child)
            definitions[child] = ('child', None, at)
            if continuation not in definitions:
                require(parent not in successors, 'original task forked into multiple continuations')
                definitions[continuation] = ('continuation', parent, at)
                successors[parent] = continuation
        elif kind in ('start', 'finish'):
            task = uint(event['task'])
            (starts if kind == 'start' else ends)[task] = at
            if kind == 'finish':
                require(event['outcome'] in ('normal', 'panic'), 'invalid original task terminal')
        elif kind == 'resume':
            task = uint(event['continuation'])
            require(task not in resumes, 'duplicate original continuation resume')
            resumes[task] = at
        elif kind == 'join':
            continuation, branch, terminal = (uint(event[k]) for k in ('continuation', 'branch', 'child'))
            require(continuation in groups and branch not in groups[continuation]['joins'],
                    'duplicate or foreign original branch join')
            groups[continuation]['joins'][branch] = terminal, at
    require(set(definitions) == set(starts) == set(ends) == set(facts['tasks']), 'incomplete original task authority')
    for task, (kind, parent, defined) in definitions.items():
        require(0 < uint(task) < pool['task_allocator_high_water'] and defined <= starts[task] <= ends[task],
                'original task outside definition/lifetime')
        if kind == 'continuation':
            require(task in resumes and defined <= resumes[task] <= starts[task], 'incomplete original continuation resume')
        else:
            require(task not in resumes, 'noncontinuation task resumed')
    for continuation, group in groups.items():
        require(len(group['branches']) == 2 and set(group['joins']) == group['branches'],
                'incomplete original two-branch join obligations')
        parent = group['parent']
        for branch in group['branches']:
            require(starts[parent] <= definitions[branch][2] <= ends[parent] <= starts[branch],
                    'original fork outside parent/child lifetimes')
            terminal, notified = group['joins'][branch]
            cursor, seen = branch, set()
            while cursor in successors:
                require(cursor not in seen, 'cyclic original continuation lineage')
                seen.add(cursor)
                cursor = successors[cursor]
            require(terminal == cursor and terminal in ends and ends[terminal] <= notified <= resumes[continuation],
                    'original join bypasses branch terminal or valid notification time')
    return facts, {task: {'start': starts[task], 'finish': ends[task]} for task in starts}


def _execution_flow_oracle(query, pool):
    """Return independently reconstructed consumer-comparable strict-positive facts.

    Raises ValueError on loss/incompleteness, unknown CPU, unassigned segments,
    malformed references, nonreciprocal authority, or invalid topology. Cut nodes
    are permitted recorded barriers, not unknown CPU measurements. Inputs are
    never modified. `longest_known_chain_steps` is the original-byte witness.
    """
    query, pool = _read(query), _read(pool)
    # Python bool is an int subclass. Reject it for wire integers rather than
    # allowing True/False to alias original IDs or zero loss counters.
    for owner, keys in [(query, ('schema', 'lost_events', 'expected_contexts')),
                        (pool, ('schema', 'observer', 'dropped_events', 'task_allocator_baseline',
                                'task_allocator_high_water', 'expected_tasks')),
                        (pool['timing'], ('schema', 'segment_allocator_baseline', 'segment_allocator_high_water',
                                          'expected_segments', 'thread_allocator_high_water', 'lost_segments',
                                          'active_segments', 'clock_failures')),
                        (pool['execution_flow'], ('schema', 'active_scopes', 'lost_nodes', 'lost_edges', 'lost_scopes'))]:
        for key in keys:
            uint(owner[key])
    for key in ('pool_observer', 'pool_epoch_in_query_ns'):
        uint(pool['timing']['clock_bridge'][key])
    require(pool['schema'] == 5 and pool['producer'] == 'rustc-thread-pool-join'
            and pool['scope'] == 'native-join-continuations', 'original pool5 required')
    require(pool['sealed'] is True and pool['interface_outcome'] == 'returned'
            and pool['dropped_events'] == 0 and uint(pool['observer']) > 0,
            'incomplete original pool')
    require(pool['query_capture_id'] == query['capture_id'], 'foreign pool/query identity')
    require(all(pool[key] is False for key in
                ('full_pipeline_complete', 'scope_coverage', 'scheduler_coverage', 'pre_attach_coverage'))
            and pool['cpu_work_ns'] is None and pool['execution_span_ns'] is None,
            'pool promoted full metrics')
    require(pool['task_allocator_baseline'] == 1
            and uint(pool['task_allocator_high_water']) - 1 == uint(pool['expected_tasks']),
            'invalid original task allocator')
    lifetimes = _query_lifetimes(query)
    segments, names = _measurements(query, pool, lifetimes)
    task_facts, task_intervals = _task_authority(pool)
    flow = pool['execution_flow']
    expected_keys = {'schema', 'scope', 'full_pipeline_complete', 'flow_complete', 'sealed',
                     'records_complete', 'identity_exhausted', 'tls_failed', 'active_scopes',
                     'lost_nodes', 'lost_edges', 'lost_scopes', 'loss_overflowed', 'nodes', 'edges', 'scopes'}
    for singular, plural in [('node', 'nodes'), ('edge', 'edges'), ('scope', 'scopes')]:
        expected_keys.update((singular + '_allocator_baseline', singular + '_allocator_high_water', 'expected_' + plural))
    require(set(flow) == expected_keys and flow['schema'] == 1 and flow['scope'] == SCOPE,
            'invalid original execution flow schema/fields')
    require(flow['full_pipeline_complete'] is False and flow['flow_complete'] is False,
            'flow cannot attest full coverage')
    require(flow['sealed'] is True and flow['records_complete'] is True
            and all(flow[key] is False for key in ('identity_exhausted', 'tls_failed', 'loss_overflowed'))
            and all(flow[key] == 0 for key in ('active_scopes', 'lost_nodes', 'lost_edges', 'lost_scopes')),
            'positive oracle rejects incomplete or lost flow')
    nodes = _inventory(flow, 'node', 'nodes')
    edges = _inventory(flow, 'edge', 'edges')
    scopes = _inventory(flow, 'scope', 'scopes')
    gate_ids = set()
    for scope in scopes.values():
        require(set(scope) == {'id', 'query_context', 'query_job', 'query_name', 'entry', 'exit', 'outcome'},
                'invalid original scope fields')
        key = uint(scope['query_context']), uint(scope['query_job'])
        require(key in lifetimes and type(scope['query_name']) is str
                and re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]{0,127}', scope['query_name']),
                'scope lacks actual query admission/category')
        require(names.setdefault(key, scope['query_name']) == scope['query_name'],
                'scope changes original query category')
        require(scope['outcome'] in ('returned', 'unwound'), 'unfinished original scope')
        for field, kind in [('entry', 'scope_enter'), ('exit', 'scope_exit')]:
            ident = uint(scope[field])
            require(ident in nodes and ident not in gate_ids, 'missing or aliased original scope gate')
            gate_ids.add(ident)
            node = nodes[ident]
            require(node['kind'] == kind and node['scope_id'] == scope['id'], 'nonreciprocal scope gate')
            require(lifetimes[key][0] <= uint(node['at_ns']) <= lifetimes[key][1], 'scope gate outside query lifetime')
        require(nodes[scope['entry']]['at_ns'] <= nodes[scope['exit']]['at_ns']
                and nodes[scope['exit']]['outcome'] == scope['outcome'], 'invalid scope terminal')
    assigned, costs, totals = set(), {}, Counter()
    for ident, node in nodes.items():
        require(set(node) == NODE_KEYS and node['kind'] in KINDS, 'invalid original flow node fields/kind')
        at, kind = uint(node['at_ns']), node['kind']
        require((kind == 'cpu_segment') == (node['segment_id'] is not None)
                and (kind in ('scope_exit', 'task_exit')) == (node['outcome'] is not None)
                and (kind == 'cut') == (node['gap_reason'] is not None), 'flow node kind/nullable fields mismatch')
        require(node['outcome'] in (None, 'returned', 'unwound'), 'unknown flow outcome')
        require(node['gap_reason'] in (None, 'dispatch', 'unobserved_query', 'foreign_recorder', 'task_switch', 'missing_flow'),
                'unknown flow cut reason')
        task, scope_id = node['task'], node['scope_id']
        for value in (task, scope_id):
            if value is not None:
                require(uint(value) > 0, 'invalid nullable flow identity')
        if task is not None:
            require(task in task_intervals and task_intervals[task]['start'] <= at <= task_intervals[task]['finish'],
                    'flow node outside original task lifecycle')
        else:
            require(kind not in ('fork', 'task_enter', 'task_exit', 'join'), 'task gate lacks original task')
        if kind in ('scope_enter', 'scope_exit'):
            require(ident in gate_ids, 'extra original scope gate')
        if scope_id is not None:
            require(scope_id in scopes, 'foreign original scope')
            scope = scopes[scope_id]
            require(nodes[scope['entry']]['at_ns'] <= at <= nodes[scope['exit']]['at_ns'], 'node outside exact scope')
        if kind != 'cpu_segment':
            costs[ident] = None if kind == 'cut' else 0
            continue
        segment_id = uint(node['segment_id'])
        require(segment_id in segments and segment_id not in assigned, 'missing or reused exact CPU segment')
        assigned.add(segment_id)
        row = segments[segment_id]
        require(task == row['task'] and at == row['start_ns'], 'CPU flow differs from exact original segment')
        if row['owner_kind'] == 'query_execution':
            require(scope_id in scopes, 'query CPU missing invocation scope')
            scope = scopes[scope_id]
            require((scope['query_context'], scope['query_job'], scope['query_name']) ==
                    (row['query_context'], row['query_job'], row['query_name']), 'CPU scope query identity mismatch')
            require(row['end_ns'] <= nodes[scope['exit']]['at_ns'], 'CPU extends beyond exact scope exit')
        else:
            require(scope_id is None, 'nonquery CPU has invocation scope')
        costs[ident] = row['cpu_ns']
        totals[row['owner_kind']] = add(totals[row['owner_kind']], row['cpu_ns'])
    require(assigned == segments.keys(), 'positive flow has unassigned original CPU segments')
    pairs, incoming, outgoing = set(), Counter(), Counter()
    calls, returns, observed_tasks = {}, {}, set()
    for edge in edges.values():
        require(set(edge) == {'id', 'from', 'to', 'kind'} and edge['kind'] in EDGE_KINDS, 'invalid flow edge fields')
        left, right, kind = uint(edge['from']), uint(edge['to']), edge['kind']
        require(left in nodes and right in nodes and left != right and (left, right) not in pairs,
                'missing, self or duplicate original edge')
        pairs.add((left, right))
        incoming[right] += 1
        outgoing[left] += 1
        a, b = nodes[left], nodes[right]
        require(a['at_ns'] <= b['at_ns'], 'edge contradicts original event clock')
        if a['segment_id'] is not None:
            require(segments[a['segment_id']]['end_ns'] <= b['at_ns'], 'edge precedes original CPU END')
        same_scope = a['scope_id'] == b['scope_id']
        task_ok = (a['task'] == b['task'] or
                   (a['task'] is None and a['kind'] in ('scope_enter', 'scope_exit')) or
                   (b['task'] is None and b['kind'] in ('scope_enter', 'scope_exit', 'cut')))
        if kind == 'strand':
            require(same_scope and task_ok and a['kind'] not in ('scope_exit', 'task_exit', 'fork', 'join')
                    and b['kind'] not in ('scope_enter', 'task_enter', 'join'), 'invalid strand transition')
        elif kind == 'sync_call':
            require(b['kind'] == 'scope_enter' and not same_scope and task_ok
                    and a['kind'] not in ('scope_exit', 'task_exit', 'fork'), 'invalid synchronous call')
            require(b['scope_id'] not in calls, 'multiple callers for original invocation')
            calls[b['scope_id']] = a['scope_id']
        elif kind == 'sync_return':
            require(a['kind'] == 'scope_exit' and not same_scope and task_ok
                    and b['kind'] not in ('scope_enter', 'task_enter', 'join'), 'invalid synchronous return')
            require(a['scope_id'] not in returns, 'multiple returns from original invocation')
            returns[a['scope_id']] = b['scope_id']
        elif kind in ('fork_branch', 'join_required'):
            require((a['kind'] == 'fork' and b['kind'] == 'task_enter') if kind == 'fork_branch'
                    else (a['kind'] in ('task_exit', 'join') and b['kind'] == 'join'), 'invalid task control edge')
            relation = a['task'], b['task'], 'fork' if kind == 'fork_branch' else 'join'
            require(relation not in observed_tasks, 'duplicate flow task lineage')
            observed_tasks.add(relation)
        else:
            require(a['kind'] in ('join', 'cut') and same_scope and b['kind'] == 'cpu_segment'
                    and (a['kind'] == 'cut' or a['task'] == b['task']), 'invalid continuation edge')
    require(all(incoming[node] <= 1 or row['kind'] == 'join' for node, row in nodes.items())
            and all(outgoing[node] <= 1 or row['kind'] == 'fork' for node, row in nodes.items()), 'implicit fork/join')
    require(all(scope in calls and calls[scope] == parent for scope, parent in returns.items()),
            'nonreciprocal synchronous caller return')
    expected_tasks = {(left, right, task_facts['edge_kinds'][left, right]) for left, right in task_facts['edges']}
    require(observed_tasks == expected_tasks, 'flow differs from exact original task edges')
    maximum, path = _known_chain(nodes, pairs, costs)
    steps = []
    for ident in path:
        node = nodes[ident]
        scope = scopes.get(node['scope_id'])
        row = segments.get(node['segment_id'])
        steps.append(dict(node_id=ident, kind=node['kind'], at_ns=node['at_ns'], segment_id=node['segment_id'],
                          scope_id=node['scope_id'], task=node['task'], query_context=scope['query_context'] if scope else None,
                          query_job=scope['query_job'] if scope else None, query_name=row['query_name'] if row else None,
                          cpu_ns=costs[ident]))
    count = Counter(row['owner_kind'] for row in segments.values())
    return dict(scope=SCOPE, pool_capture_id=pool['capture_id'], pool_observer=pool['observer'],
                records_complete=True, full_pipeline_complete=False, flow_complete=False,
                nodes=len(nodes), edges=len(edges), scopes=len(scopes), active_scopes=0,
                lost_nodes=0, lost_edges=0, lost_scopes=0, assigned_cpu_segments=len(assigned),
                unassigned_cpu_segments=0, unassigned_known_cpu_ns=0, missing_measurement_references=0,
                unavailable_cpu_segments=0, cut_nodes=sum(n['kind'] == 'cut' for n in nodes.values()),
                known_cpu_work_ns=add(add(totals['query_execution'], totals['task']), totals['opaque']),
                known_query_cpu_ns=totals['query_execution'] if count['query_execution'] else None,
                known_task_cpu_ns=totals['task'] if count['task'] else None,
                known_opaque_cpu_ns=totals['opaque'] if count['opaque'] else None,
                longest_known_chain_cpu_ns=maximum, longest_known_chain_path=path, longest_known_chain_steps=steps,
                cpu_work_ns=None, execution_span_ns=None, parallelism=None)


def execution_flow_oracle(query, pool):
    """Strict original query5/6 + pool5/timing2 oracle, no input mutation.

    Inputs: original decoded dictionaries, JSON bytes/text, or pathlib.Path.
    Returns: consumer-named count/cost fields and independently derived longest
    known CPU-chain path/steps. Cuts break paths; full work/span/parallelism stay
    null. Raises ValueError for malformed, incomplete, lost or unknown authority.
    This is not a validator for unrelated query read/cache/dispatch subprotocols.
    """
    try:
        return _execution_flow_oracle(query, pool)
    except (KeyError, TypeError, IndexError) as error:
        raise ValueError(f'malformed original flow authority: {error}') from error
