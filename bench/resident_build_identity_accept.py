#!/usr/bin/env python3
"""Execute the resident build script against independent identity fixtures.

This is a build-script correctness gate, not a compiler performance benchmark.
No subprocesses run on import. Reports retain exact source/compiler identities.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ORIGIN = '5d4f78f7577cf7170993b74f735099dd03f5c6ca'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def directives(text):
    values = {}
    for line in text.splitlines():
        if line.startswith('cargo:rustc-env='):
            name, value = line[len('cargo:rustc-env='):].split('=', 1)
            values[name] = value
    return values


def expected_key(root, target='identity-test-target', features=(), debug=False):
    files = sorted([p for top in ('crates', 'vendor') for p in (root / top).rglob('*')
                    if p.is_file()] + [root / name for name in
                    ('Cargo.toml', 'Cargo.lock', '.cargo/config.toml', 'harness/toolchain')])
    fields = [b'justrust-resident-v1', ORIGIN.encode()]
    for path in files:
        fields.extend((path.relative_to(root).as_posix().encode(), path.read_bytes()))
    fields.extend((' '.join(sorted(features)).encode(), target.encode(), bytes([debug])))
    h = hashlib.sha256()
    for field in fields:
        h.update(len(field).to_bytes(8, 'little'))
        h.update(field)
    return h.hexdigest()


def execute(args):
    if sys.flags.optimize:
        raise ValueError('identity acceptance requires assertions enabled')
    out = args.output.resolve()
    out.mkdir(mode=0o700, parents=True, exist_ok=False)
    source = args.source.resolve()
    rustc = args.rustc.resolve()
    report = {'schema': 1, 'scope': 'resident-build-script-identity-only', 'status': 'running',
              'source_sha256': digest(source), 'rustc_sha256': digest(rustc), 'rows': []}
    report_path = out / 'acceptance.json'

    def save():
        report_path.write_text(json.dumps(report, indent=2) + '\n')

    def run(name, command, env):
        result = subprocess.run(list(map(str, command)), env=env, capture_output=True, timeout=60)
        (out / f'{name}.stdout').write_bytes(result.stdout)
        (out / f'{name}.stderr').write_bytes(result.stderr)
        row = {'name': name, 'command': list(map(str, command)), 'status': result.returncode,
               'values': directives(result.stdout.decode(errors='strict'))}
        report['rows'].append(row)
        save()
        return row

    save()
    try:
        env = {k: v for k, v in os.environ.items() if not k.startswith(('CARGO_', 'RA_JIT_', 'JR_ENGINE_'))}
        env.update(CARGO_CFG_TARGET_OS='linux', TARGET='identity-test-target')
        binary = out / 'build-script'
        row = run('compile', [rustc, '--edition=2024', source, '-o', binary], env)
        assert row['status'] == 0, 'build-script compilation failed'
        row = run('missing-workspace', [binary], env)
        assert row['status'] == 0
        assert row['values']['JR_ENGINE_ORIGIN'] == ORIGIN
        assert row['values']['JR_ENGINE_BUILD_ID'] == 'resident-v1-unidentified'
        assert row['values']['RA_JIT_GIT_SHA'] == f'blaze-{ORIGIN}+justrust-local'
        root = out / 'fixture'
        for name, data in {'crates/a/src/lib.rs': 'pub fn a() {}', 'vendor/b/src/lib.rs': '// b',
                           'Cargo.toml': '[workspace]', 'Cargo.lock': 'version=4',
                           '.cargo/config.toml': '# config', 'harness/toolchain': 'nightly-test'}.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(data)
        env['CARGO_WORKSPACE_DIR'] = str(root)
        baseline = None
        for name, changes, debug in [('baseline', {}, False),
                                     ('target-change', {'TARGET': 'another-target'}, False),
                                     ('feature-change', {'CARGO_FEATURE_LLVM': '1'}, False),
                                     ('debug-change', {'CARGO_CFG_DEBUG_ASSERTIONS': '1'}, True),
                                     ('explicit-label', {'RA_JIT_GIT_SHA': 'explicit-local-label'}, False)]:
            current = dict(env, **changes)
            row = run(name, [binary], current)
            assert row['status'] == 0
            features = [k for k in current if k.startswith('CARGO_FEATURE_')]
            expected = expected_key(root, current['TARGET'], features, debug)
            assert row['values']['RA_JIT_SOURCES'] == expected, name
            assert row['values']['JR_ENGINE_BUILD_ID'] == 'resident-v1-' + expected, name
            if name == 'baseline':
                baseline = expected
            elif name == 'explicit-label':
                assert expected == baseline
                assert row['values']['RA_JIT_GIT_SHA'] == 'explicit-local-label'
            else:
                assert expected != baseline
        (root / 'crates/a/src/lib.rs').write_text('pub fn a() { let _ = 1; }')
        row = run('source-change', [binary], env)
        assert row['status'] == 0
        assert row['values']['RA_JIT_SOURCES'] == expected_key(root)
        assert row['values']['RA_JIT_SOURCES'] != baseline
        (root / 'harness/toolchain').unlink()
        row = run('missing-declared-input', [binary], env)
        assert row['status'] != 0, 'missing declared source input must fail closed'
        assert 'RA_JIT_SOURCES' not in row['values']
        assert row['values']['JR_ENGINE_BUILD_ID'] == 'resident-v1-unidentified'
        assert digest(source) == report['source_sha256']
        assert digest(rustc) == report['rustc_sha256']
        report['status'] = 'passed'
    except BaseException as error:
        report['status'] = 'failed'
        report['error'] = repr(error)
        raise
    finally:
        save()
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rustc', type=Path, required=True)
    parser.add_argument('--source', type=Path, default=Path(__file__).resolve().parents[1] /
                        'toolchain/resident/crates/ra-jit/build.rs')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = execute(args)
    print(json.dumps({'status': report['status'], 'observations': len(report['rows'])}))


if __name__ == '__main__':
    main()
