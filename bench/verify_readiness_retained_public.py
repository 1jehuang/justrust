#!/usr/bin/env python3
"""Replay four retained native guard packets, never fabricate admission authority.

This deliberately scoped oracle expects these query-only packets to have no
qualified CPU. It is not a general missing-query-root CPU suppression rule.
Full CLI output is private evidence, only summary.json is suitable to commit.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--packets', type=Path, required=True)
parser.add_argument('--evidence', type=Path, required=True)
parser.add_argument('--justrust', default=shutil.which('justrust'))
parser.add_argument('--consumer-sha256', required=True)
args = parser.parse_args()
src, out, exe = args.packets.resolve(), args.evidence.resolve(), Path(args.justrust).resolve()
assert out != src and src not in out.parents, 'Keep evidence outside original packet directory'
out.mkdir(parents=True, exist_ok=False)

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def objects(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from objects(child)
    elif isinstance(value, list):
        for child in value:
            yield from objects(child)

assert digest(exe) == args.consumer_sha256
patterns = [
    '*waiter_beating_post_unlock_publication_finalization_is_explicitly_unbound-11.pool.json',
    '*cycle_poison_and_unwind_restore_caller_with_explicit_cuts-2.pool.json',
    '*active_aborted_poisoned_foreign_and_original_wait_exhaustion_never_transfer-1.pool.json',
    '*registered_and_already_ready_have_fresh_encounters_and_two_exact_obligations-10.pool.json',
]
cases, hashes = [], {}
active_unwind = registered_null = False
for pattern in patterns:
    files = list(src.glob(pattern))
    assert len(files) == 1, pattern
    path = files[0]
    hashes[path.name] = digest(path)
    flow = json.loads(path.read_bytes())['execution_flow']
    nodes = {node['id']: node for node in flow['nodes']}
    for wait in flow['wait_attempts']:
        active_unwind |= wait['mode'] == 'active' and wait['outcome'] == 'unwound'
        registered_null |= wait['mode'] == 'registered' and wait['wait_id'] is None
        reason = {'consumed': 'wake_to_consume', 'consumed_unbound': 'readiness_unavailable',
                  'cycle': 'wait_cycle', 'poisoned': 'wait_poisoned', 'unwound': 'wait_unwound'}[wait['outcome']]
        assert nodes[wait['wake_gap']]['gap_reason'] == reason
        if wait['outcome'] != 'consumed':
            assert wait['consumed'] is None
            assert not any(edge['from'] == wait['wake_gap'] and edge['kind'] in ('wait_required', 'readiness_required') for edge in flow['edges'])
        else:
            incoming = [edge for edge in flow['edges'] if edge['to'] == wait['consumed']]
            assert sorted(edge['kind'] for edge in incoming) == ['readiness_required', 'wait_required']
            ready = next(row for row in flow['readiness'] if row['id'] == wait['readiness'])
            assert ready['outcome'] == 'published'
            assert (ready['query_context'], ready['query_job']) == (wait['query_context'], wait['waitee_job'])
            assert any(edge['kind'] == 'wait_required' and edge['from'] == wait['wake_gap'] for edge in incoming)
            assert any(edge['kind'] == 'readiness_required' and edge['from'] == ready['visibility_gap'] for edge in incoming)
    for mode, flags in [('default', []), ('query-cpu', ['--query-cpu-ownership']), ('flow', ['--execution-flow-dag']), ('readiness', ['--latch-readiness-dag'])]:
        result = subprocess.run([str(exe), 'graph', str(path), *flags, '--json'], capture_output=True, timeout=120)
        (out / (path.name + '.' + mode + '.stdout')).write_bytes(result.stdout)
        (out / (path.name + '.' + mode + '.stderr')).write_bytes(result.stderr)
        assert result.returncode == (0 if mode == 'readiness' else 1)
        if mode != 'readiness':
            assert b'unsupported join protocol' in result.stderr
        case = {'source': path.name, 'mode': mode, 'exit_code': result.returncode}
        if mode == 'readiness':
            reports = [value['latch_readiness_dag'] for value in objects(json.loads(result.stdout)) if 'latch_readiness_dag' in value]
            assert len(reports) == 1
            report = reports[0]
            assert all(report[key] is False for key in ['records_complete', 'ready_records_complete', 'wait_records_complete', 'full_pipeline_complete', 'flow_complete'])
            assert all(report[key] is None for key in ['known_cpu_work_ns', 'known_query_cpu_ns', 'known_task_cpu_ns', 'known_opaque_cpu_ns', 'longest_known_chain_cpu_ns', 'cpu_work_ns', 'execution_span_ns', 'parallelism'])
            assert report['longest_known_chain_path'] == report['longest_known_chain_steps'] == []
            assert report['matched_readiness_transfers'] == 0
            assert report['readiness_rows'] == len(flow['readiness']) and report['wait_attempts'] == len(flow['wait_attempts'])
            assert report['unbound_wait_attempts'] == sum(wait['outcome'] == 'consumed_unbound' for wait in flow['wait_attempts'])
            case.update(wait_attempts=report['wait_attempts'], unbound_wait_attempts=report['unbound_wait_attempts'], all_authority_unknown=True)
        cases.append(case)
    assert digest(path) == hashes[path.name]
assert active_unwind and registered_null
assert digest(exe) == args.consumer_sha256
summary = {'schema': 1, 'source': 'genuine-retained-native-readiness-packets', 'consumer_sha256': args.consumer_sha256, 'original_sha256': hashes, 'cases': cases, 'active_mode_unwound_checked': active_unwind, 'registered_null_checked': registered_null, 'all_checks_passed': True, 'originals_unchanged': True, 'consumer_unchanged': True, 'limitations': ['Pool-only guard-test packets have no original admission/wait root. Acceptance proves typed partial handling and exact native failure/success role shape, not qualified positive readiness transfers.', 'These four packets have no independently qualified nonquery CPU. This oracle does not require suppressing genuine nonquery CPU in other partial captures.']}
(out / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps(summary, indent=2))
