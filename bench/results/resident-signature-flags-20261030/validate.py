#!/usr/bin/env python3
"""Portable offline audit of captured native signature-flag evidence. No native execution."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import statistics

SIG = 'toolchain/resident/vendor/rust-analyzer/crates/hir-def/src/signatures.rs'
CG = 'toolchain/resident/crates/hir-ty-codegen/mod.rs'
MAIN = 'toolchain/resident/crates/ra-jit/src/main.rs'
SOURCE_HASHES = {
    13: {SIG: '9613f5ef557b891c2979db0b59bdd44d725b94303318986259470a7ef2e3428e', CG: '0a50cf5d1abf3d812a4b942b0935810276ae0f90bfcb2b34b9b991c165e89d16', MAIN: '471f0b5c10187ecbea879acfe3e5eae52925161e32bce6feeebe49b67af0368f'},
    14: {SIG: '724793541da40fe85a8f2fa738e39ae6bae70608c0b43e4b2ece80fe772c31a3', CG: '0a50cf5d1abf3d812a4b942b0935810276ae0f90bfcb2b34b9b991c165e89d16', MAIN: 'f551b94e111afc89adfc7b374efff6b8b9038accda325f5c50ec432c08ce4453'},
    15: {SIG: '724793541da40fe85a8f2fa738e39ae6bae70608c0b43e4b2ece80fe772c31a3', CG: '0a50cf5d1abf3d812a4b942b0935810276ae0f90bfcb2b34b9b991c165e89d16', MAIN: 'a26578e35b306f61354826d68d8d65c1154f0f89ac348cc6b782736260fb3c85'},
}
RUNS = {
    '20261030-094704215-549746': (13, 'build', 'check', 0, 0, None),
    '20261030-094734257-551851': (13, 'build', 'build', 0, 0, None),
    '20261030-095015494-562655': (13, 'unit', 'test', 101, 0, 'resident_signature_comptime'),
    '20261030-095427948-579324': (14, 'build', 'check', 101, 0, None),
    '20261030-095719593-590683': (15, 'build', 'check', 0, 0, None),
    '20261030-095730450-591582': (15, 'build', 'build', 0, 0, None),
    '20261030-100010641-602393': (15, 'unit', 'test', 0, 1, 'resident_signature_flags'),
    '20261030-100239129-612338': (15, 'unit', 'test', 0, 1, 'resident_graph_reuse'),
    '20261030-100239437-612419': (15, 'unit', 'test', 0, 1, 'the_roots_of_a_listed_crate'),
    '20261030-100239657-612466': (15, 'unit', 'test', 0, 3, 'telemetry'),
    '20261030-100536218-780696': (15, 'unit', 'test', 0, 347, None),
}
NAMES = sorted('summary::tests::' + n for n in (
    'parses_libtest_results', 'shares_overlapping_time',
    'splits_passes_without_double_counting', 'test_failures_are_not_compile_errors'))
PROFILES = {f'CARGO_PROFILE_{p}_{k}': v for p in ['DEV', 'TEST'] for k, v in
            [('OPT_LEVEL', '0'), ('DEBUG', '0'), ('INCREMENTAL', 'true')]}
ENGINE_HASHES = {'rsb04': 'deafad0c76f20059db6e1a07f4ed9665013ee61600db0089aec3667c38069df8',
                 'rsa10': '0e138c556368ab4f87a5b19b9c8bc0a3b0b4462807711c63f28d80bc7c00718a'}
BUILD_IDS = {'rsb04': 'resident-v1-c44b9049b4557e5ff69a31664fe9a01e9ed0eb45643aa2fecc81ecb4df2fda18',
             'rsa10': 'resident-v1-1ac2cd3d257606bbfd6559f0c27d2cb122a472e05b5dacc99b14b9df8c8d8533'}
REFERENCE = 'blaze 0.20261027.0 (5d4f78f7577cf7170993b74f735099dd03f5c6ca)'
REFERENCE_HASH = 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263'
FULL_NAMES_HASH = '02e6c1fa64cff19fac2bdc2548001b598f011cee2f8a344b58c58f57efa41b0d'
EXCLUDED = {'manifest.json', 'collect.py', 'validate.py', 'test_validate.py', 'README.md', 'audit.json', 'offline-tests.log'}


class InvalidEvidence(ValueError):
    pass


def need(condition, message):
    if not condition:
        raise InvalidEvidence(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return sha(json.dumps(value, sort_keys=True, separators=(',', ':')).encode())


class Evidence:
    def __init__(self, root, overrides=None):
        self.root = Path(root)
        self.overrides = overrides or {}

    def read(self, relative):
        path = Path(relative)
        need(not path.is_absolute() and '..' not in path.parts, 'unsafe artifact path')
        if relative in self.overrides:
            need(self.overrides[relative] is not None, f'missing artifact {relative}')
            return self.overrides[relative]
        try:
            need(not (self.root / path).is_symlink(), f'artifact symlink {relative}')
            return (self.root / path).read_bytes()
        except OSError as error:
            raise InvalidEvidence(f'missing artifact {relative}') from error

    def text(self, relative):
        return self.read(relative).decode('utf-8')

    def obj(self, relative):
        return json.loads(self.text(relative))

    def journal(self, relative):
        return [json.loads(line) for line in self.text(relative).splitlines()]


def inventory(e):
    hashes = e.obj('manifest.json')['sha256']
    actual = {str(p.relative_to(e.root)) for p in e.root.rglob('*')
              if p.is_file() and p.name not in EXCLUDED and '__pycache__' not in p.parts}
    need(set(hashes) == actual and len(hashes) == 266, 'incomplete/extra artifact inventory')
    for path, digest in hashes.items():
        need(sha(e.read(path)) == digest, f'checksum mismatch: {path}')
    return len(hashes)


def parse_sums(text):
    pairs = [line.split(None, 1) for line in text.splitlines()]
    need(all(len(p) == 2 and re.fullmatch('[0-9a-f]{64}', p[0]) for p in pairs), 'malformed source hashes')
    need(len({p[1] for p in pairs}) == len(pairs), 'duplicate source hashes')
    return {path: digest for digest, path in pairs}


def replay_patch(e, version):
    """Apply original unified hunks in memory to genuine archived baseline bytes."""
    lines = e.text(f'source-ready/v{version}/source.patch').splitlines(keepends=True)
    result, i = {}, 0
    while i < len(lines):
        if not lines[i].startswith('--- a/'):
            i += 1
            continue
        path = lines[i][6:].strip()
        need(path in [SIG, CG, MAIN] and path not in result, 'unexpected source patch path')
        i += 1
        need(lines[i] == f'+++ b/{path}\n', 'renamed patch source')
        original = e.text(f'baseline-head/{path}').splitlines(keepends=True)
        i += 1
        output, cursor = [], 0
        while i < len(lines) and not lines[i].startswith('diff --git '):
            header = re.fullmatch(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@.*\n', lines[i])
            need(header is not None, 'malformed source hunk')
            start = int(header[1]) - 1
            need(start >= cursor, 'overlapping source hunks')
            output.extend(original[cursor:start])
            cursor = start
            old_count = new_count = 0
            i += 1
            while i < len(lines) and lines[i][:1] in [' ', '+', '-'] and not lines[i].startswith('--- a/'):
                prefix, text = lines[i][0], lines[i][1:]
                if prefix != '+':
                    need(cursor < len(original) and original[cursor] == text, 'source patch context mismatch')
                    cursor += 1
                    old_count += 1
                if prefix != '-':
                    output.append(text)
                    new_count += 1
                i += 1
            need(old_count == int(header[2] or 1) and new_count == int(header[4] or 1), 'source hunk count mismatch')
        output.extend(original[cursor:])
        result[path] = ''.join(output).encode()
    need(set(result) == ({SIG, CG} if version == 13 else {SIG, CG, MAIN}), 'missing patch file')
    for path in [SIG, CG, MAIN]:
        actual = result.get(path, e.read(f'baseline-head/{path}'))
        need(sha(actual) == SOURCE_HASHES[version][path], f'patch replay source v{version} {path}')


def sources(e):
    for version in [13, 14, 15]:
        sums = parse_sums(e.text(f'native/candidate-build-v{version}/source-before.sha256'))
        need(len(sums) == 11, 'incomplete native source identity')
        for path, digest in sums.items():
            need(sha(e.read(f'native/candidate-build-v{version}/frozen-source/{path}')) == digest, 'frozen build source mismatch')
        for path, digest in SOURCE_HASHES[version].items():
            need(sums[path.removeprefix('toolchain/resident/')] == digest, 'wrong ready/build source identity')
        ready = parse_sums(e.text(f'source-ready/v{version}/SHA256SUMS'))
        need(len(ready) == (3 if version == 13 else 4), 'missing ready source receipt')
        for path, digest in ready.items():
            need(sha(e.read(f'source-ready/v{version}/{path}')) == digest, 'source ready snapshot mismatch')
            if path in SOURCE_HASHES[version]:
                need(digest == SOURCE_HASHES[version][path], 'wrong ready source hash')
        replay_patch(e, version)
        if version != 14:
            for folder in ['build'] + (['unit'] if version == 15 else []):
                after = e.text(f'native/candidate-{folder}-v{version}/source-after.log')
                need(after.splitlines() == [p + ': OK' for p in sums], 'source changed across native gate')
    need(e.text('native/candidate-unit-v15/full-native-source-after.log') ==
         e.text('native/candidate-unit-v15/source-after.log'), 'source changed during full347 gate')
    # Pinning full bytes above also pins the independent oracle and complete fixture.
    main = e.text(f'source-ready/v15/{MAIN}')
    for fragment in ['let original = AttrFlags::has_word(&db, id.into(), "rustc_comptime")',
                     'assert_eq!(comparisons, 12 * 8)', 'assert_eq!(revisions.len(), 8)',
                     'Vec<((String, String), String)>', 'format!("{signature:?}")']:
        need(fragment in main, 'missing fixture oracle/coverage/snapshot')


def libtest(text, native=False):
    pattern = r'^test (\S+) \.\.\. (?:JR_RUNTIME_EXECUTED_\d+\n)?(ok|FAILED|ignored)(?: <[0-9.]+s>)?$'
    tests = re.findall(pattern, text, re.M)
    footers = re.findall(r'^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;', text, re.M)
    need(len(footers) == 1, 'missing/duplicate raw libtest footer')
    status, passed, failed, ignored, measured, filtered = footers[0]
    counts = tuple(map(int, (passed, failed, ignored, measured, filtered)))
    need(status == ('FAILED' if counts[1] else 'ok'), 'raw libtest verdict disagrees with counts')
    need(len(tests) == len(set(name for name, _ in tests)), 'duplicate raw native test names')
    need(sum(state == 'ok' for _, state in tests) == counts[0] and
         sum(state == 'FAILED' for _, state in tests) == counts[1] and
         sum(state == 'ignored' for _, state in tests) == counts[2], 'raw test names disagree with counts')
    need(('running %d tests' % len(tests)) in text or ('running %d test' % len(tests)) in text, 'missing positive test admission')
    return sorted(tests), counts


def native_runs(e):
    exports = {}
    for version, kind in [(13, 'build'), (13, 'unit'), (14, 'build'), (15, 'build'), (15, 'unit')]:
        for row in e.journal(f'exports/native-v{version}-{kind}/2026-10.jsonl'):
            need(row['id'] not in exports, 'duplicate export run')
            need(row['machine'] == f'native-v{version}-{kind}', 'export machine group mismatch')
            exports[row['id']] = row
    need(set(exports) == set(RUNS), '11 exact exports required')
    full_names = []
    for run_id, (version, kind, cmd, exit_code, count, filt) in RUNS.items():
        folder = f'native/candidate-{kind}-v{version}/justrust-home/runs/{run_id}'
        meta, summary = e.obj(folder + '/meta.json'), e.obj(folder + '/summary.json')
        journal = e.journal(folder + '/output.jsonl')
        text = '\n'.join(line['l'] for line in journal)
        export = exports[run_id]
        need(journal and all(line['s'] in ['e', 'o'] and math.isfinite(line['t']) for line in journal), 'missing/malformed native raw journal')
        expected_args = [cmd, '--release', '--locked', '-p', 'hir-def' if version == 13 and kind == 'unit' else 'ra-jit']
        if not (version == 13 and kind == 'unit'):
            expected_args += ['--bin', 'jr-engine']
        if cmd == 'check':
            expected_args += ['--tests']
        expected_args += ['-j', '6']
        if filt:
            expected_args += ['--', filt, '--nocapture']
        need(meta['id'] == summary['id'] == export['id'] == run_id, 'native/export ID mismatch')
        need(meta['args'] == summary['args'] == expected_args, 'native command/filter mismatch')
        need(summary['subcommand'] == export['subcommand'] == cmd and export['args'] == [cmd], 'slim command mismatch')
        need(summary['exit'] == export['exit'] == exit_code, 'native exit failure erased')
        need(meta['place'] == 'local' and meta['route'] == export['route'] == 'JUSTRUST_REMOTE=off', 'wrong native execution route')
        need(summary['git']['head'] == '8905ce5931b3', 'wrong native source revision')
        need(math.isclose(summary['wall'], export['wall'], abs_tol=0.001), 'native/export timing mismatch')
        need(math.isclose(meta['start'], summary['start'], abs_tol=0.1), 'native meta/summary timestamp mismatch')
        tests = summary['tests']
        need(tests['passed'] == export['tests']['passed'] == count and tests['failed'] == export['tests']['failed'] == 0,
             'native/export test count mismatch')
        if count:
            names, counts = libtest(text, native=True)
            need(counts == (count, 0, 0, 0, 347 - count), 'wrong positive native coverage')
            need(tests['filtered_out'] == export['tests']['filtered_out'] == 347 - count, 'native filtered count mismatch')
            if count == 347:
                full_names = [name for name, _ in names]
                need(sha(('\n'.join(full_names) + '\n').encode()) == FULL_NAMES_HASH, 'full347 raw names changed')
            elif filt != 'telemetry':
                need(filt in names[0][0], 'wrong test admitted by native filter')
            else:
                need(all('telemetry' in name for name, _ in names), 'wrong telemetry tests')
        elif exit_code:
            need('requires dev-dependencies and is not a member of the workspace' in text if version == 13 else
                 'error[E0502]' in text and 'baseline.push((id, signature))' in text, 'missing original native failure')
    for version in [13, 15]:
        for name in ['check-exit', 'compile-exit', 'exit']:
            need(e.text(f'native/candidate-build-v{version}/{name}').strip() == '0', 'build exit gate mismatch')
        expected_orchestration = '101' if version == 13 else '0'
        need(e.text(f'native/candidate-build-v{version}/orchestration-exit').strip() == expected_orchestration,
             'build/unit orchestration failure erased')
    for name in ['check-exit', 'exit', 'orchestration-exit']:
        need(e.text(f'native/candidate-build-v14/{name}').strip() == '101', 'v14 compile failure erased')
    for name in ['resident_signature_comptime.exit', 'exit']:
        need(e.text(f'native/candidate-unit-v13/{name}').strip() == '101', 'v13 workspace failure erased')
    for name in ['resident_signature_flags.exit', 'resident_graph_reuse.exit', 'the_roots_of_a_listed_crate.exit', 'telemetry.exit', 'full-native.exit', 'exit']:
        need(e.text(f'native/candidate-unit-v15/{name}').strip() == '0', 'v15 unit gate mismatch')
    need(e.text('provenance/suite-v14.exit').strip() == '2', 'blocked runtime gate erased')
    admission = e.obj('native/candidate-unit-v15/positive-test-admission.json')
    expected = [{'filter': filt, 'passed': count, 'run_id': rid}
                for rid, (v, k, _, _, count, filt) in RUNS.items() if v == 15 and k == 'unit' and filt]
    need(admission == {'verdict': 'passed', 'native_test_rows': expected}, 'unit admission receipt mismatch')
    return {'runs': len(RUNS), 'slim_exports': len(exports), 'all347_raw_names': full_names,
            'full347_raw_names_sha256': FULL_NAMES_HASH, 'fixture_functions': 12,
            'fixture_revisions': 8, 'fixture_comparisons': 96,
            'failed_gates_preserved': ['v13 excluded workspace devdeps', 'v14 E0502', 'v14 runtime blocked']}


def edit_bytes(original, phase, replicate):
    anchor = b'fn shares_overlapping_time() {'
    need(original.count(anchor) == 1, 'workload edit anchor mismatch')
    n = replicate + 1
    body = (f'let observed = std::hint::black_box({n}u64); assert_eq!(observed, {n}u64); '
            f'println!("JR_RUNTIME_EXECUTED_{n}");').encode()
    if phase == 'type-error':
        body = b'let _: u64 = "JR_RUNTIME_TYPE_ERROR";'
    elif phase == 'assert-failure':
        body = b'assert_eq!(std::hint::black_box(1u64), 2u64, "JR_RUNTIME_FAILURE");'
    return original.replace(anchor, anchor + b'\n' + body, 1)


def session(e, name):
    prefix = f'sessions/{name}/'
    metadata, suite, rows = [e.obj(prefix + f'{n}.json') for n in ['metadata', 'suite', 'records']]
    need(suite['schema'] == 'native_resident/v1' and suite['metadata'] == metadata, 'suite metadata mismatch')
    need(len(rows) == 22 and metadata['status'] == 'complete' and metadata['unchanged'] is True and
         metadata['source_restored'] is True and metadata['toolchain_links_valid'] is True, 'incomplete runtime session')
    need(metadata['source_before'] == metadata['source_after'] and len(metadata['source_before']) == 63 and
         canonical(metadata['source_before']) == '79c53ff6e87a22c84c9bc7bac6917c69b3f8d0fbc29b107f734cc00cc39a64df', 'wrong pinned source63 manifest')
    need(metadata['toolchain_before'] == metadata['toolchain_after'] and
         canonical(metadata['toolchain_before']) == '8bdbccdd1ca3fd7f2935cfb5667047eb4c46824928823e96311d8eb5ec0a6371', 'wrong pinned full toolchain identity')
    need(metadata['tools_before'] == metadata['tools_after'] and set(metadata['tools_before']) == {'reference', 'engine'}, 'changed/unexpected executable identities')
    need(metadata['tools_before']['reference']['sha256'] == REFERENCE_HASH and
         metadata['tools_before']['engine']['sha256'] == ENGINE_HASHES[name], 'wrong exact executable hash')
    need(metadata['pin'] == 'f97fcc2' and metadata['reference_version_required'] == REFERENCE and
         metadata['profiles'] == PROFILES and metadata['store_scope'] == 'unseeded private cold stores, no automatic seeding', 'runtime input/policy mismatch')
    original = e.read(prefix + 'source.original')
    need(sha(original) == metadata['source_before']['src/summary.rs'], 'wrong original workload source')
    artifacts = suite['artifacts_sha256']
    need(len(artifacts) == 48, 'incomplete suite raw inventory')
    for path, digest in artifacts.items():
        need(sha(e.read(prefix + path)) == digest, 'raw suite checksum mismatch')
    samples = {}
    wanted = [('version', 0), ('prepare', 0), ('type-error', 0), ('type-recovery', 0),
              ('assert-failure', 0), ('assert-recovery', 0)] + [('edit', n) for n in range(5)]
    need(all(row['stack'] in ['reference', 'engine'] for row in rows), 'unexpected stack')
    for stack in ['reference', 'engine']:
        env = metadata['environments'][stack]
        need(all(env.get(k) == v for k, v in PROFILES.items()), 'profile environment mismatch')
        need(env['BLAZE_BACKEND'] == 'cranelift' and env['BLAZE_OPT'] == '0', 'wrong runtime backend')
        need(all(env.get(k) == v for k, v in [('BLAZE_REMOTE', 'off'), ('RA_JIT_REMOTE', 'off'),
             ('BLAZE_PROC_MACRO_SERVER', '0'), ('RA_JIT_PROC_MACRO_SERVER', '0')]), 'remote/proc-macro policy mismatch')
        private = f'/home/jeremy/{name}/{stack}/'
        for key in ['CARGO_HOME', 'CARGO_TARGET_DIR', 'RUSTUP_HOME', 'BLAZE_CACHE', 'RA_JIT_CACHE', 'BLAZE_HOME', 'RA_JIT_HOME']:
            need(env[key].startswith(private), 'stores not private/unseeded')
        selected = [row for row in rows if row['stack'] == stack]
        need([(r['phase'], r['replicate']) for r in selected] == wanted, 'missing/duplicate/out-of-order runtime phases')
        values = []
        for row in selected:
            phase, rep = row['phase'], row['replicate']
            raw = '\n'.join(e.text(prefix + f'{stack}-{phase}-{rep}.{s}') for s in ['stdout', 'stderr'])
            need(row['error'] is None and row['timed_out'] is False, 'runtime process error/timeout')
            elapsed = row['elapsed_seconds']
            need(type(elapsed) in [float, int] and math.isfinite(elapsed) and elapsed > 0, 'invalid raw timing')
            command = [metadata['tools_before'][stack]['path']]
            command += ['--version'] if phase == 'version' else ((['cargo'] if stack == 'reference' else []) +
                       ['test', '--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture'])
            need(row['command'] == command, 'runtime command mismatch')
            need(row['environment']['CARGO_TARGET_DIR'] == env['CARGO_TARGET_DIR'] and
                 all(row['environment'].get(k) == v for k, v in PROFILES.items()), 'recorded command environment mismatch')
            # The historical per-command allowlist omits remote/proc-macro
            # variables. Those are checked in session metadata above, not
            # invented as a per-command attestation here. Backend is recorded.
            need(row['environment'].get('BLAZE_BACKEND') == 'cranelift' and
                 row['environment'].get('BLAZE_OPT') == '0',
                 'effective per-command backend mismatch')
            contents = edit_bytes(original, phase, rep) if phase in ['type-error', 'assert-failure', 'edit'] else original
            need(row['source_sha256'] == sha(contents), 'stale workload edit hash')
            need(row['noise']['contamination'] == 'unknown', 'noise guarantee invented')
            if phase == 'version':
                need(row['status'] == 0 and (REFERENCE if stack == 'reference' else BUILD_IDS[name]) in raw, 'wrong raw executable version')
                need(metadata['versions'][stack]['valid'] is True and metadata['versions'][stack]['text'].strip() == raw.strip(), 'version metadata mismatch')
            elif phase == 'type-error':
                need(row['status'] == 101 and row['gate_passed'] is True and 'E0308' in raw and
                     'JR_RUNTIME_TYPE_ERROR' in raw and 'test result:' not in raw, 'missing actual type error gate')
            else:
                tests, counts = libtest(raw)
                failing = phase == 'assert-failure'
                expected = [(n, 'FAILED' if failing and n.endswith('shares_overlapping_time') else 'ok') for n in NAMES]
                need(tests == expected and counts == (3 if failing else 4, 1 if failing else 0, 0, 0, 144), 'raw libtest correctness parity failed')
                need(row['status'] == (101 if failing else 0), 'wrong runtime process status')
                need(row['parsed']['tests'] == [list(t) for t in expected] and
                     row['parsed']['passed'] == counts[0] and row['parsed']['failed'] == counts[1], 'recorded parsed tests mismatch')
                if failing:
                    need(row['gate_passed'] is True and 'JR_RUNTIME_FAILURE' in raw, 'missing assertion failure gate')
                if phase == 'edit':
                    need(row['accepted'] is True and re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*', raw) == [f'JR_RUNTIME_EXECUTED_{rep + 1}'], 'stale/missing/duplicate runtime marker')
                    values.append(elapsed)
        samples[stack] = {'samples_seconds': values, 'median_seconds': statistics.median(values)}
    need(suite['measurements'] == samples, 'suite timing summary not derived from raw records')
    summary = e.obj(prefix + 'summary.json')
    need(summary['medians_seconds'] == {k: v['median_seconds'] for k, v in samples.items()} and
         summary['excluded'] == [] and summary['source_built_parity'] is True, 'excluded sample or false parity summary')
    need(suite['noise'] == 'unknown' and suite['scope'] == 'scoped diagnostic, not comprehensive or default promotion', 'scope/noise overclaim')
    build = 'native/baseline-v4' if name == 'rsb04' else 'native/candidate-build-v15'
    need(e.text(build + '/engine-identity.sha256').split()[0] == ENGINE_HASHES[name] and
         e.text(build + '/engine-version.txt').strip() == metadata['versions']['engine']['text'].strip(), 'build/runtime executable binding mismatch')
    return metadata, samples, suite['host']


def audit(e, verify_inventory=True):
    try:
        files = inventory(e) if verify_inventory else None
        provenance = e.obj('provenance.json')
        need(provenance['decision'] == 'NOT_A_PERFORMANCE_WIN' and provenance['promotion'] is False,
             'performance promotion/decision overclaim')
        sources(e)
        native = native_runs(e)
        ma, sa, ha = session(e, 'rsb04')
        mb, sb, hb = session(e, 'rsa10')
        need(ha == hb and ha['execution'] == 'local native direct', 'host/cpu/execution mismatch')
        for key in ['source_before', 'toolchain_before', 'profiles', 'proc_macro_policy', 'store_scope']:
            need(ma[key] == mb[key], 'unmatched runtime inputs: ' + key)
        need(ma['tools_before']['reference'] == mb['tools_before']['reference'], 'reference mismatch')
        old, new = sa['engine']['median_seconds'], sb['engine']['median_seconds']
        ra, rb = old / sa['reference']['median_seconds'], new / sb['reference']['median_seconds']
        raw, normalized = (new / old - 1) * 100, (rb / ra - 1) * 100
        need(-10 < raw < 0 and normalized > 0, 'captured non-win conclusion contradicted')
        compare = ('Native resident scoped diagnostic: descriptive/uncontrolled, noise unknown.\n'
                   f'Engine median: {old:.6f}s -> {new:.6f}s, delta {new-old:+.6f}s ({raw:+.2f}%).\n'
                   f'Reference-normalized engine/reference ratio: {ra:.6f} -> {rb:.6f}, relative delta {normalized:+.2f}%.\n'
                   'Absolute median delta does not exceed requested 10% floor. No significance or speedup verdict.\n'
                   'Not a comprehensive workload or default-promotion gate.\n')
        need(e.text('provenance/suite-v15-compare.log') == compare, 'archived comparison not reproducible')
        for name in ['suite-v15.exit', 'suite-v15-baseline.exit', 'suite-v15-candidate.exit']:
            need(e.text('provenance/' + name).strip() == '0', 'runtime execution exit mismatch')
        return {'audit': 'PASS', 'decision': 'NOT_A_PERFORMANCE_WIN', 'promotion': False,
                'artifact_count': files, 'native': native,
                'runtime': {'baseline': 'RSB04', 'candidate': 'RSA10', 'engine_before_seconds': old,
                            'engine_after_seconds': new, 'raw_delta_percent': raw,
                            'reference_normalized_delta_percent': normalized, 'floor_percent': 10,
                            'noise': 'unknown', 'scope': 'descriptive diagnostic, not comprehensive or default promotion'},
                'source_state': 'Historical frozen receipts only. No current working-source restoration or source-commit claim.'}
    except (KeyError, TypeError, IndexError, UnicodeError, json.JSONDecodeError) as error:
        raise InvalidEvidence(f'malformed evidence: {error}') from error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', nargs='?', type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    try:
        print(json.dumps(audit(Evidence(args.root)), indent=2))
    except InvalidEvidence as error:
        raise SystemExit(f'INVALID EVIDENCE: {error}')


if __name__ == '__main__':
    main()
