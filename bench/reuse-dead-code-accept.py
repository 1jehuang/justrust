#!/usr/bin/env python3
"""Exact native dead-code item-retention differential gate, not a speed benchmark.

All invocations share the same source and output paths. Diagnostics are compared
as bytes, without even path normalization. Each revision is checked against fresh
off/on references, persistent off/on caches, and no/yes/no in one transition cache.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

FLAG = '-Zjr-cache-dead-code-items'
BASE = '''#![warn(dead_code, dead_code_pub_in_binary, unfulfilled_lint_expectations)]
#[derive(Debug, Clone)]
struct Data { used: u32, ignored: u32 }
#[repr(C)] struct CLayout { a: u8, b: u8 }
union Bits { number: u32, bytes: [u8; 4] }
trait Dispatch { fn number(&self) -> u32; fn opaque(&self) -> impl Copy { 1u8 } }
impl Dispatch for Data { fn number(&self) -> u32 { self.used } }
impl Data { fn inherent(&self) -> u32 { self.number() } fn unused_method(&self) {} }
fn leaf() -> u32 { 1 }
fn other() -> u32 { 2 }
fn unused_root() { other(); }
#[allow(dead_code)] fn allowed() { middle(); }
fn middle() { expected(); }
#[expect(dead_code)] fn expected() { leaf(); }
#[expect(dead_code)] fn fulfilled() {}
#[expect(dead_code)] fn unfulfilled() {}
async fn asynchronous() -> u32 { let nested = || leaf(); nested() }
macro_rules! generated { () => { fn from_macro() -> u32 { leaf() } }; }
generated!();
fn value() -> u32 {
    let data = Data { used: from_macro(), ignored: 7 };
    let _ = format!("{:?}", data.clone());
    let _ = CLayout { a: 0, b: 0 };
    let bits = Bits { number: 1 };
    let _ = unsafe { bits.number };
    let _future = asynchronous();
    let nested = || { fn inside() -> u32 { 0 } inside() };
    unfulfilled();
    data.inherent() + nested()
}
fn main() { println!("{}", value()); }
'''


def cases():
    """Ordered edit/recovery matrix. None denotes an ordinary rustc error."""
    return [
        ('initial', BASE, '1\n'),
        ('unchanged', BASE, '1\n'),
        ('body-callgraph-edit', BASE.replace('used: from_macro()', 'used: other()'), '2\n'),
        ('body-revert', BASE, '1\n'),
        ('unused-root-live', BASE.replace('    unfulfilled();', '    unused_root(); unfulfilled();'), '1\n'),
        ('field-read', BASE.replace('data.inherent() + nested()', 'data.inherent() + nested() + data.ignored'), '8\n'),
        ('repr-transparent', BASE.replace('#[repr(C)] struct CLayout { a: u8, b: u8 }', '#[repr(transparent)] struct CLayout { a: u8, b: () }').replace('CLayout { a: 0, b: 0 }', 'CLayout { a: 0, b: () }'), '1\n'),
        ('repr-union', BASE.replace('union Bits', '#[repr(C)] union Bits'), '1\n'),
        ('union-field-switch', BASE.replace('Bits { number: 1 }', 'Bits { bytes: [0; 4] }'), '1\n'),
        ('trait-call-edit', BASE.replace('fn number(&self) -> u32 { self.used }', 'fn number(&self) -> u32 { other() }'), '2\n'),
        ('inherent-call-edit', BASE.replace('fn inherent(&self) -> u32 { self.number() }', 'fn inherent(&self) -> u32 { other() }'), '2\n'),
        ('rpitit-live', BASE.replace('    unfulfilled();', '    let _ = data.opaque(); unfulfilled();'), '1\n'),
        ('derive-order', BASE.replace('derive(Debug, Clone)', 'derive(Clone, Debug)'), '1\n'),
        ('derive-remove-debug', BASE.replace('Debug, Clone', 'Clone').replace('    let _ = format!("{:?}", data.clone());', '    let _ = data.clone();'), '1\n'),
        ('nested-closure-async', BASE.replace('let nested = || leaf();', 'let _ = async || other(); let nested = || other();'), '1\n'),
        ('macro-expansion-edit', BASE.replace('fn from_macro() -> u32 { leaf() }', 'fn from_macro() -> u32 { other() }'), '2\n'),
        ('allow-to-expect', BASE.replace('#[allow(dead_code)] fn allowed()', '#[expect(dead_code)] fn allowed()'), '1\n'),
        ('allow-propagation-remove', BASE.replace('#[allow(dead_code)] fn allowed()', 'fn allowed()'), '1\n'),
        ('expected-becomes-live', BASE.replace('    unfulfilled();', '    expected(); unfulfilled();'), '1\n'),
        ('fulfilled-becomes-unfulfilled', BASE.replace('    unfulfilled();', '    fulfilled(); unfulfilled();'), '1\n'),
        ('unfulfilled-becomes-fulfilled', BASE.replace('    unfulfilled();', ''), '1\n'),
        ('expect-unused-propagation', BASE.replace('#[expect(dead_code)] fn expected()', '#[expect(unused)] fn expected()'), '1\n'),
        ('deny-dead-code', BASE.replace('#![warn(', '#![deny('), None),
        ('deny-recovery', BASE, '1\n'),
        ('deny-unfulfilled', '#![deny(unfulfilled_lint_expectations)]\n' + BASE.replace('dead_code_pub_in_binary, unfulfilled_lint_expectations', 'dead_code_pub_in_binary'), None),
        ('expectation-recovery', BASE, '1\n'),
        ('type-error', BASE.replace('fn leaf() -> u32 { 1 }', 'fn leaf() -> u32 { "wrong" }'), None),
        ('type-error-recovery', BASE, '1\n'),
        ('owner-churn', BASE.replace('fn leaf()', 'fn inserted_owner() {}\nfn leaf()'), '1\n'),
        ('variant-pattern', BASE + '\nenum E { A, B }\nfn pattern(e: E) -> E { match e { E::A => E::A, E::B => E::B } }\n', '1\n'),
        ('variant-pattern-live', BASE.replace('    unfulfilled();', '    let _ = pattern(E::A); unfulfilled();') + '\nenum E { A, B }\nfn pattern(e: E) -> E { match e { E::A => E::A, E::B => E::B } }\n', '1\n'),
        ('final-recovery', BASE, '1\n'),
    ]


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def run(command, timeout=180):
    command = [str(x) for x in command]
    started = time.monotonic()
    result = subprocess.run(command, capture_output=True, timeout=timeout)
    return dict(command=command, status=result.returncode,
                stdout_b64=base64.b64encode(result.stdout).decode('ascii'),
                stderr_b64=base64.b64encode(result.stderr).decode('ascii'),
                elapsed_seconds=time.monotonic() - started)


def observable(record):
    return {key: record[key] for key in ('status', 'stdout_b64', 'stderr_b64')}


def compare(reference, actual, label):
    if observable(reference) != observable(actual):
        raise AssertionError(label + ': exact status/diagnostic/output bytes differ\n' +
                             json.dumps(dict(reference=reference, actual=actual), indent=2))


def schedule(index):
    # All three transition invocations use precisely the same incremental path.
    return [('fresh-no', False, f'fresh/{index}/no'),
            ('fresh-yes', True, f'fresh/{index}/yes'),
            ('persistent-no', False, 'persistent/no'),
            ('persistent-yes', True, 'persistent/yes'),
            ('transition-no-before', False, 'transition'),
            ('transition-yes', True, 'transition'),
            ('transition-no-after', False, 'transition')]


def compile_case(rustc, root, source, enabled, cache):
    source_path = root / 'fixture.rs'
    binary = root / 'fixture'
    source_path.write_text(source)
    binary.unlink(missing_ok=True)
    command = [rustc, source_path, '--crate-name=dead_code_retention', '--edition=2024',
               '-Zincremental-verify-ich', '-Cdebuginfo=0',
               f'{FLAG}={"yes" if enabled else "no"}', '-C',
               f'incremental={root / "incremental" / cache}', '-o', binary]
    result = dict(compile=run(command))
    if result['compile']['status'] == 0:
        result['runtime'] = run([binary])
        result['artifact_sha256'] = digest(binary)
    return result


def validate_expected(record, expected, label):
    status = record['compile']['status']
    if expected is None:
        if status != 1:
            raise AssertionError(f'{label}: expected ordinary compiler error (1), got {status}')
        return
    if status != 0:
        raise AssertionError(f'{label}: unexpected compiler failure\n' + json.dumps(record, indent=2))
    actual = observable(record['runtime'])
    wanted = dict(status=0, stdout_b64=base64.b64encode(expected.encode()).decode('ascii'), stderr_b64='')
    if actual != wanted:
        raise AssertionError(f'{label}: wrong runtime result\n' + json.dumps(record, indent=2))


def capture(rustc, root):
    root.mkdir(mode=0o700, parents=False, exist_ok=False)
    report = dict(schema=1, flag=FLAG, scope='correctness-only-no-speedup-claim',
                  verify_ich=True, diagnostics='exact-bytes-no-normalization',
                  planned_cases=len(cases()), completed_cases=0, verdict='incomplete', observations=[])
    try:
        report['compiler'] = dict(path=str(rustc), sha256=digest(rustc), version=run([rustc, '-vV']))
        sysroot = run([rustc, '--print', 'sysroot'])
        report['sysroot_query'] = sysroot
        if report['compiler']['version']['status'] or sysroot['status']:
            raise AssertionError('Compiler identity query failed')
        sysroot_path = Path(base64.b64decode(sysroot['stdout_b64']).decode().strip())
        drivers = sorted((sysroot_path / 'lib').glob('*rustc_driver*'))
        report['drivers'] = [dict(path=str(p.resolve()), sha256=digest(p)) for p in drivers if p.is_file()]
        if not report['drivers']:
            raise AssertionError('No compiler driver library found in sysroot/lib')
        for index, (name, source, expected) in enumerate(cases()):
            observation = dict(name=name, source=source, expected_stdout=expected, modes={})
            report['observations'].append(observation)
            reference = None
            for mode, enabled, cache in schedule(index):
                result = compile_case(rustc, root, source, enabled, cache)
                observation['modes'][mode] = result
                validate_expected(result, expected, f'{name}/{mode}')
                if reference is None:
                    reference = result
                else:
                    compare(reference['compile'], result['compile'], f'{name}/{mode}/compile')
                    if expected is not None:
                        compare(reference['runtime'], result['runtime'], f'{name}/{mode}/runtime')
            report['completed_cases'] += 1
        report['verdict'] = 'passed'
    except Exception as error:
        report['verdict'] = 'failed'
        report['error'] = str(error)
        raise
    finally:
        (root / 'acceptance.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rustc', '--compiler', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='New directory whose parent exists')
    args = parser.parse_args()
    os.umask(0o077)
    report = capture(args.rustc.resolve(), args.output.resolve())
    print(json.dumps(dict(output=str(args.output), verdict=report['verdict'], cases=report['completed_cases'])))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
