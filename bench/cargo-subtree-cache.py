#!/usr/bin/env python3
"""Paired native Cargo recursive-input validation benchmark, not the broad gate.

Uses the same generated source, rustc and target directory for both executables.
Fails if any measured warm invocation reruns its build script or compiler.
The candidate must also rebuild after a watched edit, addition and deletion.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cargo', type=Path, help='one candidate binary measured with the native gate off/on')
    parser.add_argument('--before', type=Path)
    parser.add_argument('--after', type=Path)
    parser.add_argument('--rustc', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('-n', type=int, default=16)
    args = parser.parse_args()
    if args.cargo:
        if args.before or args.after:
            parser.error('--cargo cannot be combined with --before/--after')
        args.before = args.after = args.cargo
    if not args.before or not args.after or args.n < 1:
        parser.error('provide --cargo or both --before/--after, and positive -n')
    executables = {'before': args.before.resolve(), 'after': args.after.resolve()}
    env = dict(os.environ, RUSTC=str(args.rustc.resolve()), CARGO_INCREMENTAL='0')
    env.pop('RUSTC_WRAPPER', None)
    env.pop('RUSTC_WORKSPACE_WRAPPER', None)
    env.pop('JUSTRUST_CARGO_SUBTREE_CACHE', None)
    environments = {key: dict(env, JUSTRUST_CARGO_SUBTREE_CACHE='1' if key == 'after' else '0') for key in executables}
    results = {'native_gate': {'before': '0', 'after': '1', 'default': 'unset'},
               'same_binary': executables['before'] == executables['after'],
               'scope': 'native Cargo warm overlapping rerun-if-changed validation',
               'identity': {key: {'sha256': digest(binary), 'version': subprocess.check_output([binary, '-vV'], text=True)}
                            for key, binary in executables.items()},
               'rustc': {'sha256': digest(args.rustc), 'version': subprocess.check_output([args.rustc, '-vV'], text=True)},
               'files': 3072, 'samples': [], 'correctness': []}
    scratch = Path(os.environ.get('JCODE_SCRATCH_DIR', tempfile.gettempdir()))
    with tempfile.TemporaryDirectory(prefix='cargo-subtree-', dir=scratch) as temporary:
        root = Path(temporary)
        for index in range(48):
            directory = root / 'inputs' / f'd{index}' / 'nested'
            directory.mkdir(parents=True)
            for file in range(64):
                (directory / f'f{file}').write_text('1')
        (root / 'src').mkdir()
        (root / 'src/lib.rs').write_text('include!(concat!(env!("OUT_DIR"), "/generated.rs"));\n')
        (root / 'Cargo.toml').write_text('[package]\nname="cargo-subtree-fixture"\nversion="0.0.0"\nedition="2024"\n[workspace]\n')

        def run(which, measured=False):
            start = time.perf_counter_ns()
            proc = subprocess.run([executables[which], 'check', '--offline', '-v'], cwd=root, env=environments[which],
                                  text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            elapsed = time.perf_counter_ns() - start
            if proc.returncode:
                raise RuntimeError(proc.stdout + proc.stderr)
            if measured and ('Compiling ' in proc.stderr or 'Running `' in proc.stderr):
                raise RuntimeError('warm sample rebuilt units: ' + proc.stderr)
            return elapsed

        for scenario in ['single', 'overlapping']:
            directives = ['inputs']
            if scenario == 'overlapping':
                directives += [f'inputs/d{i}{suffix}' for i in range(48) for suffix in ['', '/nested']]
            script = 'fn main() {\n'
            script += ''.join(f'println!("cargo::rerun-if-changed={path}");\n' for path in directives)
            script += 'let value = std::fs::read_to_string("inputs/d0/nested/f0").unwrap();\n'
            script += 'let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());\n'
            script += 'std::fs::write(out.join("generated.rs"), format!("pub const VALUE: u32 = {};", value)).unwrap();\n}\n'
            (root / 'build.rs').write_text(script)
            run('before')
            run('after', measured=True)
            default = subprocess.run([executables['after'], 'check', '--offline', '-v'], cwd=root, env=env, text=True, capture_output=True)
            if default.returncode or 'Compiling ' in default.stderr or 'Running `' in default.stderr:
                raise RuntimeError('unset gate did not reuse artifacts: ' + default.stderr)
            results['correctness'].append(f'{scenario}: unset default gate reuses off/on artifacts')
            for trial in range(args.n):
                order = ['before', 'after'] if trial % 2 == 0 else ['after', 'before']
                for which in order:
                    elapsed = run(which, measured=True)
                    results['samples'].append({'scenario': scenario, 'trial': trial, 'which': which, 'elapsed_ns': elapsed})

        def require_rebuild(action):
            time.sleep(.02)
            action()
            proc = subprocess.run([executables['after'], 'check', '--offline', '-v'], cwd=root, env=environments['after'],
                                  text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            if proc.returncode or 'Running `' not in proc.stderr:
                raise RuntimeError('changed input did not rebuild: ' + proc.stderr)
            run('after', measured=True)

        require_rebuild(lambda: (root / 'inputs/d0/nested/f0').write_text('2'))
        generated = list((root / 'target/debug/build').rglob('generated.rs'))
        assert generated and all(path.read_text() == 'pub const VALUE: u32 = 2;' for path in generated), [path.read_text() for path in generated]
        results['correctness'].append('watched edit changed compiled generated constant')
        added = root / 'inputs/d1/nested/new'
        require_rebuild(lambda: added.write_text('new'))
        results['correctness'].append('watched nested addition reran build script and compiler')
        require_rebuild(added.unlink)
        results['correctness'].append('watched nested deletion reran build script and compiler')
        results['correctness'].append('all paired warm calls reused artifacts across Cargo executables')
    results['summary'] = {}
    for scenario in ['single', 'overlapping']:
        medians = {which: statistics.median(s['elapsed_ns'] for s in results['samples']
                                          if s['scenario'] == scenario and s['which'] == which)
                   for which in executables}
        medians['delta_percent'] = 100 * (medians['after'] / medians['before'] - 1)
        results['summary'][scenario] = medians
    for key, binary in executables.items():
        assert digest(binary) == results['identity'][key]['sha256'], 'Cargo binary changed during measurement'
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(results['summary'], indent=2))
    print(json.dumps(results['correctness'], indent=2))


if __name__ == '__main__':
    main()
