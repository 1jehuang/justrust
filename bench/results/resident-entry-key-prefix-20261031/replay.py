#!/usr/bin/env python3
"""Offline exact native-journal replay. Never invokes a compiler, engine, git, or network."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import sys
sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent / 'oracle'))
import resident_state_matrix as matrix

PHASES = ['cold-prepare', 'repeated-noop-1', 'manifest-feature-add', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery']
MODES = ['off', 'on', 'on-repeat', 'off-repeat']
MARKER = b'ra-jit entry-key-prefix v1 prepared'
CACHED = Path('toolchain/resident/crates/hir-ty-codegen/cached.rs')
BASE = '11543e4a6a48afec1c23819a999cf6835ed2c1d7990919aea70cabe134c2906f'

def digest(data):
    return hashlib.sha256(data).hexdigest()

def require(ok, reason):
    if not ok:
        raise ValueError(reason)

def read_json(path):
    return json.loads(path.read_text())

def seal(root):
    files = sorted(p for p in root.rglob('*') if p.is_file() and p.name != 'SHA256SUMS.bundle' and '__pycache__' not in p.parts)
    (root / 'SHA256SUMS.bundle').write_text(''.join(f'{digest(p.read_bytes())}  {p.relative_to(root).as_posix()}\n' for p in files))

def verify_seal(root):
    listed = set()
    for line in (root / 'SHA256SUMS.bundle').read_text().splitlines():
        sha, name = line.split('  ', 1)
        p = Path(name)
        require(not p.is_absolute() and '..' not in p.parts and name not in listed, 'unsafe/duplicate seal path')
        listed.add(name)
        require(digest((root / p).read_bytes()) == sha, 'bundle hash: ' + name)
    actual = {p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file() and p.name != 'SHA256SUMS.bundle' and '__pycache__' not in p.parts}
    require(actual == listed, 'bundle file inventory mismatch')

def validate(root):
    verify_seal(root)
    frozen = root / 'frozen'
    for version in ('v44', 'v45'):
        for line in (frozen / version / 'SHA256SUMS').read_text().splitlines():
            sha, name = line.split('  ', 1)
            require(not Path(name).is_absolute() and '..' not in Path(name).parts, 'safe frozen checksum path')
            require(digest((frozen / version / name).read_bytes()) == sha, 'frozen source journal checksum')
    require(digest((frozen / 'v44' / 'original' / CACHED).read_bytes()) == BASE, 'accepted original identity')
    v44 = (frozen / 'v44' / 'candidate' / CACHED).read_bytes()
    v45 = (frozen / 'v45' / 'candidate' / CACHED).read_bytes()
    require((frozen / 'v45' / 'original' / CACHED).read_bytes() == v44, 'repair original identity')
    require((frozen / 'v45' / 'rejected-source-before-restoration.rs').read_bytes() == v45, 'rejected preservation')
    marker_block = b'        if key_prefix.is_some() {\n            let _ = std::io::Write::write_all(\n                &mut std::io::stderr().lock(),\n                b"ra-jit entry-key-prefix v1 prepared\\n",\n            );\n        }\n'
    require(v45.count(marker_block) == 1 and v45.replace(marker_block, b'', 1) == v44, 'marker-only repair')
    require(MARKER not in v44, 'v44 must retain absent activation control')
    def oracle_bytes(source):
        return source.split(b'    pub(super) fn key(name: &str, debug: Told, program:', 1)[1].split(b'    pub(super) fn encode', 1)[0]
    require(oracle_bytes((frozen / 'v44' / 'original' / CACHED).read_bytes()) == oracle_bytes(v44) == oracle_bytes(v45), 'untouched direct oracle')
    all_rows = []
    activation = {}
    timings = {}
    identities = {}
    source_identity = None
    toolchain_identity = None
    for version in ('v44', 'v45'):
        native = root / 'native' / version
        for kind, names in [('build', ['check.exit', 'compile.exit', 'orchestration-exit']), ('unit', ['codegen-native.exit', 'full-native.exit', 'exit'])]:
            for name in names:
                require((native / kind / name).read_text().strip() == '0', f'{version} native {name}')
        require((native / 'pairs.exit').read_text().strip() == '0', 'pairs process exit')
        for name, count in [('codegen-native.log', 57), ('full-native.log', 360)]:
            require(f'{count} tests passed' in (native / 'unit' / name).read_text(), 'native test count')
        native_counts = []
        for run in sorted((native / 'unit' / 'runs').iterdir()):
            meta = read_json(run / 'meta.json')
            text = '\n'.join(json.loads(line)['l'] for line in (run / 'output.jsonl').read_text().splitlines())
            parsed = matrix.shared.runtime.test_results(text)
            require(parsed is not None and parsed['failed'] == 0 and parsed['ignored'] == 0 and parsed['filtered'] == 0, 'full native raw test contract')
            native_counts.append(parsed['passed'])
            require(meta['args'][:3] == ['test', '--release', '--locked'], 'native command identity')
            if parsed['passed'] == 57:
                tests = [name for name, status in parsed['tests'] if 'entry_key_prefix_' in name and status == 'ok']
                require(len(tests) == 4, 'four native prefix fixtures actually passed')
        require(sorted(native_counts) == [57, 360], 'full native suite inventory')
        build_manifest = (native / 'build' / 'all-source-before.sha256').read_text()
        expected_sha = digest((frozen / version / 'candidate' / CACHED).read_bytes())
        require(f'{expected_sha}  crates/hir-ty-codegen/cached.rs\n' in build_manifest, 'native source identity')
        require('FAILED' not in (native / 'unit' / 'all-source-after.log').read_text(), 'source after check')
        engine = (native / 'build' / 'engine-identity.sha256').read_text().split()[0]
        identities[version] = dict(engine_sha256=engine, version=(native / 'build' / 'engine-version.txt').read_text().strip())
        activation[version] = {}
        timings[version] = {}
        for mode in MODES:
            directory = root / 'raw' / version / mode
            state = read_json(directory / 'state.json')
            on = mode.startswith('on')
            require(state['startup_controls'] == dict(RA_JIT_DEMAND_STD='1', RA_JIT_ENTRY_KEY_PREFIX='1' if on else '0', RA_JIT_STATS='1'), 'startup control')
            require(state['status'] == 'passed' and state['unchanged'], 'state completion')
            require(state['source_before'] == state['source_after'], 'source restoration')
            require(state['tool_before'] == state['tool_after'] == engine, 'binary identity')
            require(state['toolchain_before'] == state['toolchain_after'], 'toolchain identity')
            if source_identity is None:
                source_identity = state['source_before']
                toolchain_identity = state['toolchain_before']
            require(state['source_before'] == source_identity, 'identical pinned source across pairs')
            require(state['toolchain_before'] == toolchain_identity, 'identical toolchain across pairs')
            tc = state['toolchain_before']['files']
            require(tc['lib/rustlib/multirust-channel-manifest.toml']['sha256'] == matrix.shared.TOOLCHAIN_MANIFEST, 'pinned toolchain manifest')
            require([r['phase'] for r in state['rows']] == PHASES, 'phase inventory/order')
            originals = {name: (directory / f'original-{i}').read_bytes() for i, name in enumerate(matrix.FILES)}
            for name, contents in originals.items():
                require(digest(contents) == state['source_before'][name], 'original backup hash')
            contract = None
            counts = []
            times = []
            for index, row in enumerate(state['rows']):
                phase = row['phase']
                require(re.fullmatch('[0-9a-f]{32}', row['nonce']) is not None, 'nonce shape')
                require(row['accepted'] and not row['failures'], 'recorded oracle admission')
                require(math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0, 'finite positive timing')
                require(row['noise']['contamination'] == 'unknown', 'retain unknown noise')
                require(row['command'][1:] == ['test', *matrix.arguments(phase)], 'command contract')
                wanted = dict(state['source_before'])
                for name, contents in matrix.inputs(originals, phase, 0, row['nonce']).items():
                    wanted[name] = digest(contents)
                require(row['inputs'] == wanted, 'exact input recipe')
                parts = []
                for suffix in ('stdout', 'stderr'):
                    data = (directory / f'{index:02}-{phase}.{suffix}').read_bytes()
                    raw = row['raw'][suffix]
                    require(len(data) == raw['bytes'] and digest(data) == raw['sha256'], 'raw receipt')
                    require(len(data) <= matrix.MAX_RAW_BYTES, 'bounded raw parser')
                    parts.append(data.decode(errors='replace'))
                text = '\n'.join(parts)
                parsed = matrix.shared.runtime.test_results(text)
                # JSON serializes native tuple pairs as lists.
                require(json.loads(json.dumps(parsed)) == row['parsed'], 'parsed exact raw replay')
                actual = dict(row, parsed=parsed)
                if phase == 'cold-prepare':
                    contract = matrix.contract(parsed, phase)
                    require(contract is not None, 'exact cold contract')
                failures = matrix.oracle(actual, text, contract, phase, row['nonce'], 0)
                require(not failures, 'exact original semantic oracle: ' + repr(failures))
                logs = sorted(directory.glob(f'{index:02}-{phase}-worker-*.log'))
                require(len(logs) == 1, 'worker log inventory')
                count = sum(log.read_bytes().splitlines().count(MARKER) for log in logs)
                require(count == (1 if version == 'v45' and on else 0), 'activation positive/negative control')
                counts.append(count)
                times.append(row['elapsed_seconds'])
                all_rows.append(dict(version=version, mode=mode, **row))
            activation[version][mode] = counts
            timings[version][mode] = times
    require(sorted(all_rows, key=lambda r:(r['version'],r['mode'],PHASES.index(r['phase']))) == read_json(root / 'rows56.json'), 'raw56 coherent index')
    percentages = {}
    for version in ('v44', 'v45'):
        percentages[version] = {phase: [(timings[version]['on' + rep][i] / timings[version]['off' + rep][i] - 1) * 100 for rep in ('', '-repeat')] for i, phase in enumerate(PHASES)}
    journal = read_json(root / 'provenance' / 'prefix-v45-validation.json')
    require(journal['decision'] == 'REJECTED' and journal['promotion'] is False, 'root rejection retained')
    require([r['phase'] for r in journal['comparisons']] == PHASES, 'root comparison phases')
    for row in journal['comparisons']:
        require([row['first_on_relative_to_off_percent'], row['repeat_on_relative_to_off_percent']] == percentages['v45'][row['phase']], 'root comparison exact recomputation')
    for mode in MODES:
        require(journal['observations'][mode]['activation_marker_counts'] == activation['v45'][mode], 'root activation exact recomputation')
    restoration = read_json(root / 'restoration.json')
    require(restoration['restored_sha256'] == BASE and restoration['cmp_backup_pass'] and restoration['git_diff_head_pass'], 'source restoration receipt')
    for name in ('completed-v43-v44-stop', 'completed-v45-stop'):
        receipt = read_json(root / 'provenance' / name / 'receipt.json')
        require(receipt['stores_deleted'] is False, 'cleanup never deletes stores')
        require(all(r['status'] == 0 and not r['timed_out'] and not r['error'] for r in receipt['rows']), 'operational cleanup status')
    return dict(status='PASS', decision='REJECTED: no repeatable useful win; no timing attribution', raw_rows=len(all_rows), v44_activation='FAILED, unadmitted', v45_activation='positive ON and negative OFF controls PASS', activation=activation, percentages=percentages, identities=identities, noise='unknown', source_restored=True)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument('--seal', action='store_true', help='Recompute transport checksums, not semantic admission')
    args = parser.parse_args()
    if args.seal:
        seal(args.root)
    print(json.dumps(validate(args.root), indent=2))
