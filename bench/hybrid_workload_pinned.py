#!/usr/bin/env python3
"""Pinned f97fcc2 shipping/hybrid filtered-test differential, host-run only.

Fresh private copies and engine homes. Preparation and source edits are outside
measured subprocess intervals. Focused diagnostic, not a broad promotion gate.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import statistics

import hybrid_workload as core
import resident_engine_compare as resident


PIN_TREE = '76fcd81383914fa70ca2d42ed346fb4fc81d3ed8'


def source_tree(root):
    """Reconstruct Git's tree digest directly, without Git or a repository on host."""
    def object_id(kind, payload):
        return hashlib.sha1(kind + b' ' + str(len(payload)).encode() + b'\0' + payload).digest()
    def visit(directory):
        entries = []
        for path in directory.iterdir():
            if path.name == '.git':
                continue
            if path.is_symlink():
                raise ValueError(f'pinned source must not contain symlinks: {path}')
            name = os.fsencode(path.name)
            if path.is_dir():
                subtree = visit(path)
                if subtree is None:
                    continue  # Git does not track empty directories.
                entries.append((name + b'/', b'40000', name, subtree))
            elif path.is_file():
                mode = b'100755' if path.stat().st_mode & 0o111 else b'100644'
                entries.append((name, mode, name, object_id(b'blob', path.read_bytes())))
            else:
                raise ValueError(f'pinned source contains a special file: {path}')
        if not entries:
            return None
        payload = b''.join(mode + b' ' + name + b'\0' + oid
                           for _, mode, name, oid in sorted(entries))
        return object_id(b'tree', payload)
    tree = visit(root)
    return tree.hex() if tree is not None else object_id(b'tree', b'').hex()


def pinned_source(root):
    if source_tree(root) != PIN_TREE:
        raise ValueError(f'source tree must exactly match pinned {resident.PIN} ({PIN_TREE})')


def edited(original, nonce, kind='edit'):
    anchor = b'fn shares_overlapping_time() {'
    if original.count(anchor) != 1:
        raise ValueError('pinned test anchor must occur once')
    prefix = f'''\nlet path = std::env::var("HYBRID_SIDE_EFFECT").unwrap();
let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
std::io::Write::write_all(&mut file, b"nonce={nonce}\\n").unwrap();
println!("JR_RUNTIME_EXECUTED_{nonce}");
'''.encode()
    if kind == 'type-error':
        prefix = b'\nlet _: u64 = "JR_RUNTIME_TYPE_ERROR";\n'
    elif kind == 'test-failure':
        prefix += b'assert_eq!(std::hint::black_box(1u64), 2u64, "JR_RUNTIME_FAILURE");\n'
    return original.replace(anchor, anchor + prefix, 1)


def accepted(row, text, effects, nonce, kind, expected=None):
    parsed = resident.runtime.test_results(text)
    healthy = not row['timed_out'] and not row['error']
    if kind == 'type-error':
        good = (healthy and row['status'] == 101 and 'E0308' in text and
                'JR_RUNTIME_TYPE_ERROR' in text and parsed is None and not effects)
    else:
        failing = kind == 'test-failure'
        good = (healthy and row['status'] == (101 if failing else 0) and
                resident.valid_tests(parsed, expected, failing=failing) and
                effects == [f'nonce={nonce}'] and
                re.findall(r'JR_RUNTIME_EXECUTED_\d+', text) == [f'JR_RUNTIME_EXECUTED_{nonce}'])
        if failing:
            good = good and 'JR_RUNTIME_FAILURE' in text
    return bool(good), parsed


@core.cleanup_after_execute
def execute(args):
    automatic = getattr(args, 'automatic', False)
    automatic_pattern = getattr(args, 'automatic_ready_regex', None)
    if automatic and (args.prewarm or not automatic_pattern):
        raise ValueError('--automatic requires --automatic-ready-regex and cannot combine with --prewarm')
    if args.output.exists() or args.output.is_relative_to(args.source) or args.source.is_relative_to(args.output):
        raise ValueError('output must be fresh and disjoint from source')
    if args.samples < 1 or not math.isfinite(args.timeout) or args.timeout <= 0:
        raise ValueError('positive samples and finite positive timeout required')
    pinned_source(args.source)
    shipping = getattr(args, 'shipping_justrust', None) or args.justrust
    tools = (args.justrust, args.engine, shipping)
    for tool in tools:
        if not tool.is_file() or not os.access(tool, os.X_OK):
            raise ValueError(f'not executable: {tool}')
    original = (args.source / 'src/summary.rs').read_bytes()
    env = {k: v for k, v in os.environ.items() if not k.startswith(('JUSTRUST_', 'JR_ENGINE_', 'RA_JIT_', 'BLAZE_'))}
    for assignment in args.env:
        key, sep, value = assignment.partition('=')
        if not sep or not key:
            raise ValueError('--env requires KEY=VALUE')
        env[key] = value
    if args.toolchain:
        env['JUSTRUST_HYBRID_TOOLCHAIN'] = str(args.toolchain)
    args.output.mkdir(parents=True, mode=0o700)
    rows = []
    report = {'schema': 1, 'pin': resident.PIN, 'pin_tree': PIN_TREE, 'scope': __doc__, 'status': 'incomplete',
              'tool_sha256': {str(p): core.digest(p) for p in tools},
              'roles': {'shipping': str(shipping), 'hybrid': str(args.justrust), 'engine': str(args.engine)},
              'environment': core.environment_receipt(env),
              'source_sha256': resident.source_hashes(args.source), 'rows': rows}
    args._cleanup_report = report
    def save():
        (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    states = {}
    for stack in ('shipping', 'hybrid'):
        state = args.output / stack
        state.mkdir(mode=0o700)
        project = state / 'project'
        shutil.copytree(args.source, project, ignore=shutil.ignore_patterns('.git'))
        home = state / 'engine-home'
        home.mkdir(mode=0o700)
        local = dict(env, JUSTRUST_NATIVE_ENGINE=str(args.engine), JR_ENGINE_HOME=str(home),
                     CARGO_TARGET_DIR=str(state / 'target'), HYBRID_SIDE_EFFECT=str(state / 'side-effect'))
        if automatic and stack == 'hybrid':
            local['JUSTRUST_HYBRID_PREWARM'] = 'after-success'
        states[stack] = (project, local)
        args._cleanup_states.append({'client': args.justrust, 'cwd': project, 'env': local,
                                     'log': args.output / ('cleanup-' + stack + '.log'), 'stack': stack})
    expected = None
    phases = [('cold', 0, 'edit'), ('prewarm', 0, 'prewarm'), ('warm', 0, 'edit'),
              ('type-error', 1, 'type-error'), ('type-recovery', 2, 'edit'),
              ('assert-error', 3, 'test-failure'), ('assert-recovery', 4, 'edit')]
    phases += [('edit', rep + 5, 'edit') for rep in range(args.samples)]
    save()
    try:
        for index, (phase, nonce, kind) in enumerate(phases):
            pair = []
            for stack in (('shipping', 'hybrid') if index % 2 == 0 else ('hybrid', 'shipping')):
                if kind == 'prewarm' and (stack == 'shipping' or not args.prewarm):
                    continue
                project, local = states[stack]
                target = project / 'src/summary.rs'
                contents = edited(original, nonce, 'edit' if kind == 'prewarm' else kind)
                if target.read_bytes() != contents:
                    target.write_bytes(contents)
                side = Path(local['HYBRID_SIDE_EFFECT'])
                before = core.side_lines(side)
                command = [str(args.justrust if stack == 'hybrid' else shipping)] + (['hybrid'] if stack == 'hybrid' else [])
                command += (['prewarm'] if kind == 'prewarm' else []) + ['test'] + resident.FILTER
                prefix = args.output / f'{index:02d}-{phase}-{stack}'
                memory_before = core.namespace_pss(local['JR_ENGINE_HOME'])
                offset = core.background_offset(local['JR_ENGINE_HOME'])
                row = resident.runtime.run_process(command, project, local, prefix, args.timeout)
                row.update(namespace_pss_before=memory_before,
                           namespace_pss_after=core.namespace_pss(local['JR_ENGINE_HOME']))
                text = '\n'.join(prefix.with_suffix('.' + suffix).read_text(errors='replace') for suffix in ('stdout', 'stderr'))
                semantic_text, receipt, recovered = core.full_output(
                    command[0], text, project, local,
                    args.output / f'{index:02d}-{phase}-{stack}-recorded.log', args.timeout)
                row['full_output'] = receipt
                after = core.side_lines(side)
                effects = after[len(before):]
                if kind == 'prewarm':
                    good, parsed = row['status'] == 0 and not row['timed_out'] and not row['error'] and after == before, None
                else:
                    good, parsed = accepted(row, semantic_text, effects, nonce, kind, expected)
                    if expected is None and stack == 'shipping' and good:
                        expected = parsed
                route = args.cold_route_regex if phase == 'cold' else args.warm_route_regex if phase in ('warm', 'edit') else None
                route_verified = bool(route and re.search(route, text)) if stack == 'hybrid' and kind != 'prewarm' else None
                if stack == 'hybrid' and route:
                    good = good and route_verified
                if automatic and stack == 'hybrid' and row['status'] == 0:
                    queued = 'justrust hybrid: queued background preparation' in text
                    row['automatic_preparation_queued'] = queued
                    if queued:
                        row['automatic_preparation'] = core.wait_background(
                            local['JR_ENGINE_HOME'], offset, side, after, args.timeout,
                            automatic_pattern, args.output / f'{index:02d}-{phase}-background.log')
                        good = good and row['automatic_preparation']['ok']
                        row['namespace_pss_prepared'] = core.namespace_pss(local['JR_ENGINE_HOME'])
                    elif phase == 'cold':
                        good = False
                good = good and recovered
                row.update(phase=phase, nonce=nonce, stack=stack, parsed=parsed,
                           environment=core.environment_receipt(local),
                           side_effect_delta=effects, route_verified=route_verified,
                           source_sha256=hashlib.sha256((project / 'src/summary.rs').read_bytes()).hexdigest(),
                           ok=good and after[:len(before)] == before,
                           raw_prefix=str(prefix), stdout_sha256=core.digest(prefix.with_suffix('.stdout')),
                           stderr_sha256=core.digest(prefix.with_suffix('.stderr')))
                rows.append(row)
                pair.append(row)
                save()
            if len(pair) == 2:
                equal = all(pair[0][key] == pair[1][key] for key in ('status', 'parsed', 'side_effect_delta'))
                for row in pair:
                    row.update(differential_equal=equal, ok=row['ok'] and equal)
                save()
        report['status'] = 'passed' if all(row['ok'] for row in rows) else 'failed'
        report['routing_evidence_complete'] = bool((args.prewarm or automatic) and args.cold_route_regex and args.warm_route_regex and all(row['route_verified'] for row in rows if row['stack'] == 'hybrid' and row['phase'] in ('cold', 'warm', 'edit')))
        report['focused_edit_medians_seconds'] = {stack: statistics.median(row['elapsed_seconds'] for row in rows if row['stack'] == stack and row['phase'] == 'edit') for stack in ('shipping', 'hybrid')} if report['status'] == 'passed' else {}
        report['source_unchanged'] = resident.source_hashes(args.source) == report['source_sha256']
        report['tools_after'] = {str(p): core.digest(p) for p in tools}
        report['tools_unchanged'] = report['tools_after'] == report['tool_sha256']
        if not report['source_unchanged'] or not report['tools_unchanged']:
            report['status'] = 'failed'
            report['focused_edit_medians_seconds'] = {}
    except BaseException as exc:
        report['error'] = repr(exc)
        raise
    finally:
        save()
    return 0 if report['status'] == 'passed' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('justrust', 'engine', 'source', 'output'):
        parser.add_argument('--' + name, required=True, type=core.absolute)
    parser.add_argument('--toolchain', type=core.absolute)
    parser.add_argument('--shipping-justrust', type=core.absolute)
    parser.add_argument('--samples', type=int, default=5)
    parser.add_argument('--timeout', type=float, default=600)
    parser.add_argument('--env', action='append', default=[])
    parser.add_argument('--prewarm', action='store_true')
    parser.add_argument('--automatic', action='store_true', help='after-success preparation with untimed readiness wait')
    parser.add_argument('--automatic-ready-regex', help='required NEW successful preparation receipt in prewarm.log')
    parser.add_argument('--cold-route-regex')
    parser.add_argument('--warm-route-regex')
    raise SystemExit(execute(parser.parse_args()))


if __name__ == '__main__':
    main()
