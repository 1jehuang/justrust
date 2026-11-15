#!/usr/bin/env python3
"""Actual managed hosted hybrid experiment, not a guest-shell latency claim.

Runs ordinary `justrust test --remote` from fresh local pinned source copies.
Explicit hosted-only controls select an already staged, immutable guest engine.
Manual preparation, remote side-effect checks, and cleanup are outside timing.
A focused diagnostic, never a substitute for the canonical promotion suite.
"""
import argparse
import json
import math
import os
from pathlib import Path
import shlex
import shutil
import statistics
import time

import hybrid_workload as core
import hybrid_workload_pinned as pinned
import resident_engine_compare as resident


def shell_env(values):
    return 'env ' + ' '.join(shlex.quote(k + '=' + str(v)) for k, v in sorted(values.items()))


def preparation_environment(args, state):
    # Managed admission mirrors the caller's explicit Cargo home into the guest.
    # The complete CARGO_* namespace participates in resident generation identity.
    return {'JR_ENGINE_HOME': state['home'], 'JUSTRUST_NATIVE_ENGINE': args.guest_engine,
            'JUSTRUST_HYBRID_TOOLCHAIN': args.guest_toolchain, 'CARGO_BUILD_JOBS': '4',
            'CARGO_HOME': args.cargo_home}


def managed_preparation_command(command):
    # The managed session prefixes PATH before __agent, which prefixes it again
    # for the child. A normal remote shell has neither prefix. Match both layers
    # exactly rather than weakening native checkout identity for the benchmark.
    prefix = '$HOME/.cargo/bin:/opt/homebrew/bin:$HOME/.homebrew/bin:/usr/local/bin:'
    return 'PATH="' + prefix + prefix + '$PATH" ' + command


def management_request(attempt, read_only=False, pause=time.sleep):
    # Reads may safely retry after an unknown transport outcome. Never retry
    # prewarm or a build: compile-time user code may already have executed.
    limit = 3 if read_only else 1
    for index in range(limit):
        row, log = attempt(index)
        if not (row['returncode'] or row['timeout'] or row['error']):
            return row, log
        if index + 1 < limit:
            pause(index + 1)
    return row, log


def parse_side_effects(row, path):
    if row['returncode'] != 0 or row['timeout'] or row['error']:
        raise ValueError('could not read authoritative guest side effects')
    values = []
    for line in path.read_text().splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, list) and all(isinstance(x, str) for x in value):
            values.append(value)
    if len(values) != 1:
        raise ValueError('expected one authoritative guest side-effect receipt')
    return values[0]


def namespace_probe(home):
    code = """import json,os,pathlib,time
marker=b'JR_ENGINE_HOME='+os.fsencode(__HOME_LITERAL__)
def members():
    found=[]
    for path in pathlib.Path('/proc').iterdir():
        if not path.name.isdigit(): continue
        try:
            if path.stat().st_uid==os.geteuid() and marker in (path/'environ').read_bytes().split(b'\\0'):
                found.append(path.name)
        except (OSError,ValueError): pass
    return found
until=time.monotonic()+30
found=members()
while found and time.monotonic()<until:
    time.sleep(.05)
    found=members()
print(json.dumps(found))
""".replace('__HOME_LITERAL__', repr(str(home)))
    return 'python3 -c ' + shlex.quote(code)


def summarize(rows, samples):
    result = {}
    for stack in ('shipping', 'hybrid-cold', 'hybrid-warm'):
        selected = [r for r in rows if r['stack'] == stack and r['phase'] == 'edit']
        valid = len(selected) == samples and all(r['ok'] for r in selected)
        result[stack] = {'samples': len(selected), 'accepted': valid,
                         'median_seconds': statistics.median(r['wall_seconds'] for r in selected) if valid else None}
    return result


def execute(args):
    if args.output.exists() or args.source.is_relative_to(args.output) or args.output.is_relative_to(args.source):
        raise ValueError('fresh output disjoint from pristine source required')
    if args.samples < 3 or not math.isfinite(args.timeout) or args.timeout <= 0:
        raise ValueError('at least three samples and positive finite timeout required')
    resident.pinned_source(args.source)
    for tool in (args.justrust, args.shipping_justrust):
        if not tool.is_file() or not os.access(tool, os.X_OK):
            raise ValueError(f'not an executable: {tool}')
    args.output.mkdir(mode=0o700, parents=True)
    clean = {k: v for k, v in os.environ.items() if not k.startswith(
        ('JUSTRUST_', 'JR_ENGINE_', 'JR_HYBRID_', 'RA_JIT_', 'BLAZE_', 'CARGO_', 'RUST'))}
    original = (args.source / 'src/summary.rs').read_bytes()
    tools_before = {str(p): core.digest(p) for p in (args.justrust, args.shipping_justrust)}
    report = {'schema': 1, 'status': 'incomplete', 'scope': __doc__, 'pin': resident.PIN,
              'tool_sha256': tools_before, 'source_sha256': resident.source_hashes(args.source),
              'guest_inputs': {k: str(getattr(args, k)) for k in ('guest_client', 'guest_engine', 'guest_toolchain', 'guest_home', 'cargo_home')},
              'rows': [], 'management': [], 'cleanup': []}
    states = {}
    def save():
        (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    def guest(command, prefix, cwd, read_only=False):
        def attempt(index):
            suffix = '' if index == 0 else f'-retry-{index}'
            log = args.output / (prefix + suffix + '.log')
            row = core.run([str(args.justrust), 'remote', 'ssh', command], cwd, clean, log, args.timeout)
            row.update(management_read_only=read_only, management_attempt=index + 1)
            report['management'].append(row)
            save()
            return row, log
        return management_request(attempt, read_only=read_only)
    def effects(state, label):
        # The explicit remote shell owns only this benchmark's private fixture.
        code = 'import json,pathlib; p=pathlib.Path(' + repr(str(state['side'])) + '); print(json.dumps(p.read_text().splitlines() if p.exists() else []))'
        row, log = guest('python3 -c ' + shlex.quote(code), label, state['project'], read_only=True)
        return parse_side_effects(row, log)
    for stack in ('shipping', 'hybrid-cold', 'hybrid-warm'):
        root = args.output / stack
        root.mkdir()
        project = root / 'project'
        shutil.copytree(args.source, project)
        home = args.guest_home / stack
        side = root / 'side-effect'
        env = dict(clean, CARGO_BUILD_JOBS='4', CARGO_HOME=str(args.cargo_home))
        if stack != 'shipping':
            env.update(JUSTRUST_HOSTED_HYBRID='ready', JUSTRUST_HOSTED_HYBRID_ENGINE=str(args.guest_engine),
                       JUSTRUST_HOSTED_HYBRID_TOOLCHAIN=str(args.guest_toolchain), JUSTRUST_HOSTED_HYBRID_HOME=str(home))
        states[stack] = {'project': project, 'home': home, 'side': side, 'env': env}
    save()
    expected = None
    created = set()
    try:
        # Create only private experiment homes, never replace or delete an existing one.
        for stack, state in states.items():
            if stack == 'shipping':
                continue
            row, _ = guest('mkdir -m 700 -- ' + shlex.quote(str(state['home'])), 'mkdir-' + stack, state['project'])
            if row['returncode'] != 0 or row['error'] or row['timeout']:
                raise ValueError('guest home must be fresh with a pre-existing parent')
            created.add(stack)
        for phase, nonce, kind in [('prepare', 0, 'edit')] + [('edit', n + 1, 'edit') for n in range(args.samples)] + [('type-error', 101, 'type-error'), ('recovery', 102, 'edit'), ('assert-error', 103, 'test-failure'), ('assert-recovery', 104, 'edit')]:
            order = list(states) if nonce % 2 == 0 else list(reversed(states))
            for stack in order:
                state = states[stack]
                contents = pinned.edited(original, nonce, kind).replace(
                    b'std::env::var("HYBRID_SIDE_EFFECT").unwrap()',
                    json.dumps(str(state['side'])).encode())
                (state['project'] / 'src/summary.rs').write_bytes(contents)
                before = effects(state, f'{phase}-{nonce}-{stack}-before')
                tool = args.shipping_justrust if stack == 'shipping' else args.justrust
                log = args.output / f'{phase}-{nonce}-{stack}.log'
                row = core.run([str(tool), 'test', '--remote', *resident.FILTER], state['project'], state['env'], log, args.timeout)
                console = log.read_text(errors='replace')
                text, recording, recording_ok = core.full_output(tool, console, state['project'], state['env'], args.output / f'{phase}-{nonce}-{stack}-recorded.log', args.timeout)
                after = effects(state, f'{phase}-{nonce}-{stack}-after')
                adapter = {'status': row['returncode'], 'timed_out': row['timeout'], 'error': row['error']}
                good, parsed = pinned.accepted(adapter, text, after[len(before):], nonce, kind, expected)
                if stack == 'shipping' and expected is None and good:
                    expected = parsed
                want = 'justrust hybrid: resident committed' if stack == 'hybrid-warm' and phase != 'prepare' else 'justrust hybrid: Cargo fallback'
                route_ok = stack == 'shipping' or want in console
                row.update(stack=stack, phase=phase, nonce=nonce, parsed=parsed,
                           route_verified=route_ok, recording=recording, side_effect_delta=after[len(before):],
                           ok=good and recording_ok and route_ok and after[:len(before)] == before)
                report['rows'].append(row)
                save()
                if not row['ok']:
                    raise ValueError(f'actual managed hosted contract failed: {stack} {phase}')
            if phase == 'prepare':
                state = states['hybrid-warm']
                values = preparation_environment(args, state)
                command = 'cd ' + shlex.quote(str(state['project'])) + ' && ' + managed_preparation_command(shell_env(values) + ' ' + shlex.quote(str(args.guest_client)) + ' hybrid prewarm test ' + ' '.join(map(shlex.quote, resident.FILTER)))
                before = effects(state, 'prewarm-before')
                row, _ = guest(command, 'prewarm', state['project'])
                after = effects(state, 'prewarm-after')
                if row['returncode'] != 0 or row['timeout'] or row['error'] or before != after:
                    raise ValueError('manual preparation failed or executed user tests')
        report['summary'] = summarize(report['rows'], args.samples)
        report['status'] = 'passed' if all(x['accepted'] for x in report['summary'].values()) else 'failed'
    except BaseException as exc:
        report['error'] = repr(exc)
        report['status'] = 'failed'
        raise
    finally:
        for stack, state in states.items():
            if stack not in created:
                continue
            values = {'JR_ENGINE_HOME': state['home'], 'JUSTRUST_NATIVE_ENGINE': args.guest_engine,
                      'JUSTRUST_HYBRID_TOOLCHAIN': args.guest_toolchain}
            command = shell_env(values) + ' ' + shlex.quote(str(args.guest_client)) + ' hybrid stop'
            try:
                row, _ = guest(command, 'stop-' + stack, state['project'])
                verification, log = guest(namespace_probe(state['home']), 'stopped-' + stack, state['project'], read_only=True)
                alive = parse_side_effects(verification, log)
                report['cleanup'].append({'stack': stack, **row, 'remaining_namespace_pids': alive,
                                          'ok': row['returncode'] == 0 and not row['timeout'] and not row['error'] and not alive})
            except Exception as exc:
                report['cleanup'].append({'stack': stack, 'ok': False, 'error': repr(exc)})
        report['cleanup_passed'] = len(report['cleanup']) == len(created) and all(r['ok'] for r in report['cleanup'])
        report['tools_unchanged'] = {str(p): core.digest(p) for p in (args.justrust, args.shipping_justrust)} == tools_before
        report['source_unchanged'] = resident.source_hashes(args.source) == report['source_sha256']
        if not report['tools_unchanged'] or not report['source_unchanged'] or not report['cleanup_passed']:
            report['status'] = 'failed'
        save()
    print(json.dumps({'status': report['status'], 'report': str(args.output / 'report.json'), 'summary': report.get('summary')}))
    return 0 if report['status'] == 'passed' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('justrust', 'shipping-justrust', 'source', 'output', 'guest-client', 'guest-engine', 'guest-toolchain', 'guest-home'):
        parser.add_argument('--' + name, required=True, type=core.absolute)
    parser.add_argument('--cargo-home', type=core.absolute, default=Path.home() / '.cargo',
                        help='explicit caller Cargo home mirrored by managed admission, also used for guest preparation')
    parser.add_argument('--samples', type=int, default=5)
    parser.add_argument('--timeout', type=float, default=360)
    return execute(parser.parse_args())


if __name__ == '__main__':
    raise SystemExit(main())
