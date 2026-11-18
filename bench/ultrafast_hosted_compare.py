#!/usr/bin/env python3
"""Host-only pinned Cargo versus Blaze-derived jr-engine diagnostic.

No Rust commands run on import. Requires an explicit execution acknowledgement.
Optional --upstream adds the official stable Blaze ELF as a third matched arm.
Cold means fresh private compiler/target stores, NOT a flushed OS page cache.
Results are descriptive for this fixture, not a general speed or compatibility claim.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import uuid

import resident_engine_compare as shared
import resident_tier_compare as tier

runtime = shared.runtime
STACKS = ['stock', 'engine']
UPSTREAM_VERSION = 'blaze 0.20261027.0 (49ca152)'
PHASES = ('cold', 'warm-unchanged', 'edit', 'type-error', 'type-recovery',
          'assert-failure', 'assert-recovery')


def marker_edit(original, nonce):
    # Reuse the existing reachable-body edit, including black_box and assertion.
    return runtime.edit_bytes(original, replicate=nonce - 1)


def success(row, text, marker, full=False):
    parsed = runtime.test_results(text)
    contract = (parsed is not None and parsed['passed'] == 147 and
                parsed['failed'] == 0 and parsed['ignored'] == 1 and
                parsed['filtered'] == 0 and len(parsed['tests']) == 148) if full else (
                    shared.valid_tests(parsed) and parsed['filtered'] == 144)
    return (runtime.base.succeeded(row) and contract and
            re.findall(r'JR_RUNTIME_EXECUTED_\d+', text) == [marker])


def accepted(phase, row, text, marker):
    if phase == 'type-error':
        return (row['status'] == 101 and not row['timed_out'] and not row['error']
                and 'E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text
                and runtime.test_results(text) is None
                and not re.search(r'JR_RUNTIME_EXECUTED_\d+', text))
    if phase == 'assert-failure':
        parsed = runtime.test_results(text)
        return (row['status'] == 101 and not row['timed_out'] and not row['error']
                and shared.valid_tests(parsed, failing=True) and parsed['filtered'] == 144
                and 'JR_RUNTIME_FAILURE' in text
                and re.findall(r'JR_RUNTIME_EXECUTED_\d+', text) == [marker])
    return success(row, text, marker, full=phase == 'full-library')


def phase_nonce(nonce, phase):
    return nonce + {"cold": 0, "warm-unchanged": 0, "edit": 1, "type-error": 2,
                    "type-recovery": 3, "assert-failure": 4, "assert-recovery": 5,
                    "full-library": 6}[phase]


def inputs(original, nonce):
    first, edited = marker_edit(original, nonce), marker_edit(original, nonce + 1)
    # Insert the failure after the runtime marker so failed-test execution is proven.
    failure = marker_edit(original, nonce + 4).replace(
                            b'println!("JR_RUNTIME_EXECUTED_' + str(nonce + 4).encode() + b'");',
                            b'println!("JR_RUNTIME_EXECUTED_' + str(nonce + 4).encode() + b'");'
                            b'assert_eq!(std::hint::black_box(1u64), 2u64, "JR_RUNTIME_FAILURE");', 1)
    return {'cold': first, 'warm-unchanged': first, 'edit': edited,
            'type-error': runtime.edit_bytes(original, kind='type-error'),
            'type-recovery': marker_edit(original, nonce + 3), 'assert-failure': failure,
            'assert-recovery': marker_edit(original, nonce + 5),
            'full-library': marker_edit(original, nonce + 6)}


def identities(args):
    result = {'engine': {'path': str(args.engine), 'sha256': runtime.base.digest(args.engine)},
              'toolchain': shared.toolchain_identity(args.toolchain)}
    if getattr(args, 'upstream', None):
        result['upstream'] = {'path': str(args.upstream),
                              'sha256': runtime.base.digest(args.upstream),
                              'required_version': UPSTREAM_VERSION}
    return result


def tool_for(stack, args):
    return (args.engine if stack == 'engine' else args.upstream if stack == 'upstream'
            else args.toolchain / 'bin/cargo')


def test_command(stack, args, arguments):
    return shared.command(tool_for(stack, args),
                          'reference' if stack == 'upstream' else stack,
                          arguments=arguments)


def environment(root, stack, args):
    env = shared.environment(root, stack, args.toolchain, None)
    # No rustup selection, compiler wrappers, remote downloads of compiler artifacts,
    # automatic publishing, or inherited user Cargo configuration.
    env.update(RUSTC=str(args.toolchain / 'bin/rustc'),
               RUSTDOC=str(args.toolchain / 'bin/rustdoc'),
               RUSTUP_TOOLCHAIN=str(args.toolchain),
               PATH=str(args.toolchain / 'bin') + os.pathsep + env.get('PATH', ''),
               RA_JIT_REMOTE_PUSH_EVERY='off', BLAZE_REMOTE_PUSH_EVERY='off',
               RA_JIT_MEMORY_MAX='off', RA_JIT_SPARE='0', RA_JIT_IDLE_SECS='3600')
    return env


def execute(args):
    if not args.execute_host_workloads:
        raise ValueError('explicit --execute-host-workloads acknowledgement required')
    if args.samples < 5 or args.timeout <= 0:
        raise ValueError('at least five samples and positive timeout required')
    if (args.output.exists() or args.output.is_relative_to(args.source)
            or args.source.is_relative_to(args.output) or len(os.fsencode(args.output)) > 50):
        raise ValueError('output must be fresh, disjoint from source, and <=50 path bytes')
    shared.pinned_source(args.source)
    stacks = STACKS + (['upstream'] if getattr(args, 'upstream', None) else [])
    for name in ('engine', 'upstream'):
        tool = getattr(args, name, None)
        if tool is None:
            continue
        with tool.open('rb') as stream:
            if not tool.is_absolute() or stream.read(4) != b'\x7fELF' or not os.access(tool, os.X_OK):
                raise ValueError(name + ' must be an absolute executable ELF, not a wrapper')
    identity_before = identities(args)  # Read-only hashes, never install a toolchain.
    args.output.mkdir(mode=0o700, parents=True)
    original = (args.source / 'src/summary.rs').read_bytes()
    before = shared.source_hashes(args.source)
    rows, copies, envs, full_results = [], {}, {}, {}
    metadata = dict(schema=1, status='incomplete', pin=shared.PIN, samples=args.samples,
                    host_label=args.host_label, hostname=platform.node(),
                    cpuinfo=Path('/proc/cpuinfo').read_text(),
                    meminfo=Path('/proc/meminfo').read_text(), source_before=before,
                    identities_before=identity_before, environments={}, stacks=stacks,
                    scope='Host-only pinned filtered tests. No universal speed claim.',
                    cold_policy='Fresh private target and engine stores per replicate; OS page cache not flushed.',
                    full_library_policy='required' if not args.skip_full_library else 'explicitly skipped; not correctness-certified',
                    proc_macro_policy='Both server variables 0, rustc-built compatibility path.',
                    memory_policy='Post-command retained engine process /proc snapshot, not peak or stock memory parity.')

    def save():
        (args.output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
        (args.output / 'records.json').write_text(json.dumps(rows, indent=2) + '\n')

    def invoke(stack, rep, phase, cmd, env, source):
        prefix = args.output / f'{rep}-{stack}-{phase}'
        row = runtime.run_process(cmd, source, env, prefix, args.timeout)
        text = '\n'.join(prefix.with_suffix('.' + s).read_text(errors='replace')
                         for s in ('stdout', 'stderr'))
        row.update(stack=stack, replicate=rep, phase=phase,
                   source_sha256=runtime.base.digest(source / 'src/summary.rs'),
                   parsed=runtime.test_results(text), raw={s: {
                       'file': prefix.with_suffix('.' + s).name,
                       'sha256': runtime.base.digest(prefix.with_suffix('.' + s))}
                       for s in ('stdout', 'stderr')})
        rows.append(row)
        save()
        return row, text

    def observe(rep, env, source):
        row, text = invoke('engine', rep, 'status-' + str(len(rows)),
                           [str(args.engine), 'status'], env, source)
        status = tier.parse_status(text)
        row['native_status'] = status
        row['memory'] = tier.memory_snapshot(([status['daemon']] if status['daemon'] else []) +
                                              status['workers'])
        save()

    try:
        save()
        # Version-only queries are retained separately, outside timed phases.
        for rep in range(args.samples):
            nonce = (uuid.uuid4().int & ((1 << 63) - 1)) + 1
            contents = inputs(original, nonce)
            for stack in runtime.order(stacks, rep):
                root = args.output / str(rep) / stack[0]
                root.mkdir(mode=0o700, parents=True)
                source = root / 'src'
                shutil.copytree(args.source, source, ignore=shutil.ignore_patterns('.git'), symlinks=True)
                backup = root / 'summary.original'
                with backup.open('xb') as stream:
                    stream.write(original)
                    stream.flush()
                    os.fsync(stream.fileno())
                copies[(rep, stack)] = source
                if shared.source_hashes(source) != before:
                    raise ValueError('source changed during private copy')
                env = environment(root, stack, args)
                envs[(rep, stack)] = env
                metadata['environments'][f'{rep}-{stack}'] = {
                    k: v for k, v in env.items() if k.startswith(
                        ('BLAZE_', 'RA_JIT_', 'JR_ENGINE_', 'CARGO_', 'RUST', 'XDG_')) or k == 'PATH'}
                tool = tool_for(stack, args)
                row, text = invoke(stack, rep, 'version', [str(tool), '--version'], env, source)
                row['version_text'] = text
                row['accepted'] = runtime.base.succeeded(row) and bool(re.search(
                    r'^jr-engine experimental .*build resident-v1-[0-9a-f]{64}\)' if stack == 'engine'
                    else '^' + re.escape(UPSTREAM_VERSION) + r'$' if stack == 'upstream'
                    else r'^cargo ', text, re.M))
                if stack == 'stock':
                    rust, version = invoke(stack, rep, 'rustc-version',
                                           [str(args.toolchain / 'bin/rustc'), '-vV'], env, source)
                    rust['accepted'] = runtime.base.succeeded(rust) and runtime.nightly_version(version)
            # Phase-major rotation gives matched input bytes and alternates the first stack.
            for phase in PHASES + (() if args.skip_full_library or rep != 0 else ('full-library',)):
                for stack in runtime.order(stacks, rep):
                    source, env = copies[(rep, stack)], envs[(rep, stack)]
                    (source / 'src/summary.rs').write_bytes(contents[phase])
                    arguments = (['--lib', '--', '--test-threads=1', '--nocapture']
                                 if phase == 'full-library' else shared.FILTER)
                    row, text = invoke(stack, rep, phase, test_command(stack, args, arguments), env, source)
                    marker = f'JR_RUNTIME_EXECUTED_{phase_nonce(nonce, phase)}'
                    row['expected_marker'] = marker
                    row['accepted'] = accepted(phase, row, text, marker)
                    if phase == 'full-library':
                        full_results[stack] = row['parsed']
                    save()
                    if stack == 'engine':
                        observe(rep, env, source)
            # Stop only the namespace created by this replicate, never global servers.
            for stack in stacks:
                if stack == 'stock':
                    continue
                row, text = invoke(stack, rep, 'version-after',
                                 [str(tool_for(stack, args)), '--version'],
                                 envs[(rep, stack)], copies[(rep, stack)])
                initial = next(r for r in rows if r['stack'] == stack and
                               r['replicate'] == rep and r['phase'] == 'version')
                row['version_text'] = text
                row['accepted'] = runtime.base.succeeded(row) and text == initial['version_text']
                row, _ = invoke(stack, rep, 'stop', [str(tool_for(stack, args)), 'stop'],
                                 envs[(rep, stack)], copies[(rep, stack)])
                row['accepted'] = runtime.base.succeeded(row)
                save()
    except BaseException as exc:
        metadata['error'] = repr(exc)
        raise
    finally:
        cleanup_errors = []
        for (rep, stack), source in copies.items():
            if stack != 'stock' and (rep, stack) in envs:
                try:
                    stop, _ = invoke(stack, rep, 'cleanup', [str(tool_for(stack, args)), 'stop'],
                                     envs[(rep, stack)], source)
                    stop['accepted'] = runtime.base.succeeded(stop)
                    if not stop['accepted']:
                        cleanup_errors.append(f'{rep}: {stack} stop failed')
                except Exception as exc:
                    cleanup_errors.append(repr(exc))
            try:
                (source / 'src/summary.rs').write_bytes((source.parent / 'summary.original').read_bytes())
            except OSError as exc:
                cleanup_errors.append(f'{rep}-{stack}: source restoration failed: {exc!r}')
        metadata['cleanup_errors'] = cleanup_errors
        metadata['restored_copies'] = {}
        try:
            metadata['restored_copies'] = {f'{r}-{s}': shared.source_hashes(p) == before
                                          for (r, s), p in copies.items()}
            metadata['source_after'] = shared.source_hashes(args.source)
            metadata['identities_after'] = identities(args)
            metadata['unchanged'] = (before == metadata['source_after'] and
                identity_before == metadata['identities_after'] and
                all(metadata['restored_copies'].values()))
        except (OSError, ValueError) as exc:
            metadata.update(identity_error=repr(exc), unchanged=False)
        gate = (not args.skip_full_library and len(full_results) == len(stacks) and
                full_results['stock'] is not None and
                all(full_results['stock'] == full_results[s] for s in stacks) and
                all(r.get('accepted') for r in rows if r['phase'] == 'full-library'))
        valid = (metadata['unchanged'] and not cleanup_errors and 'error' not in metadata
                 and all(r.get('accepted', False) for r in rows if not r['phase'].startswith('status-'))
                 and len([r for r in rows if r['phase'] in PHASES]) == args.samples * len(stacks) * len(PHASES))
        metadata['full_library_parity'] = gate
        metadata['status'] = 'passed' if valid and gate else 'failed/incomplete'
        medians = {}
        if valid and gate:
            for stack in stacks:
                medians[stack] = {phase: statistics.median(
                    r['elapsed_seconds'] for r in rows if r['stack'] == stack and r['phase'] == phase)
                    for phase in PHASES}
        save()
        (args.output / 'summary.json').write_text(json.dumps(dict(
            status=metadata['status'], medians_seconds=medians,
            policy='Only all-gates-passed results summarized. Descriptive scoped host-only comparison, not general speedup proof.'), indent=2) + '\n')
    return 0 if metadata['status'] == 'passed' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('engine', 'source', 'output', 'toolchain'):
        parser.add_argument('--' + name, required=True, type=shared.absolute)
    parser.add_argument('--upstream', type=shared.absolute,
                        help='Optional official stable Blaze 0.20261027.0 absolute ELF path')
    parser.add_argument('--host-label', required=True, help='Dedicated host identity/operator label')
    parser.add_argument('--execute-host-workloads', action='store_true')
    parser.add_argument('--samples', type=int, default=5)
    parser.add_argument('--timeout', type=float, default=360)
    parser.add_argument('--skip-full-library', action='store_true',
                        help='Retain diagnostic rows but never certify or summarize speed')
    raise SystemExit(execute(parser.parse_args()))


if __name__ == '__main__':
    main()
