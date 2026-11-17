#!/usr/bin/env python3
"""Frozen owned lexer correctness acceptance. Never builds a compiler or claims speed.

CLI: --compiler /explicit/frozen/jr-rustc --out /new/evidence/directory
API: capture(compiler, out, timeout=180). Raw transport and receipts survive failure.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import uuid

import stable_resident as shared
import stable_resident_owned as owned

COUNTERS = ('hits', 'misses', 'rejected', 'retained_entries', 'retained_estimated_bytes')
ROOT = 'mod module_a;\nmod module_b;\n'


def environment(mode):
    if mode not in ('control', '1'):
        raise ValueError('Expected control or 1')
    env = owned.environment()
    env['JUSTRUST_RUSTC_RESIDENT_LEXER'] = '0' if mode == 'control' else '1'
    return env


class LexerResident(owned.OwnedResident):
    def __init__(self, compiler, root, mode, timeout=180):
        self.command = [str(compiler), '--jr-resident-owned']
        self.root, self.timeout = root, timeout
        self.stdout = (root / 'resident.stdout').open('w+b', buffering=0)
        self.stderr = (root / 'resident.stderr').open('w+b', buffering=0)
        self.transcript = (root / 'resident.jsonl').open('wb')
        read_fd, write_fd = os.pipe()
        env = environment(mode)
        env['JUSTRUST_RUSTC_RESIDENT_FD'] = str(write_fd)
        try:
            self.process = subprocess.Popen(self.command, stdin=subprocess.PIPE,
                stdout=self.stdout, stderr=self.stderr, bufsize=0, cwd=root,
                env=env, pass_fds=(write_fd,))
        except BaseException:
            os.close(read_fd)
            self.stdout.close()
            self.stderr.close()
            self.transcript.close()
            raise
        finally:
            os.close(write_fd)
        self.response_fd = read_fd
        self.selector = selectors.DefaultSelector()
        self.selector.register(read_fd, selectors.EVENT_READ)
        self.buffer, self.sequence, self.offsets = b'', 0, [0, 0]
        self.args = self.owned = None


def sources(a=7, b=11):
    # Attributes reject the root/module as appropriate, while clean third module
    # bytes exercise the actual lexer. Export attributes live in the root only.
    root = ROOT + '''#[unsafe(no_mangle)]
pub extern "C" fn lexer_a() -> u64 { module_a::value() }
#[unsafe(no_mangle)]
pub extern "C" fn lexer_b() -> u64 { module_b::value() }
'''
    return {'fixture.rs': root.encode(),
            'module_a.rs': ('pub fn value() -> u64 { %d + 2 }\n' % a).encode(),
            'module_b.rs': ('pub fn value() -> u64 { %d + 3 }\n' % b).encode()}


def cases():
    result = [('initial', sources(), (9, 14), 'cold'),
              ('unchanged', sources(), (9, 14), 'same'),
              ('unrelated-edit', sources(b=17), (9, 20), 'cross'),
              ('relevant-edit', sources(a=23, b=17), (25, 20), 'cross')]
    bad = sources(a=29, b=17)
    bad['module_a.rs'] = b'pub fn value() -> u64 { 29 + 2 ) }\n'
    result.append(('delimiter-error', bad, None, 'error'))
    result.append(('recovery', sources(a=31, b=19), (33, 22), 'recovery'))
    for name, body in [('comment', '/* fallback */ 37 + 2'),
                       ('raw-ident', '{ let r#value = 41; r#value + 2 }'),
                       ('unicode', '{ let café = 43; café + 2 }'),
                       ('string', '{ let _text = "fallback"; 47 + 2 }')]:
        data = sources(a=31, b=19)
        data['module_a.rs'] = ('pub fn value() -> u64 { ' + body + ' }\n').encode()
        result.append(('unsupported-' + name, data,
                       ({'comment': 39, 'raw-ident': 43, 'unicode': 45, 'string': 49}[name], 22),
                       'fallback'))
    return result


def validate_transition(previous, current, kind, seen):
    changed = [key for key in current if previous is None or current[key] != previous[key]]
    if kind == 'same' and changed:
        raise AssertionError('Unchanged source changed')
    if kind == 'cross':
        if len(changed) != 1 or changed[0] not in ('module_a.rs', 'module_b.rs'):
            raise AssertionError('Cross edit must change exactly one module')
        if current[changed[0]] in seen:
            raise AssertionError('Changed source must be globally unique')
    return changed


def validate_lexer(response, previous, mode, kind):
    if response.get('executed') is not True:
        raise AssertionError('Request not executed')
    if response.get('owned', {}).get('retirement_reason') is not None:
        raise AssertionError('Worker retired')
    stats = response.get('lexer')
    if not isinstance(stats, dict):
        raise AssertionError('Missing lexer counters')
    for key in COUNTERS:
        if type(stats.get(key)) is not int or stats[key] < 0:
            raise AssertionError('Invalid lexer counter: ' + key)
    for key in COUNTERS[:3]:
        if previous is not None and stats[key] < previous[key]:
            raise AssertionError('Cumulative lexer counter decreased')
    if stats['retained_entries'] > 128 or stats['retained_estimated_bytes'] > 8 * 1024 * 1024:
        raise AssertionError('Lexer cache exceeds 128 files / 8 MiB')
    delta = {key: stats[key] - (previous[key] if previous else 0) for key in COUNTERS[:3]}
    if mode == 'control':
        if any(stats.values()):
            raise AssertionError('Disabled lexer counters nonzero')
    elif mode != '1':
        raise ValueError('Unknown mode')
    else:
        if kind == 'cold' and delta['hits']:
            raise AssertionError('Cold lexer hit')
        if kind in ('same', 'cross'):
            expected_hits = 2 if kind == 'same' else 1
            if response['status'] != 0 or delta['hits'] != expected_hits or not stats['retained_entries']:
                raise AssertionError('Successful unchanged/cross edit requires exact expected module lexer hits')
        if kind == 'cross' and delta['misses'] < 1:
            raise AssertionError('Unique changed source requires lexer miss')
        if kind in ('error', 'fallback') and delta['rejected'] < 1:
            raise AssertionError('Unsupported/error source requires lexer rejection')
        # Errors may legitimately hit unchanged modules, but cannot be accepted
        # as a successful hit stage or execute a stale artifact.
        if kind == 'error' and response['status'] != 1:
            raise AssertionError('Delimiter error must fail')
    return dict(stats)


def arguments(root):
    return [str(root / 'fixture.rs'), '--crate-name=stable_resident_lexer_fixture',
            '--crate-type=lib', '--edition=2024', '-Copt-level=0',
            '-Cdebuginfo=0', '-Cpanic=abort', '--emit=obj=' + str(root / 'fixture.o')]


def execute(root, timeout):
    binary = root / 'oracle'
    binary.unlink(missing_ok=True)
    link = shared.run(['cc', str(root / 'oracle.c'), str(root / 'fixture.o'),
                       '-o', str(binary)], timeout, cwd=root, env=owned.environment())
    if shared.observable(link) != dict(status=0, stdout_b64='', stderr_b64=''):
        raise AssertionError('C oracle link failed: ' + json.dumps(link))
    nonce = uuid.uuid4().hex
    return dict(nonce=nonce, link=link,
                execution=shared.run([binary, nonce], timeout, cwd=root, env=owned.environment()))


def capture(compiler, out, timeout=180):
    compiler, out = Path(compiler).resolve(), Path(out).resolve()
    out.mkdir(parents=True, exist_ok=False)
    report = dict(schema=1, verdict='incomplete', modes=[], claim='Actual lexer correctness only. No speed claim.')
    worker = None
    try:
        report['identity_before'] = shared.identity(compiler)
        for mode in ('control', '1'):
            root = out / ('control' if mode == 'control' else 'lexer')
            root.mkdir()
            (root / 'oracle.c').write_text('#include <stdio.h>\n#include <stdint.h>\n'
                'extern uint64_t lexer_a(void), lexer_b(void);\n'
                'int main(int argc,char **argv) { if(argc!=2)return 2; '
                'printf("%s:%llu:%llu\\n",argv[1],(unsigned long long)lexer_a(),'
                '(unsigned long long)lexer_b()); return 0; }\n')
            env = environment(mode)
            entry = dict(mode=mode, observations=[],
                         environment_sha256=hashlib.sha256(json.dumps(env, sort_keys=True).encode()).hexdigest(),
                         argv=arguments(root))
            report['modes'].append(entry)
            worker = LexerResident(compiler, root, mode, timeout)
            entry['pid'] = worker.process.pid
            previous = old = None
            seen = set()
            for name, data, expected, kind in cases():
                changed = validate_transition(old, data, kind, seen)
                row = dict(name=name, kind=kind, changed=changed, expected=expected,
                           sources={k: shared.encoded(v) for k, v in data.items()}, modes={})
                entry['observations'].append(row)
                for key, value in data.items():
                    (root / key).write_bytes(value)
                for variant in ('fresh', 'owned'):
                    if environment(mode) != env:
                        raise AssertionError('Environment changed')
                    (root / 'fixture.o').unlink(missing_ok=True)
                    record = row['modes'][variant] = {}
                    record['compile'] = (shared.run([compiler, *entry['argv']], timeout,
                        cwd=root, env=env) if variant == 'fresh' else worker.compile(entry['argv']))
                    if record['compile']['status'] == 0:
                        record['artifact_sha256'] = shared.digest(root / 'fixture.o')
                        record['runtime'] = execute(root, timeout)
                    if any((root / key).read_bytes() != value for key, value in data.items()):
                        raise AssertionError('Source identity changed during compile')
                expected_text = None if expected is None else '%d:%d' % expected
                owned.validate_pair(row['modes'], expected_text, worker.process.pid)
                previous = validate_lexer(row['modes']['owned']['compile'], previous, mode, kind)
                seen.update(data.values())
                old = data
            closing, worker = worker, None
            closing.close()
        report['identity_after'] = shared.identity(compiler)
        if report['identity_before'] != report['identity_after']:
            raise AssertionError('Compiler identity changed')
        report['verdict'] = 'passed'
        return report
    except BaseException as error:
        report.update(verdict='failed', error=f'{type(error).__name__}: {error}')
        raise
    finally:
        if worker is not None:
            try:
                worker.close()
            except BaseException as error:
                report['close_error'] = str(error)
        (out / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--timeout', type=float, default=180)
    args = parser.parse_args()
    capture(args.compiler, args.out, args.timeout)
    print(str(args.out / 'receipt.json'))


if __name__ == '__main__':
    main()
