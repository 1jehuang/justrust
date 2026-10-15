#!/usr/bin/env python3
"""Independent original-record authority for explicitly opted-in latch readiness.

Readiness is an observed prerequisite, not provider freshness or execution span.
A completed wait notification is not itself an exact result-consumption edge.
Positive acceptance requires complete lossless original inventories. Failed,
poisoned, cyclic and unbound encounters belong in separate negative controls.
"""
from collections import defaultdict

try:
    from .execution_flow_oracle import require, uint
except ImportError:
    from execution_flow_oracle import require, uint

SCOPE = 'observed-query-join-and-latch-control-flow'


def original_waits(query, lifetimes):
    """Reconstruct all four actual wait stages without consumer projections.

    Exhausted original wait allocators cannot authorize a successful registered
    encounter. Positive cohorts require independent, complete contiguous IDs.
    """
    require(uint(query['wait_allocator_baseline']) == 1, 'invalid original wait allocator baseline')
    high, expected = uint(query['wait_allocator_high_water']), uint(query['expected_waits'])
    require(high > 0 and high - 1 == expected and query['wait_capture_complete'] is True,
            'positive readiness requires complete unexhausted original waits')
    stages = {'registered': 0, 'notified': 1, 'condvar_returned': 2, 'resumed': 3}
    waits = {}
    terminals = {(row['context_id'], row['job_id']): row['kind']
                 for row in query['events'] if row['kind'] in ('completed', 'poisoned')}
    for index, event in enumerate(query['wait_events']):
        kind = event['kind']
        require(kind in stages, 'unknown original wait stage')
        fields = {'kind', 'wait_id', 'context_id', 'waitee_job_id', 'consumer_job_id', 'at_ns'}
        require(set(event) == fields | ({'reason'} if kind == 'notified' else set()),
                'invalid original wait event fields')
        ident, context, waitee = (uint(event[key]) for key in ('wait_id', 'context_id', 'waitee_job_id'))
        consumer = event['consumer_job_id']
        if consumer is not None:
            require(uint(consumer) > 0 and (context, consumer) in lifetimes, 'unadmitted original wait consumer')
        require(0 < ident < high and (context, waitee) in lifetimes, 'foreign original wait identity/waitee')
        endpoints = context, waitee, consumer
        row = waits.setdefault(ident, {'endpoints': endpoints, 'stages': {}, 'reason': None})
        require(row['endpoints'] == endpoints and kind not in row['stages'],
                'duplicate or inconsistent original wait lifecycle')
        row['stages'][kind] = index, uint(event['at_ns'])
        if kind == 'notified':
            require(event['reason'] in ('completed', 'poisoned', 'cycle'), 'unknown original wait reason')
            row['reason'] = event['reason']
    require(len(waits) == expected, 'missing original allocated waits')
    for row in waits.values():
        require(set(row['stages']) == set(stages), 'incomplete original wait stages')
        ordered = [row['stages'][kind] for kind in stages]
        require(all(a[0] < b[0] and a[1] <= b[1] for a, b in zip(ordered, ordered[1:])),
                'invalid original wait stage chronology')
        context, waitee, consumer = row['endpoints']
        registered, notified, _, resumed = [stage[1] for stage in ordered]
        require(lifetimes[context, waitee][0] <= registered, 'wait registered before producer admission')
        if consumer is not None:
            start, end = lifetimes[context, consumer]
            require(start <= registered <= resumed <= end, 'original wait outside consumer admission')
        if row['reason'] in ('completed', 'poisoned'):
            require(terminals[context, waitee] == row['reason'], 'original wait reason/producer terminal mismatch')
    return waits


def _rows(flow, prefix, plural):
    require(uint(flow[prefix + '_allocator_baseline']) == 1, 'invalid independent readiness allocator baseline')
    high, count = uint(flow[prefix + '_allocator_high_water']), uint(flow['expected_' + plural])
    require(high > 0 and high - 1 == count == len(flow[plural]) and count <= 100_000
            and uint(flow['lost_' + plural]) == 0 and uint(flow['active_' + plural]) == 0,
            'positive readiness requires lossless complete row inventories')
    rows = {}
    for row in flow[plural]:
        ident = uint(row['id'])
        require(0 < ident < high and ident not in rows, 'duplicate or foreign readiness row identity')
        rows[ident] = row
    return rows


def _readiness_authority(query, pool, nodes, edges, scopes, lifetimes, *, allow_unbound=False):
    """Validate row-indexed control obligations before the generic CPU DAG pass."""
    flow = pool['execution_flow']
    for key in ('pool_observer', 'pool_epoch_in_query_ns'):
        uint(query['execution_clock'][key])
    ready = _rows(flow, 'readiness', 'readiness')
    attempts = _rows(flow, 'wait_attempt', 'wait_attempts')
    waits = original_waits(query, lifetimes)
    terminal = {(row['context_id'], row['job_id']): (row['kind'], row['at_ns'])
                for row in query['events'] if row['kind'] in ('completed', 'poisoned')}
    triples = {(uint(edge['from']), uint(edge['to']), edge['kind']) for edge in edges.values()}
    require(len(triples) == len(edges), 'duplicate original readiness edge')
    incoming, outgoing = defaultdict(set), defaultdict(set)
    for left, right, kind in triples:
        require(left in nodes and right in nodes, 'readiness edge lacks retained original endpoint')
        incoming[right].add((left, right, kind))
        outgoing[left].add((left, right, kind))
    role_nodes, special, visibility_nodes, consumed_nodes = set(), set(), set(), set()
    producer_keys = set()
    expected_visibility_out = defaultdict(set)

    def gate(ident, kind, scope, reason=None):
        ident = uint(ident)
        require(ident in nodes and ident not in role_nodes, 'missing or aliased readiness role gate')
        role_nodes.add(ident)
        node = nodes[ident]
        require(node['kind'] == kind and node['scope_id'] == scope
                and node['gap_reason'] == reason and node['segment_id'] is None
                and node['outcome'] is None, 'readiness role differs from original node')
        uint(node['at_ns'])
        return node

    for row in ready.values():
        require(set(row) == {'id', 'query_context', 'query_job', 'prefix', 'prerequisite', 'visibility_gap', 'outcome'},
                'invalid original ReadyRow fields')
        key = uint(row['query_context']), uint(row['query_job'])
        require(key in lifetimes and key not in producer_keys, 'foreign or duplicate producer readiness identity')
        producer_keys.add(key)
        require(row['outcome'] == 'published' and terminal[key][0] == 'completed',
                'positive readiness rejects failed or unpublished producer')
        prerequisite = gate(row['prerequisite'], 'readiness_prerequisite', None)
        visibility = gate(row['visibility_gap'], 'cut', None, 'latch_visibility')
        require(prerequisite['task'] is visibility['task'] is None, 'producer readiness gates invent task ownership')
        require(lifetimes[key][0] <= prerequisite['at_ns'] <= visibility['at_ns'] <= terminal[key][1],
                'producer prerequisite outside preterminal admission')
        pair = row['prerequisite'], row['visibility_gap'], 'strand'
        require(incoming[row['visibility_gap']] == {pair} and outgoing[row['prerequisite']] == {pair},
                'visibility gap not exact own readiness prerequisite')
        special.add(pair)
        prefix = row['prefix']
        if prefix is None:
            require(not incoming[row['prerequisite']], 'root readiness prerequisite invents prefix')
        else:
            require(uint(prefix) in nodes, 'readiness prefix not an original retained cursor')
            pair = prefix, row['prerequisite'], 'readiness_prepare'
            require(incoming[row['prerequisite']] == {pair}, 'readiness_prepare not exact saved prefix')
            special.add(pair)
        visibility_nodes.add(row['visibility_gap'])

    registered_ids, consumed_count, unbound_count = set(), 0, 0
    for row in attempts.values():
        require(set(row) == {'id', 'query_context', 'waitee_job', 'consumer_job', 'caller_scope', 'wait_id',
                             'mode', 'outcome', 'begin', 'wait_gap', 'readiness', 'wake_gap', 'consumed'},
                'invalid original WaitRow fields')
        context, waitee = uint(row['query_context']), uint(row['waitee_job'])
        unbound = allow_unbound and row['outcome'] == 'consumed_unbound'
        consumer = row['consumer_job']
        if consumer is not None:
            consumer = uint(consumer)
            require(consumer > 0 and consumer != waitee and (context, consumer) in lifetimes,
                    'foreign original wait consumer')
        require((context, waitee) in lifetimes and (unbound or consumer is not None),
                'unbound or foreign original wait endpoints')
        require((row['outcome'] == 'consumed' or unbound) and row['mode'] in ('registered', 'already_ready'),
                'positive readiness rejects cycle, poison, unwind, active or unbound encounter')
        scope_id = row['caller_scope']
        if scope_id is not None:
            scope_id = uint(scope_id)
            require(scope_id in scopes, 'wait caller lacks exact original invocation')
            scope = scopes[scope_id]
            require((scope['query_context'], scope['query_job']) == (context, consumer)
                    and scope['outcome'] == 'returned', 'saved caller is another or unsuccessful invocation')
        else:
            require(unbound, 'successful wait lacks saved invocation')
            scope = None
        begin = gate(row['begin'], 'wait_begin', scope_id)
        gap = gate(row['wait_gap'], 'cut', scope_id, 'query_wait')
        wake = gate(row['wake_gap'], 'cut', scope_id,
                    'readiness_unavailable' if unbound else 'wake_to_consume')
        consumed = None if unbound else gate(row['consumed'], 'wait_consumed', scope_id)
        if unbound:
            require(row['consumed'] is None, 'unbound encounter invents successful consumption gate')
        require(gap['task'] is None and not incoming[row['wait_gap']],
                'wait/help gap must be an independent root, never caller ancestry')
        # Native restores this exact detached stream root before opening the
        # first genuine helper CPU interval. Its null task is neutral, not a
        # caller-prefix or readiness transfer. Generic v1 cuts gain no role.
        for triple in outgoing[row['wait_gap']]:
            _, target, kind = triple
            destination = nodes[target]
            require(kind in ('strand', 'continuation') and destination['kind'] == 'cpu_segment'
                    and destination['scope_id'] == scope_id,
                    'query wait root must resume actual CPU in exact saved caller scope')
            special.add(triple)
        end_at = wake['at_ns'] if unbound else consumed['at_ns']
        require(begin['at_ns'] <= gap['at_ns'] <= wake['at_ns'] <= end_at,
                'invalid native wait encounter chronology')
        if scope is not None:
            entry, exit = nodes[scope['entry']]['at_ns'], nodes[scope['exit']]['at_ns']
            require(entry <= begin['at_ns'] <= end_at <= exit,
                    'wait escapes saved exact invocation lifetime')
        if consumer is not None:
            start, end = lifetimes[context, consumer]
            require(start <= begin['at_ns'] <= end_at <= end, 'wait escapes consumer admission lifetime')
        ready_id = row['readiness']
        producer = None
        if ready_id is not None:
            ready_id = uint(ready_id)
            require(ready_id in ready, 'wait transfer lacks same original published readiness row')
            producer = ready[ready_id]
            require((producer['query_context'], producer['query_job']) == (context, waitee),
                    'readiness transfer crosses original latch/producer')
        require(unbound or producer is not None, 'successful transfer lacks published readiness')
        if producer is not None:
            prerequisite = nodes[producer['prerequisite']]
            visibility = nodes[producer['visibility_gap']]
            if not unbound:
                require(visibility['at_ns'] <= end_at, 'consumption precedes published prerequisite visibility')
        if row['mode'] == 'registered':
            wait_id = uint(row['wait_id'])
            require(wait_id in waits and wait_id not in registered_ids,
                    'registered encounter missing or reusing original wait lifecycle')
            registered_ids.add(wait_id)
            original = waits[wait_id]
            require(original['endpoints'] == (context, waitee, consumer) and original['reason'] == 'completed',
                    'registered encounter differs from original consumer/waitee or completed reason')
            times = {kind: value[1] for kind, value in original['stages'].items()}
            require(begin['at_ns'] <= times['registered'] <= times['resumed'] <= wake['at_ns']
                    and (producer is None or visibility['at_ns'] <= times['notified']),
                    'registered wait/readiness chronology contradicts original lifecycle')
        else:
            require(row['wait_id'] is None, 'already-ready encounter fabricates registered wait')
        boundary = row['begin'], row['wake_gap'], 'wait_boundary'
        require(outgoing[row['begin']] == {boundary} and incoming[row['wake_gap']] == {boundary},
                'wait boundary differs from original encounter')
        if unbound:
            require(all(kind == 'continuation' and nodes[right]['kind'] == 'cpu_segment'
                        and nodes[right]['scope_id'] == scope_id
                        for _, right, kind in outgoing[row['wake_gap']]),
                    'unbound wake invents consumption obligation')
            special.add(boundary)
            unbound_count += 1
            continue
        required = row['wake_gap'], row['consumed'], 'wait_required'
        transfer = producer['visibility_gap'], row['consumed'], 'readiness_required'
        require(outgoing[row['begin']] == {boundary} and incoming[row['wake_gap']] == {boundary}
                and outgoing[row['wake_gap']] == {required} and incoming[row['consumed']] == {required, transfer},
                'successful consumption lacks exact two original same-row obligations')
        special.update((boundary, required, transfer))
        expected_visibility_out[producer['visibility_gap']].add(transfer)
        consumed_nodes.add(row['consumed'])
        consumed_count += 1
    require(registered_ids == waits.keys(), 'original registered waits missing from independent attempts')
    for visibility in visibility_nodes:
        require(outgoing[visibility] == expected_visibility_out[visibility],
                'visibility gap has unrelated or invented normal continuation')
    new_kinds = {'readiness_prerequisite', 'wait_begin', 'wait_consumed'}
    new_gaps = {'latch_visibility', 'query_wait', 'wake_to_consume', 'readiness_unavailable',
                'wait_cycle', 'wait_poisoned', 'wait_unwound'}
    for ident, node in nodes.items():
        if node['kind'] in new_kinds or node['gap_reason'] in new_gaps:
            require(ident in role_nodes, 'unclaimed readiness gate or gap without exact row authority')
    # Native resumes a real exclusive CPU interval before the guard returns,
    # including zero-CPU measurements. No no-CPU continuation role is implied.
    # Nullable source task is neutral, never fabricated task ancestry.
    for consumed in consumed_nodes:
        source = nodes[consumed]
        for triple in outgoing[consumed]:
            _, target, kind = triple
            destination = nodes[target]
            if kind == 'continuation' and destination['kind'] == 'cpu_segment':
                require(destination['scope_id'] == source['scope_id']
                        and (source['task'] is None or destination['task'] == source['task']),
                        'consumed continuation crosses caller/task')
                special.add(triple)
    for triple in triples:
        if triple[2] in ('readiness_prepare', 'wait_boundary', 'wait_required', 'readiness_required'):
            require(triple in special, 'unclaimed readiness edge without exact row authority')
    return dict(special_edges=special, consumed_nodes=consumed_nodes, visibility_nodes=visibility_nodes,
                counts=dict(ready_records_complete=True, wait_records_complete=not unbound_count,
                            readiness_rows=len(ready), active_readiness=0, lost_readiness=0,
                            wait_attempts=len(attempts), active_wait_attempts=0, lost_wait_attempts=0,
                            matched_readiness_transfers=consumed_count, unbound_wait_attempts=unbound_count,
                            unavailable_readiness_references=0))


def latch_readiness_oracle(query, pool, *, allow_unbound=False):
    """Strict-positive original pool6/flow2 oracle, explicit opt-in only.

    Returns consumer-compatible observed flow counts/costs/witness plus readiness
    counters. Registered and already-ready encounters are distinguished using
    original rows, never by timing heuristics. No source dictionary is mutated.
    Default failure/unknown controls are rejected, not silently certified.
    Explicit allow_unbound=True also admits complete original consumed_unbound
    rows as conservative barriers, never successful transfers. Their exact count
    is retained and records_complete/wait_records_complete remain false. Other
    failures, loss, unknown measurements and allocator exhaustion still reject.
    """
    try:
        from .execution_flow_oracle import execution_flow_oracle
    except ImportError:
        from execution_flow_oracle import execution_flow_oracle
    return execution_flow_oracle(query, pool, readiness=True, allow_unbound=allow_unbound)
