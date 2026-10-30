#!/usr/bin/env python3
"""Portable rsm02 replay plus explicitly separate intrinsic-only error derivative."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re

HERE = Path(__file__).resolve().parent
BASE_SHA = 'c68f6d4051dba968ddc2623f3dbcd013614ec6db42d2c31a7560cf0c0a688e5f'
PRIVATE = {'metadata.json': 'f2f5299e94362e74fcb258f944966b340bc5f9774c570d2d47fa0f12a29c705d', 'records.json': '23a6f4ac9142244d7a1bcea32da1a011f308ab05998011e2d715eb00d029615f', 'summary.json': 'f428c7b5baca23a180e7369815c90f210c2a67568371346dc5cfaa8b7c22fe06'}
TOOLS = {'reference': 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263', 'engine': 'c2606d25eb820ee515f5c5ae9d42f803916a4e3af974adb872c2fc4ba31c4e8b'}


def require(ok, message):
    if not ok:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def base(root):
    path = root / 'provenance/matrix-evidence-validator.py'
    require(sha(path.read_bytes()) == BASE_SHA, 'frozen portable helper drift')
    spec = importlib.util.spec_from_file_location('frozen_rsm02_base', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    # These are the only data bindings changed from the retained historical parser.
    # Its raw/restoration/source/environment checks are not replaced or weakened.
    module.TOOLS = TOOLS
    module.PRIVATE_SHA['rsm02'] = PRIVATE
    return module


def environment(b, matrix, stack, rep):
    return {key: value.replace('/home/jeremy/rsm01/', '/home/jeremy/rsm02/')
            for key, value in b.expected_environment(matrix, stack, rep).items()}


# Ordered exact replay derived from the frozen matrix-evidence-validator helper.
# Only run identity, environment root and explicit intrinsic-vs-cross-engine error
# observations differ. Reference gating and original medians remain unchanged.
def check_matrix(root, matrix, originals, source_hashes, b):
    directory, meta, rows, summary, _ = b.run_data(root, 'rsm02', matrix, source_hashes)
    require(meta['status'] == 'failed/incomplete' and meta['samples'] == 5 and meta['pin'] == 'f97fcc2'
            and meta['phases'] == list(matrix.PHASES), 'failed matrix scope/status changed')
    require(meta['counts'] == {p: 10 for p in matrix.PHASES} and meta['planned_workload_calls'] == 140
            and meta['planned_version_calls'] == 10 and meta['toolchain_links_valid'] is True, 'phase counts/link gate')
    schedule = [(s, p, rep) for rep in range(5) for p in matrix.PHASES for s in matrix.runtime.order(matrix.STACKS, rep)]
    require(len(rows) == 140 and [(r['stack'], r['phase'], r['replicate']) for r in rows] == schedule, 'matrix exact phase/rotation schedule')
    require(len(meta['versions']) == 10, 'version invocation count')
    for rep in range(5):
        for stack in matrix.STACKS:
            env = environment(b, matrix, stack, rep)
            require(meta['environments'][f'{rep}:{stack}'] == env, 'private store/profile isolation drift')
            version = meta['versions'][f'{rep}:{stack}']
            row = version['process']
            text = b.raw_text(directory, 'rsm02', row)
            require(row['command'] == [meta['tools_before'][stack]['path'], '--version']
                    and row['status'] == 0 and version['valid'] is True and row['environment'] == env, 'native version receipt')
            pattern = re.escape(matrix.shared.REFERENCE_VERSION) + '$' if stack == 'reference' else r'jr-engine\s+\S+'
            require(re.search('^' + pattern, text, re.M), 'native version identity')
    previous, contracts, nonces, replay, intrinsic = {}, {}, set(), [], []
    for rep in range(5):
        expected_summary = None
        for phase in matrix.PHASES:
            pair = {}
            for stack in matrix.runtime.order(matrix.STACKS, rep):
                row = rows[len(replay)]
                nonce = row['nonce']
                require(re.fullmatch(r'[0-9a-f]{32}', nonce) and nonce not in nonces, 'nonce reused/invalid')
                nonces.add(nonce)
                state = matrix.inputs(originals, phase, rep, nonce)
                inputs = dict(source_hashes, **{n: sha(data) for n, data in state.items()})
                key = f'{rep}:{stack}'
                require(row['before_phase_inputs'] == inputs and row['previous_stack_inputs'] == previous.get(key)
                        and row['changed_inputs_since_stack_phase'] == [n for n in inputs if inputs[n] != previous.get(key, {}).get(n)],
                        'exact before-phase source/transition hash mismatch')
                previous[key] = inputs
                require(row['cache_scope'] == key and row['cwd'] == meta['source'] and row['input_unchanged'] is True,
                        'cache/source scope mismatch')
                require(row['environment'] == environment(b, matrix, stack, rep), 'process environment drift')
                wanted = matrix.shared.command(meta['tools_before'][stack]['path'], stack, arguments=matrix.arguments(phase))
                require(row['command'] == wanted, 'direct command/filter mismatch')
                require(row['status'] == (101 if phase in ('type-error', 'assert-failure') else 0), 'native exit status mismatch')
                text = b.raw_text(directory, 'rsm02', row)
                parsed = matrix.runtime.test_results(text)
                require(b.normalized(parsed) == row['parsed'], 'raw libtest parser disagrees with row')
                copied = dict(row, parsed=parsed)
                pair[stack] = copied, text, nonce
                replay.append(copied)
            reference, text, nonce = pair['reference']
            expected, count = expected_summary, None
            if phase in ('cold-prepare', 'ordinary-shim-edit', 'all-lib'):
                expected = matrix.contract(reference['parsed'], phase)
                if phase == 'cold-prepare':
                    expected_summary = expected
            if phase == 'ordinary-shim-edit':
                markers = re.findall(r'JR_WORKLOAD_EXECUTED_[^\s]*', text)
                if markers and markers == [f'JR_WORKLOAD_EXECUTED_shim_function_{rep + 1}'] * len(markers):
                    count = len(markers)
            errors = matrix.oracle(reference, text, expected, phase, nonce, rep, count)
            require(reference['failures'] == errors and reference['accepted'] == (not errors), 'reference oracle admission drift')
            if errors:
                expected = None
            if phase == 'cold-prepare' and errors:
                expected_summary = None
            contracts[f'{rep}:{phase}'] = dict(expected=expected, ordinary_invocations=count, reference_accepted=not errors)
            candidate, text, nonce = pair['engine']
            errors = matrix.oracle(candidate, text, expected, phase, nonce, rep, count)
            if not reference['accepted']:
                errors.append('reference state oracle unavailable/failed')
            require(candidate['failures'] == errors and candidate['accepted'] == (not errors), 'candidate oracle admission drift')
            for row, text, nonce in pair.values():
                if phase == 'type-error':
                    require(row['accepted'] is False and row['parsed'] is None, 'original type-error cross-engine exclusion must remain')
                    if row['stack'] == 'reference':
                        require('HasErrors' in text and 'E0308' not in text and nonce not in text, 'reference diagnostic negatives remain rejected')
                    else:
                        failures = matrix.oracle(row, text, None, phase, nonce, rep, count)
                        require(failures == [] and row['failures'] == ['reference state oracle unavailable/failed'], 'candidate intrinsic oracle must pass without relabeling cross-engine exclusion')
                        intrinsic.append(dict(replicate=rep, stack='engine', phase=phase, nonce=nonce,
                            original_cross_engine_accepted=False, original_failures=row['failures'],
                            original_status=row['status'], raw=row['raw'], intrinsic_failures=failures,
                            intrinsic_accepted=True, summary_source_sha256=row['before_phase_inputs']['src/summary.rs']))
                else:
                    require(row['accepted'] is True, 'unexpected rejection outside ten diagnostic negatives')
    require(b.normalized(contracts) == meta['contracts'], 'actual reference contract drift')
    result = matrix.summarize(replay, 5, True)
    require(summary == dict(states=result, policy=matrix.POLICY, identities_unchanged=True), 'per-state median/exclusion summary drift')
    return dict(rows=140, accepted=130, rejected=10, states=result), dict(policy='Explicit offline intrinsic-only derivative. Original cross-engine exclusions unchanged. Not native re-execution, cross-engine admission, median promotion or a complete passing matrix.', cases=intrinsic)


def validate(root, write_derivative=False):
    root = Path(root)
    b = base(root)
    b.manifest(root)
    evidence = b.load(root / 'evidence.json')
    require(evidence['version'] == 1 and evidence['original_matrix'] == 'rsm02'
            and evidence['source_commit'] == 'f97fcc2' and evidence['native_fix_commit'] == '91220ff43', 'separate rsm02 provenance')
    require(sha((root / 'input/tree.json').read_bytes()) == b.TREE_SHA, 'pinned source63 tree identity')
    tree = b.load(root / 'input/tree.json')
    require(len(tree) == 63 and set(b.files(root / 'input/source63')) == {item['path'] for item in tree}, 'complete pinned source63 inventory')
    source_hashes = {}
    for item in tree:
        data = b.safe(root / 'input/source63', item['path']).read_bytes()
        require(sha(data) == item['sha256'] and hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest() == item['git_blob'], 'pinned source63 blob drift')
        source_hashes[item['path']] = item['sha256']
    matrix = b.frozen_modules(root)
    originals = {name: (root / 'input/source63' / name).read_bytes() for name in matrix.FILES}
    result, derivative = check_matrix(root, matrix, originals, source_hashes, b)
    transform = b.load(root / 'runs/rsm02/transformation.json')
    require(evidence['private_commitments'] == transform['private_original_sha256'], 'private original commitment inventory')
    require(len(derivative['cases']) == 5 and all(case['intrinsic_accepted'] and not case['original_cross_engine_accepted'] for case in derivative['cases']), 'all five intrinsic errors, no cross-engine promotion')
    path = root / 'intrinsic-typeerror.json'
    if write_derivative:
        path.write_text(json.dumps(derivative, indent=2) + '\n')
    else:
        require(b.load(path) == derivative, 'explicit intrinsic derivative differs from unchanged strict oracle')
    return dict(status='PASS', original_matrix=result,
        intrinsic_derivative=dict(cases=5, intrinsic_accepted=5, original_cross_engine_admitted=0, typeerror_medians_still_excluded=True),
        scope='Faithful replay of FAILED cross-engine matrix. Candidate intrinsic nonce/E0308 oracles pass separately, not original cross-engine admission. No aggregate, general speedup, comprehensive/default gate or significance claim.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path, nargs='?', default=HERE)
    parser.add_argument('--seal', action='store_true')
    parser.add_argument('--derive', action='store_true', help='Write separate offline derivative, never change original run receipts')
    args = parser.parse_args()
    b = base(args.root)
    if args.derive:
        b.seal(args.root)
        validate(args.root, write_derivative=True)
        b.seal(args.root)
    elif args.seal:
        b.seal(args.root)
    print(json.dumps(validate(args.root), indent=2))


if __name__ == '__main__':
    main()
