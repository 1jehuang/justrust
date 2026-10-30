#!/usr/bin/env python3
"""Real owned-compiler acceptance for green-query reuse profiling.

Direct rustc invocation is intentional: this tests the compiler's private
self-profile protocol, not Cargo routing or a build performance claim.
Outputs are private and retained. Never overwrite an existing capture root.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

SOURCE = '''#![allow(dead_code)]
trait Number { fn number(&self) -> u64; }
impl Number for u64 { fn number(&self) -> u64 { *self } }
fn sum<T: Number>(xs: &[T]) -> u64 { xs.iter().map(Number::number).sum() }
fn edited() -> u64 { 7 }
fn main() { println!("{}", sum(&[1u64, 2, 3]) + edited()); }
'''


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command):
    p = subprocess.run(command, capture_output=True, text=True)
    return dict(command=[str(x) for x in command], status=p.returncode,
                stdout=p.stdout, stderr=p.stderr)


def capture(rustc, root):
    root.mkdir(mode=0o700, parents=False, exist_ok=False)
    source = root / 'fixture.rs'
    source.write_text(SOURCE)
    binary = root / 'fixture'
    base = [str(rustc), str(source), '--edition=2024', '--crate-name=reuse_fixture',
            '-C', f'incremental={root / "incremental"}', '-C', 'debuginfo=1',
            '-Zincremental-verify-ich', '-o', str(binary)]
    version = run([str(rustc), '-vV'])
    if version['status']:
        raise RuntimeError(version)
    observations = []
    for name, content, expected in (
        ('initial', SOURCE, '13\n'),
        ('unchanged', SOURCE, '13\n'),
        ('body-edit', SOURCE.replace('{ 7 }', '{ 8 }'), '14\n'),
        ('revert', SOURCE, '13\n'),
        ('type-error', SOURCE.replace('{ 7 }', '{ "wrong" }'), None),
    ):
        source.write_text(content)
        prefix = root / name
        profiled = run(base + [f'-Zself-profile={prefix}'])
        before = digest(binary) if profiled['status'] == 0 else None
        output = run([str(binary)]) if profiled['status'] == 0 else None
        plain = run(base)
        after = digest(binary) if plain['status'] == 0 else None
        plain_output = run([str(binary)]) if plain['status'] == 0 else None
        if profiled['status'] != plain['status']:
            raise AssertionError(f'{name}: profiling changed exit status')
        if profiled['stderr'] != plain['stderr']:
            raise AssertionError(f'{name}: profiling changed diagnostics')
        if expected is not None:
            if profiled['status'] or before != after or output != plain_output:
                raise AssertionError(json.dumps(dict(name=name, profiled=profiled,
                    plain=plain, artifact_sha256=before, plain_artifact_sha256=after,
                    output=output, plain_output=plain_output), indent=2))
            if output['status'] or output['stdout'] != expected:
                raise AssertionError(f'{name}: incorrect program result')
        elif profiled['status'] == 0:
            raise AssertionError('type-error unexpectedly compiled')
        observations.append(dict(name=name, profiled=profiled, plain=plain,
            artifact_sha256=before, plain_artifact_sha256=after,
            artifacts_equal=before == after if before else None, program=output))
    report = dict(schema=1, scope='native-telemetry-correctness-not-speedup',
                  compiler=version, observations=observations)
    (root / 'acceptance.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rustc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    result = capture(args.rustc.resolve(), args.output.resolve())
    print(json.dumps(dict(output=str(args.output), cases=len(result['observations']),
                         verdict='passed'), indent=2))


if __name__ == '__main__':
    main()
