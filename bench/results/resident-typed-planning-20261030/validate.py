#!/usr/bin/env python3
"""Portable offline audit of a rejected native candidate. Never executes workloads.

Original absolute paths in raw receipts are provenance, never opened. All reads are
bounded to this bundle. Hashes check preservation, semantic checks reject rehashed
contradictions. This is not a proof of Rust correctness or causal performance.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import statistics

LIB = 'toolchain/resident/crates/blaze-metadata/src/lib.rs'
CALLER = 'toolchain/resident/crates/ra-jit/src/cargo/metadata.rs'
TEST = 'toolchain/resident/crates/blaze-metadata/tests/resident_typed_planning.rs'
SOURCE_HASHES = {
    'candidate/' + LIB: 'e6a3b87dab7a5b17f2e300e5e173608cd6985c45158a9203f8ba1574dc659e9f',
    'candidate/' + CALLER: '8974812146961ff0ca9a70b1306eb8cf96d5b27c37871f1c239a8655843082dd',
    'candidate/' + TEST: '6b57f050d8cbc2bcfac52da070cb32fa90a103413e0bdb4bd693868aced27a6b',
    'baseline/' + LIB: 'af59a6616646759334113567cd8818d2066a111999d2878d16ea879811e48dd5',
    'baseline/' + CALLER: '1c913b348bb893d478d9bba1ab21c80180fafa0aef104d116d31bf7e45d27b48',
}
OLD = 'deafad0c76f20059db6e1a07f4ed9665013ee61600db0089aec3667c38069df8'
NEW = 'e483c2e6c1675786e1c293266bcf1ccc7d1f9219f1b8070e917d7728c10cb9f5'
REFERENCE = 'fa622aeca09858c050264f701fbbebd1a14aa07d2b02c1eb4de511b684ba0263'
NAMES = ['summary::tests::' + x for x in ('parses_libtest_results', 'shares_overlapping_time', 'splits_passes_without_double_counting', 'test_failures_are_not_compile_errors')]
COMMAND = ['test', '--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture']
CASES = {'full': [], 'no-deps': ['--no-deps'], 'all-features': ['--all-features'],
         'no-default-features': ['--no-default-features'],
         'missing-feature': ['--features', 'JR_TYPED_UNKNOWN_FEATURE'],
         'bad-format': ['--format-version', '999']}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


class Bundle:
    def __init__(self, root):
        self.root = Path(root).resolve()

    def path(self, name):
        path = self.root / name
        require(not Path(name).is_absolute() and '..' not in Path(name).parts, 'unsafe path')
        require(not path.is_symlink() and path.resolve().is_relative_to(self.root), 'symlink escape')
        return path

    def raw(self, name):
        return self.path(name).read_bytes()

    def text(self, name):
        return self.raw(name).decode('utf-8')

    def json(self, name):
        return json.loads(self.text(name))


def parse_tests(output):
    names = re.findall(r'^test ([^\s]+) \.\.\. (?:(?:[^\n]*\n)*?)(ok|FAILED|ignored)(?: <[0-9]+(?:\.[0-9]+)?s>)?[ \t]*$', output, re.M)
    totals = re.findall(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out', output)
    require(len(totals) == 1 and len(names) == 4, 'actual test output missing')
    state, passed, failed, ignored, measured, filtered = totals[0]
    require(int(measured) == 0 and state == ('FAILED' if int(failed) else 'ok'), 'test state')
    return dict(tests=[list(x) for x in sorted(names)], passed=int(passed), failed=int(failed), ignored=int(ignored), filtered=int(filtered))


def runtime_tests(output, failing=False):
    parsed = parse_tests(output)
    wanted = [[name, 'FAILED' if failing and name == NAMES[1] else 'ok'] for name in NAMES]
    require(parsed == dict(tests=wanted, passed=3 if failing else 4, failed=int(failing), ignored=0, filtered=144), 'runtime result mismatch')
    return parsed


def successful(row):
    require(row['status'] == 0 and row['error'] is None and row['timed_out'] is False, 'process failure')
    require(isinstance(row['elapsed_seconds'], (float, int)) and math.isfinite(row['elapsed_seconds']) and row['elapsed_seconds'] > 0, 'invalid timing')


def validate_patch(b):
    lines = b.text('candidate.patch').splitlines(keepends=True)
    outputs = {}
    i = 0
    while i < len(lines):
        if not lines[i].startswith('--- '):
            i += 1
            continue
        old = lines[i][4:].strip()
        new = lines[i + 1][4:].strip()
        require(lines[i + 1].startswith('+++ b/'), 'patch new path')
        name = new[2:]
        require(name in (LIB, CALLER, TEST) and name not in outputs, 'patch file scope')
        require(old == ('/dev/null' if name == TEST else 'a/' + name), 'patch old path')
        source = [] if old == '/dev/null' else b.text('baseline/' + name).splitlines(keepends=True)
        out, cursor = [], 0
        i += 2
        while i < len(lines) and not lines[i].startswith('diff --git '):
            match = re.match(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@', lines[i])
            require(match is not None, 'patch hunk header')
            start, old_count, _, new_count = [int(x) if x is not None else 1 for x in match.groups()]
            start = max(0, start - 1)
            require(start >= cursor, 'patch hunk overlap')
            out.extend(source[cursor:start])
            cursor = start
            used_old = used_new = 0
            i += 1
            while i < len(lines) and lines[i][:1] in (' ', '+', '-'):
                tag, text = lines[i][0], lines[i][1:]
                if tag in (' ', '-'):
                    require(cursor < len(source) and source[cursor] == text, 'patch baseline context')
                    cursor += 1
                    used_old += 1
                if tag in (' ', '+'):
                    out.append(text)
                    used_new += 1
                i += 1
            require((used_old, used_new) == (old_count, new_count), 'patch hunk counts')
        out.extend(source[cursor:])
        outputs[name] = ''.join(out).encode()
    require(set(outputs) == {LIB, CALLER, TEST}, 'patch missing source')
    for name, data in outputs.items():
        require(data == b.raw('candidate/' + name), 'patch does not reproduce exact source')


def validate_gates(b):
    for name in ('build-v8/check-exit', 'build-v8/compile-exit', 'build-v8/exit', 'unit-v8/exit',
                 'unit-v8/typed_programs.exit', 'unit-v8/metadata_resolver.exit',
                 'unit-v8/resident_graph_reuse.exit', 'unit-v8/the_roots_of_a_listed_crate.exit', 'unit-v8/telemetry.exit'):
        require(b.text(name).strip() == '0', 'native gate nonzero: ' + name)
    require('5 tests passed' in b.text('unit-v8/typed_programs.log'), 'five typed tests absent')
    require('49 tests passed' in b.text('unit-v8/metadata_resolver.log'), '49 existing tests absent')
    for name, count in (('resident_graph_reuse', 1), ('the_roots_of_a_listed_crate', 1), ('telemetry', 3)):
        require(f'{count} test' in b.text('unit-v8/' + name + '.log') and 'passed' in b.text('unit-v8/' + name + '.log'), name + ' receipt absent')
    require(b.text('build-v7/check-exit').strip() == '0', 'v7 compile check lost')
    require(b.text('build-v7/exit').strip() != '0' and b.text('unit-v7/typed_programs.exit').strip() != '0', 'v7 failed fixture relabeled')
    require('0 passed; 4 failed' in b.text('unit-v7/typed_programs.log') and 'both `dylib` and `cdylib`' in b.text('unit-v7/typed_programs.log'), 'v7 original failure absent')
    hashes = dict(line.split(maxsplit=1)[::-1] for line in b.text('build-v8/source-before.sha256').splitlines())
    for name in (LIB, CALLER, TEST):
        short = name.removeprefix('toolchain/resident/')
        require(hashes[short] == digest(b.raw('candidate/' + name)), 'built candidate source mismatch')
        require(short + ': OK' in b.text('build-v8/source-after.log'), 'source after gate missing')
    require(b.text('build-v8/engine-identity.sha256').split()[0] == NEW, 'native tool identity')


def validate_exports(b):
    groups = {'native-v8-build': ('build-v8', 4), 'native-v8-unit': ('unit-v8', 3),
              'native-v7-rejected': ('build-v7', 2)}
    ids = set()
    for group, (folder, count) in groups.items():
        rows = [json.loads(line) for line in b.text(f'recorder-exports/{group}/2026-10.jsonl').splitlines()]
        require(len(rows) == count, 'slim export count')
        raw_ids = {p.name for p in b.path(folder + '/raw-runs').iterdir()}
        require({r['id'] for r in rows} == raw_ids, 'slim/raw recorder IDs differ')
        for row in rows:
            run_id = row['id']
            require(run_id not in ids, 'duplicate recorder ID')
            ids.add(run_id)
            raw = b.json(f'{folder}/raw-runs/{run_id}/summary.json')
            meta = b.json(f'{folder}/raw-runs/{run_id}/meta.json')
            require(meta['id'] == raw['id'] == run_id and row['machine'] == group, 'recorder provenance')
            require(meta['subcommand'] == raw['subcommand'] == row['subcommand'], 'recorder subcommand')
            for key in ('exit', 'diagnostics'):
                require(raw[key] == row[key], 'slim/raw recorder verdict differs')
            # Slim export intentionally removes slowest names, anonymizes binaries,
            # and rounds floats to milliseconds. Retain and verify that transform.
            tests = {key: (round(value, 3) if isinstance(value, float) else value)
                     for key, value in raw['tests'].items() if key not in ('slowest', 'binaries')}
            tests['binaries'] = [dict(
                (key, 'bin' + str(index) if key == 'name' else round(value, 3) if isinstance(value, float) else value)
                for key, value in binary.items()) for index, binary in enumerate(raw['tests']['binaries'])]
            require(tests == row['tests'], 'slim/raw test counts or timing transform differ')
            if row['tests']['passed'] in (5, 49):
                require(meta['subcommand'] == 'test' and row['exit'] == 0, 'test export nonzero')
            require(row['exit'] == (101 if row['tests']['failed'] == 4 else 0), 'export expected verdict')
    require(len(ids) == 9, 'nine recorder receipts required')
    return sorted(ids)


def validate_suite(b, run):
    suite, meta = b.json(run + '/suite.json'), b.json(run + '/metadata.json')
    records, summary = b.json(run + '/records.json'), b.json(run + '/summary.json')
    require(suite['schema'] == 'native_resident/v1' and suite['metadata'] == meta, 'suite metadata mismatch')
    for name, sha in suite['artifacts_sha256'].items():
        require(digest(b.raw(run + '/' + name)) == sha, 'suite artifact mismatch: ' + name)
    require(meta['status'] == 'complete' and meta['source_restored'] is True and meta['unchanged'] is True, 'suite not complete/restored')
    for key in ('source', 'tools', 'toolchain'):
        require(meta[key + '_before'] == meta[key + '_after'], 'suite changed ' + key)
    require(meta['tools_before']['reference']['sha256'] == REFERENCE, 'reference tool changed')
    require(meta['tools_before']['engine']['sha256'] == (OLD if run == 'rsa01' else NEW), 'engine tool changed')
    require(summary['excluded'] == [] and summary['source_built_parity'] is True, 'suite exclusions/parity')
    expected = {(stack, phase, 0) for stack in ('reference', 'engine') for phase in ('version', 'prepare', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery')}
    expected |= {(stack, 'edit', rep) for stack in ('reference', 'engine') for rep in range(5)}
    keys = [(r['stack'], r['phase'], r['replicate']) for r in records]
    require(len(records) == 22 and set(keys) == expected, 'suite observation completeness')
    samples = {stack: [] for stack in ('reference', 'engine')}
    for row in records:
        stack, phase, rep = row['stack'], row['phase'], row['replicate']
        prefix = f'{run}/{stack}-{phase}-{rep}'
        output = b.text(prefix + '.stdout') + '\n' + b.text(prefix + '.stderr')
        require(row['error'] is None and row['timed_out'] is False, 'suite infrastructure failure')
        if phase in ('type-error', 'assert-failure'):
            require(row['status'] != 0 and row['gate_passed'] is True, 'expected failure missing')
            if phase == 'assert-failure':
                require(runtime_tests(output, True) == row['parsed'], 'failure parsed mismatch')
            else:
                require("error[E0308]: expected u64, found &'static str" in output and 'JR_RUNTIME_TYPE_ERROR' in output, 'type diagnostic absent')
        else:
            successful(row)
            if phase != 'version':
                require(runtime_tests(output) == row['parsed'], 'parsed tests mismatch')
        if phase != 'version':
            require(row['command'][1:] == (['cargo'] if stack == 'reference' else []) + COMMAND, 'wrong runtime command')
        if phase == 'edit':
            require(row['accepted'] is True and re.findall(r'JR_RUNTIME_EXECUTED_\d+', output) == [f'JR_RUNTIME_EXECUTED_{rep + 1}'], 'fresh runtime marker')
            samples[stack].append(row['elapsed_seconds'])
    for stack, values in samples.items():
        require(suite['measurements'][stack]['samples_seconds'] == values, 'sample list mismatch')
        median = statistics.median(values)
        require(suite['measurements'][stack]['median_seconds'] == median == summary['medians_seconds'][stack], 'median mismatch')
    return summary['medians_seconds']


def validate_parity(b):
    failed = b.json('parity-failed-dispatch/state.json')
    require(failed['status'] == 'failed' and len(failed['rows']) == 2 and 'unexpected failure' in failed['error'], 'initial dispatch failure missing')
    for row in failed['rows']:
        require(row['command'][1] == 'metadata' and row['status'] != 0, 'initial dispatch relabeled')
    state = b.json('parity-six-cases/state.json')
    require(state['status'] == 'passed' and len(state['rows']) == 12, 'six parity cases missing')
    expected = {(case, variant) for case in CASES for variant in ('old', 'new')}
    require({(r['case'], r['variant']) for r in state['rows']} == expected, 'parity case inventory')
    pairs = {}
    for row in state['rows']:
        case, variant = row['case'], row['variant']
        require(row['command'][1:] == ['cargo', 'metadata', '--format-version', '1'] + CASES[case], 'metadata dispatch/flags')
        require(row['tool_sha256'] == (OLD if variant == 'old' else NEW), 'parity identity')
        require(row['error'] is None and row['timed_out'] is False, 'parity process failure')
        rejection = case in ('missing-feature', 'bad-format')
        require((row['status'] != 0) == rejection, 'metadata rejection status')
        prefix = f'parity-six-cases/{case}-{variant}'
        pairs[(case, variant)] = (row['status'], b.raw(prefix + '.stdout'), b.raw(prefix + '.stderr'))
    for case in CASES:
        require(pairs[(case, 'old')] == pairs[(case, 'new')], 'metadata stdout/stderr/status differ')
    return len(CASES)


def validate_profile(b):
    state = b.json('profile/state.json')
    require(state['status'] == 'passed' and state['source_restored'] is True and state['identities_unchanged'] is True, 'profile restoration/identity')
    require(digest(b.raw('profile/original.rs')) == state['source_before'], 'profile original source')
    rows = state['rows']
    order = [('old', 'warmup'), ('new', 'warmup')]
    for rep in range(5):
        order += [(name, rep) for name in (('old', 'new') if rep % 2 == 0 else ('new', 'old'))]
    require([(r['stack'], r['replicate']) for r in rows] == order, 'profile 12-row alternating order')
    selected = {stack: {} for stack in ('old', 'new')}
    wanted = ('client planning', 'client metadata', 'client command', 'apply change', 'roots')
    for row in rows:
        successful(row)
        stack, rep = row['stack'], row['replicate']
        require(row['command'][1:] == COMMAND, 'profile runtime command')
        require(row['tool_sha256'] == (OLD if stack == 'old' else NEW), 'profile tool identity')
        prefix = f'profile/{stack}-{rep}'
        trace_raw = b.raw(prefix + '.trace.json')
        require(digest(trace_raw) == row['trace_sha256'], 'trace hash mismatch')
        events = json.loads(trace_raw)['traceEvents']
        actual = {}
        for event in events:
            if event.get('ph') == 'X' and event.get('name') in wanted:
                require(isinstance(event.get('dur'), (int, float)) and event['dur'] >= 0, 'invalid trace duration')
                actual.setdefault(event['name'], []).append(event['dur'])
        require(actual == row['selected_nested_elapsed_us'] and len(actual.get('client planning', [])) == 1, 'raw trace phase mismatch')
        output = b.text(prefix + '.stdout') + '\n' + b.text(prefix + '.stderr')
        runtime_tests(output)
        if rep != 'warmup':
            require(re.findall(r'JR_RUNTIME_EXECUTED_\d+', output) == [f'JR_RUNTIME_EXECUTED_{rep + 501}'], 'profile fresh edit marker')
            for name, values in actual.items():
                selected[stack].setdefault(name, []).extend(values)
    medians = {stack: {name: statistics.median(values) for name, values in phases.items()} for stack, phases in selected.items()}
    require(medians['old']['client planning'] == 21160 and medians['new']['client planning'] == 20599, 'planning observation changed')
    return medians


def validate(root):
    b = Bundle(root)
    manifest = b.json('manifest.json')
    require(manifest['decision'] == 'REJECTED_NOT_PROMOTED' and manifest['baseline_commit'] == '23a1490cb84fa0aaca872dcf6a430f35e19a3863', 'rejection/provenance')
    excluded = {'manifest.json', 'validation.json', 'README.md', 'validate.py', 'test_validate.py', 'make_manifest.py'}
    inventory = set()
    for path in b.root.rglob('*'):
        if '__pycache__' in path.parts:
            continue
        require(not path.is_symlink(), 'symlink in evidence bundle')
        if path.is_file() and path.relative_to(b.root).as_posix() not in excluded:
            name = path.relative_to(b.root).as_posix()
            require(not b.raw(name).startswith(b'\x7fELF') and path.suffix not in ('.rlib', '.rmeta', '.so', '.sqlite', '.db'), 'binary/cache in bundle')
            inventory.add(name)
    require(inventory == set(manifest['files_sha256']), 'incomplete/unindexed evidence inventory')
    for name, sha in manifest['files_sha256'].items():
        require(digest(b.raw(name)) == sha, 'bundle hash mismatch: ' + name)
    for name, sha in SOURCE_HASHES.items():
        require(digest(b.raw(name)) == sha, 'exact source hash mismatch')
    require(set(SOURCE_HASHES).issubset(manifest['files_sha256']), 'source not indexed')
    validate_patch(b)
    validate_gates(b)
    recorder_ids = validate_exports(b)
    suites = {run: validate_suite(b, run) for run in ('rsa01', 'rsa04', 'rsa05')}
    cases = validate_parity(b)
    phases = validate_profile(b)
    baseline, repeat = suites['rsa01'], suites['rsa05']
    raw = (repeat['engine'] / baseline['engine'] - 1) * 100
    normalized = ((repeat['engine'] / repeat['reference']) / (baseline['engine'] / baseline['reference']) - 1) * 100
    require(raw > 0 and abs(normalized) < 10, 'rejection floor contradicted')
    return dict(status='PASS', decision='REJECTED_NOT_PROMOTED', source_hashes=SOURCE_HASHES,
                native_tests=dict(typed=5, existing_metadata_manifest=49), metadata_parity_cases=cases,
                recorder_ids=recorder_ids,
                profile_rows=12, edited_rows=10, nested_elapsed_medians_us=phases,
                suites_seconds=suites, repeat_engine_raw_change_percent=raw,
                repeat_reference_normalized_change_percent=normalized,
                floor_percent=10, causal_savings_claim=False, default_promotion=False,
                limitations='Offline receipt consistency, not rerun or proof of causation. Compatibility-control proc macros. Noise unknown. One scoped workload, not comprehensive.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('bundle', nargs='?', type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    try:
        print(json.dumps(validate(args.bundle), indent=2, sort_keys=True))
    except (ValueError, KeyError, OSError, TypeError, IndexError) as error:
        parser.exit(1, 'FAIL: ' + str(error) + '\n')


if __name__ == '__main__':
    main()
