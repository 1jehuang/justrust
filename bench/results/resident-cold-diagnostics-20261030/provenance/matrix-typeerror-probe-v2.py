#!/usr/bin/env python3
"""Root-only native differential. Never import this file to execute a native probe."""
import argparse
import hashlib
import json
import math
import os
import re
from pathlib import Path
import sys
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=Path('/home/jeremy/justrust'))
    parser.add_argument('--matrix-output', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--timeout', type=float, default=120)
    args = parser.parse_args()
    sys.path.insert(0, str(args.repo / 'bench'))
    import resident_state_matrix as matrix
    shared, runtime = matrix.shared, matrix.runtime
    old = shared.absolute(str(args.matrix_output))
    out = shared.absolute(str(args.output))
    if (out.exists() or out.is_relative_to(old) or old.is_relative_to(out)
            or len(os.fsencode(out)) > 60 or not math.isfinite(args.timeout)
            or not 0 < args.timeout <= 360):
        raise ValueError('fresh short disjoint output and finite timeout <=360 required')
    original_meta = json.loads((old / 'metadata.json').read_text())
    original_rows = json.loads((old / 'records.json').read_text())
    if not original_meta['unchanged'] or not original_meta['source_restored']:
        raise ValueError('original matrix must have completed and restored unchanged inputs')
    source = shared.absolute(original_meta['source'])
    if out.is_relative_to(source) or source.is_relative_to(out):
        raise ValueError('output must be disjoint from source')
    shared.pinned_source(source)
    tools = {name: shared.absolute(value['path']) for name, value in original_meta['tools_before'].items()}
    tc = shared.absolute(original_meta['toolchain_before']['path'])
    identities = lambda: {name: dict(path=str(path), sha256=runtime.base.digest(path))
                          for name, path in tools.items()}
    if identities() != original_meta['tools_before']:
        raise ValueError('tools no longer match original frozen native matrix')
    tc_before = shared.toolchain_identity(tc)
    if tc_before != original_meta['toolchain_before']:
        raise ValueError('toolchain no longer matches original frozen native matrix')
    if shared.source_hashes(source) != original_meta['source_before']:
        raise ValueError('source must match original pristine matrix fixture')
    frozen = {str(p): runtime.base.digest(p) for p in (
        args.repo / 'bench/resident_state_matrix.py',
        args.repo / 'bench/resident_engine_compare.py',
        args.repo / 'bench/resident_workload_compare.py',
        args.repo / 'bench/blaze_runtime_compare.py',
        args.repo / 'bench/blaze_edit_compare.py')}
    if frozen[str(args.repo / 'bench/resident_state_matrix.py')] != 'be64861cbbd8295cc103b219ee9e028daa478f301be91046e8d9d4b0ae6f8358':
        raise ValueError('matrix source is not frozen 5ec494e60')
    preserved = {str(old / name): runtime.base.digest(old / name)
                 for name in ('metadata.json', 'records.json', 'summary.json')}
    for row in original_rows:
        for receipt in row.get('raw', {}).values():
            path = Path(receipt['path'])
            if runtime.base.digest(path) != receipt['sha256']:
                raise ValueError('original matrix raw receipt changed: ' + str(path))
            preserved[str(path)] = receipt['sha256']
    for version in original_meta['versions'].values():
        for receipt in version['raw'].values():
            path = Path(receipt['path'])
            if runtime.base.digest(path) != receipt['sha256']:
                raise ValueError('original version receipt changed')
            preserved[str(path)] = receipt['sha256']
    out.mkdir(parents=True, mode=0o700)
    originals = {name: (source / name).read_bytes() for name in matrix.FILES}
    rows, backups, envs = [], {}, {}
    meta = dict(source=str(source), original_matrix=str(old), source_before=shared.source_hashes(source),
                tools_before=identities(), toolchain_before=tc_before, harness_before=frozen,
                original_receipts_before=preserved, status='incomplete', backups={},
                store_scope='existing original matrix s0 stores, no seeding or cold claims',
                policy='Diagnostic differential only. Generic HasErrors is not E0308. Original rejection retained.',
                environment_policy='full original helper reconstructed, private root paths remapped to existing s0; safe logging allowlist',
                planned_calls=8, variants=['canonical-fixed-uninstrumented', 'probe-only-fresh'],
                control_policy='Exact fixed original single-error assignment is observational only, never freshness-admitted. Recoveries use imported unique executed-marker edits.')
    expected = original_meta['contracts']['0:cold-prepare']['expected']
    if not expected:
        raise ValueError('missing original reference baseline contract')

    def save():
        for name, value in (('metadata.json', meta), ('records.json', rows)):
            (out / name).write_text(json.dumps(value, indent=2) + '\n')

    def restore():
        errors = []
        for name, backup in backups.items():
            try:
                data = backup.read_bytes()
                if hashlib.sha256(data).hexdigest() != meta['backups'][name]['sha256']:
                    raise ValueError('backup digest mismatch: ' + name)
                matrix.durable(source / name, data)
            except (OSError, ValueError) as exc:
                errors.append(repr(exc))
        if errors:
            raise ValueError('restoration errors ' + repr(errors))

    def worker_logs(env):
        root = Path(env['BLAZE_HOME']) / 'workers'
        paths = sorted(root.glob('*/log')) if root.is_dir() else []
        if len(paths) > 256:
            raise ValueError('worker log count exceeds bound')
        result = {}
        for path in paths:
            if path.is_symlink() or not path.is_file():
                raise ValueError('worker log must be an ordinary file')
            with path.open('rb') as stream:
                data = stream.read(matrix.MAX_RAW_BYTES + 1)
            if len(data) > matrix.MAX_RAW_BYTES:
                raise ValueError('worker log exceeds full-snapshot bound')
            result[str(path)] = data
        return result

    def retain_logs(before, after, prefix):
        receipts = []
        for index, path in enumerate(sorted(set(before) | set(after))):
            a, b = before.get(path), after.get(path)
            entry = dict(worker_log=path, before_present=a is not None, after_present=b is not None)
            for label, data in (('before', a), ('after', b)):
                if data is None:
                    continue
                target = prefix.with_name(prefix.name + f'-worker-{index}-{label}.log')
                matrix.durable(target, data, exclusive=True)
                entry[label] = dict(path=str(target), bytes=len(data), sha256=runtime.base.digest(target))
            append = a is not None and b is not None and b.startswith(a)
            delta = b[len(a):] if append else b
            entry['delta_kind'] = 'append' if append else 'new/replaced/deleted'
            if delta is not None:
                target = prefix.with_name(prefix.name + f'-worker-{index}-delta.log')
                matrix.durable(target, delta, exclusive=True)
                entry['delta'] = dict(path=str(target), bytes=len(delta), sha256=runtime.base.digest(target))
                entry['delta_has_E0308'] = b'E0308' in delta
            receipts.append(entry)
        return receipts

    try:
        for index, (name, data) in enumerate(originals.items()):
            backup = out / f'original-{index}'
            matrix.durable(backup, data, exclusive=True)
            meta['backups'][name] = dict(path=str(backup), sha256=runtime.base.digest(backup))
            backups[name] = backup
        plan = out / 'environment-plan'
        plan.mkdir(mode=0o700)
        for stack in matrix.STACKS:
            fresh = shared.environment(plan, stack, tc, tools.get('proc-macro-server'))
            # The original helper owns all controls. Only root-path values change.
            envs[stack] = {key: str(old / 's0') + value[len(str(plan)):]
                           if value.startswith(str(plan) + '/') else value
                           for key, value in fresh.items()}
            requested = matrix.workload.receipt_environment(envs[stack])
            if requested != original_meta['environments']['0:' + stack]:
                raise ValueError('helper reconstruction differs from original environment: ' + stack)
            for key in ('BLAZE_HOME', 'CARGO_HOME', 'CARGO_TARGET_DIR'):
                if not Path(envs[stack][key]).is_dir():
                    raise ValueError('original private state root missing: ' + key)
        meta['environments'] = {s: matrix.workload.receipt_environment(e) for s, e in envs.items()}
        meta['initial_worker_logs'] = {s: retain_logs({}, worker_logs(e), out / ('initial-' + s))
                                       for s, e in envs.items()}
        save()
        for variant in meta['variants']:
            for stack in matrix.STACKS:
                for phase in ('type-error', 'type-recovery'):
                    nonce = uuid.uuid4().hex
                    state = dict(originals)
                    if variant == 'probe-only-fresh':
                        state = matrix.baseline(originals)
                        state['src/summary.rs'] = state['src/summary.rs'].replace(matrix.SIGNATURE, b'', 1)
                    recovery_number = (1 << 60) + int(nonce[:15], 16)
                    marker = 'JR_RUNTIME_EXECUTED_' + str(recovery_number + 1)
                    if phase == 'type-error':
                        state['src/summary.rs'] = runtime.edit_bytes(state['src/summary.rs'], kind='type-error')
                        if variant == 'probe-only-fresh':
                            state['src/summary.rs'] = state['src/summary.rs'].replace(
                                b'JR_RUNTIME_TYPE_ERROR', ('JR_MATRIX_TYPE_' + nonce).encode(), 1)
                        equivalent = runtime.edit_bytes(originals['src/summary.rs'], kind='type-error')
                        canonical = state['src/summary.rs'].replace(matrix.PROBE, b'', 1).replace(
                            ('JR_MATRIX_TYPE_' + nonce).encode(), b'JR_RUNTIME_TYPE_ERROR', 1)
                    else:
                        state['src/summary.rs'] = runtime.edit_bytes(state['src/summary.rs'], recovery_number)
                        canonical = state['src/summary.rs'].replace(matrix.PROBE, b'', 1)
                        number = str(recovery_number + 1).encode()
                        if canonical.count(number) != 3:
                            raise ValueError('fresh imported recovery marker/value count mismatch')
                        canonical = canonical.replace(number, b'1')
                        equivalent = runtime.edit_bytes(originals['src/summary.rs'], 0)
                    if (canonical != equivalent or state['Cargo.toml'] != originals['Cargo.toml']
                            or state['src/shim.rs'].replace(matrix.PROBE, b'', 1) != originals['src/shim.rs']
                            or matrix.SIGNATURE in state['src/summary.rs']):
                        raise ValueError('variant differs beyond explicit probe/literal/recovery-marker controls')
                    before_logs = worker_logs(envs[stack])
                    for name, data in state.items():
                        if (source / name).read_bytes() != data:
                            matrix.durable(source / name, data)
                    before_inputs = shared.source_hashes(source)
                    env = dict(envs[stack], JR_MATRIX_NONCE=nonce)
                    prefix = out / f'{variant}-{stack}-{phase}'
                    row = dict(variant=variant, stack=stack, phase=phase, nonce=nonce,
                               before_phase_inputs=before_inputs,
                               normalized_summary_sha256=hashlib.sha256(canonical).hexdigest(),
                               environment=matrix.workload.receipt_environment(env), cwd=str(source))
                    rows.append(row)
                    save()
                    process = runtime.run_process(shared.command(tools[stack], stack), source, env, prefix, args.timeout)
                    process['environment'] = matrix.workload.receipt_environment(env)
                    row.update(process)
                    raw, text, raw_error = matrix.captured(prefix)
                    row.update(raw=raw, raw_error=raw_error, parsed=runtime.test_results(text),
                               input_unchanged=before_inputs == shared.source_hashes(source))
                    row['worker_logs'] = retain_logs(before_logs, worker_logs(env), prefix)
                    for entry in row['worker_logs']:
                        if 'delta' in entry:
                            entry['delta_has_current_nonce'] = nonce.encode() in Path(entry['delta']['path']).read_bytes()
                    row['direct_has_E0308'] = 'E0308' in text
                    row['direct_has_current_nonce'] = ('JR_MATRIX_TYPE_' + nonce in text if phase == 'type-error'
                                                       else marker in text)
                    row['observational_control'] = variant == 'canonical-fixed-uninstrumented' and phase == 'type-error'
                    row['direct_has_fixed_original_literal'] = 'JR_RUNTIME_TYPE_ERROR' in text
                    if phase == 'type-error':
                        if row['observational_control']:
                            row['observation_passed'] = (row['status'] == 101 and not row['timed_out']
                                and not row['error'] and not raw_error and row['input_unchanged']
                                and row['parsed'] is None and 'E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text
                                and not re.findall(r'JR_(?:MATRIX_RUNTIME|RUNTIME_EXECUTED)_[^\s]*', text))
                            row['failures'] = ['fixed canonical control is observational, not freshness-admitted']
                            row['accepted'] = False
                        else:
                            row['failures'] = matrix.oracle(row, text, expected, phase, nonce, 0)
                            row['accepted'] = not row['failures']
                    else:
                        failures = []
                        if (not runtime.base.succeeded(row) or raw_error or not row['input_unchanged']
                                or not matrix.workload.valid(row['parsed'], expected)):
                            failures.append('recovery process/input/exact test contract failed')
                        if re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*', text) != [marker]:
                            failures.append('imported fresh recovery marker/count failed')
                        probes = re.findall(r'JR_MATRIX_RUNTIME_[^\s]*', text)
                        wanted = ['JR_MATRIX_RUNTIME_' + nonce] if variant == 'probe-only-fresh' else []
                        if probes != wanted:
                            failures.append('variant-specific runtime probe/count mismatch')
                        row.update(failures=failures, accepted=not failures, expected_recovery_marker=marker)
                    save()
                    if row['timed_out'] or row['error'] or raw_error:
                        raise RuntimeError('probe aborts on timeout, launch or raw admission failure')
                restore()
    except BaseException as exc:
        meta['error'] = repr(exc)
        raise
    finally:
        unchanged = False
        meta['source_restored'] = False
        try:
            restore()
            meta['source_after'] = shared.source_hashes(source)
            meta['source_restored'] = meta['source_before'] == meta['source_after']
            meta['tools_after'] = identities()
            meta['toolchain_after'] = shared.toolchain_identity(tc)
            meta['harness_after'] = {p: runtime.base.digest(Path(p)) for p in frozen}
            meta['original_receipts_unchanged'] = all(runtime.base.digest(Path(p)) == h for p, h in preserved.items())
            meta['raw_receipts_unchanged'] = all(runtime.base.digest(Path(v['path'])) == v['sha256']
                for row in rows for v in row.get('raw', {}).values())
            unchanged = (all(meta[k + '_before'] == meta[k + '_after']
                             for k in ('source', 'tools', 'toolchain', 'harness'))
                         and meta['original_receipts_unchanged'] and meta['raw_receipts_unchanged']
                         and 'error' not in meta)
        except (OSError, ValueError) as exc:
            meta['identity_or_restoration_error'] = repr(exc)
            unchanged = False
        meta.update(unchanged=unchanged, status='complete' if unchanged and len(rows) == 8 else 'failed/incomplete')
        save()
        summary = dict(unchanged=unchanged, calls=len(rows), results=[dict(variant=r['variant'],
            stack=r['stack'], phase=r['phase'], status=r.get('status'), accepted=r.get('accepted', False),
            failures=r.get('failures', ['incomplete']), direct_has_E0308=r.get('direct_has_E0308'),
            direct_has_current_nonce=r.get('direct_has_current_nonce'),
            observational_control=r.get('observational_control'), observation_passed=r.get('observation_passed'),
            direct_has_fixed_original_literal=r.get('direct_has_fixed_original_literal')) for r in rows],
            policy=meta['policy'])
        (out / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    return 0 if unchanged and len(rows) == 8 and all(r.get('accepted') for r in rows) else 1


if __name__ == '__main__':
    raise SystemExit(main())
