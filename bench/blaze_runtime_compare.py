#!/usr/bin/env python3
"""Private scoped filtered-test runtime diagnostic. Output contains private paths.

No native workloads are run on import. Installed Blaze toolchains are linked for
read-only use by intent, never installed or enabled by this harness.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import signal
import statistics
import subprocess
import threading
import time

import blaze_edit_compare as base

WILD_VERSION = '0.10.0'  # Shipping client's pinned src/linker.rs layout.

PROFILES = {f'CARGO_PROFILE_{p}_{k}': v for p in ('DEV', 'TEST')
            for k, v in (('OPT_LEVEL', '0'), ('DEBUG', '0'), ('INCREMENTAL', 'true'))}
ALLOW = base.ENV_LOG_ALLOWLIST | frozenset(PROFILES) | {'BLAZE_BACKEND', 'BLAZE_OPT', 'JUSTRUST_HOTPATCH'}


def logged_environment(env):
    return {k: env[k] for k in sorted(ALLOW) if k in env}


def edit_bytes(original, replicate=0, anchor=b'fn shares_overlapping_time() {', kind='edit'):
    if original.count(anchor) != 1:
        raise ValueError('edit anchor must occur exactly once')
    n = replicate + 1
    body = (f'let observed = std::hint::black_box({n}u64); '
            f'assert_eq!(observed, {n}u64); '
            f'println!("JR_RUNTIME_EXECUTED_{n}");').encode()
    if kind == 'type-error':
        body = b'let _: u64 = "JR_RUNTIME_TYPE_ERROR";'
    elif kind == 'test-failure':
        body = b'assert_eq!(std::hint::black_box(1u64), 2u64, "JR_RUNTIME_FAILURE");'
    return original.replace(anchor, anchor + b'\n' + body, 1)


def order(stacks, replicate):
    pivot = replicate % len(stacks)
    return stacks[pivot:] + stacks[:pivot]


def run_process(command, cwd, env, prefix, timeout):
    """Blocking wait, with no subprocess timeout polling quantization."""
    before, child_before = base.cpu_snapshot(), base.child_cpu()
    start = time.perf_counter()
    status, error = None, None
    expired = threading.Event()
    with prefix.with_suffix('.stdout').open('xb') as out, prefix.with_suffix('.stderr').open('xb') as err:
        try:
            process = subprocess.Popen(command, cwd=cwd, env=env, stdout=out, stderr=err,
                                       start_new_session=True)
            def kill():
                expired.set()
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            timer = threading.Timer(timeout, kill)
            timer.start()
            try:
                status = process.wait()
            except BaseException:
                kill()
                process.wait()
                raise
            finally:
                timer.cancel()
                timer.join()
        except OSError as exc:
            error = repr(exc)
    elapsed = time.perf_counter() - start
    after, child_after = base.cpu_snapshot(), base.child_cpu()
    noise = {'before': before, 'after': after,
             'child_cpu_seconds': child_after - child_before,
             'contamination': 'unknown',
             'reason': 'Resident daemon CPU is not waited child CPU. This proxy does not establish a quiet machine.'}
    if before and after:
        noise['other_cpu_seconds_estimate'] = max(0, after['busy_seconds'] - before['busy_seconds'] - (child_after - child_before))
    return dict(command=list(map(str, command)), status=status, error=error,
                timed_out=expired.is_set(), elapsed_seconds=elapsed, noise=noise,
                environment=logged_environment(env))


def identities(args):
    result = dict(base.identities(args))
    if args.wild:
        result['wild'] = {'path': str(args.wild), 'sha256': base.digest(args.wild)}
    return result


def install_wild(source, env):
    source = Path(source)
    if not source.is_file() or not os.access(source, os.X_OK):
        raise ValueError('--wild must name an executable file')
    destination = Path(env['JUSTRUST_HOME']) / 'toolchains/linkers' / f'wild-{WILD_VERSION}' / 'wild'
    destination.parent.mkdir(mode=0o700, parents=True, exist_ok=False)
    shutil.copy2(source, destination)
    expected = base.digest(source)
    if base.digest(destination) != expected:
        raise ValueError('wild source changed while copying')
    return {'source': str(source), 'path': str(destination), 'sha256': expected,
            'version_layout': WILD_VERSION}


def retained_hotpatch(root, prefix):
    journal = root / 'hotpatch.jsonl'
    if not journal.exists():
        return {'present': False, 'outcomes': []}
    raw = journal.read_bytes()
    artifact = prefix.with_suffix('.hotpatch.jsonl')
    with artifact.open('xb') as stream:
        stream.write(raw)
    entries = [json.loads(line) for line in raw.decode('utf-8').splitlines()]
    if not entries or any(not isinstance(e, dict) or not isinstance(e.get('outcome'), str)
                          for e in entries):
        raise ValueError('invalid hotpatch journal')
    return {'present': True, 'artifact': artifact.name,
            'sha256': hashlib.sha256(raw).hexdigest(),
            'outcomes': [e['outcome'] for e in entries]}


def recorded_runs(env):
    """Snapshot only this stack's private recording home, never global last."""
    root = Path(env['JUSTRUST_HOME']) / 'runs'
    return {p.name for p in root.iterdir() if p.is_dir()} if root.is_dir() else set()


def recorded_output(env, before, prefix):
    """Retrieve fresh unabridged output after timing, with strict provenance.

    Console rendering is intentionally lossy. Never append it to saved output:
    doing so duplicates runtime markers and can conceal missing raw capture.
    """
    fresh = recorded_runs(env) - before
    if len(fresh) != 1:
        raise ValueError(f'expected one fresh recorded run, found {len(fresh)}')
    run_id = fresh.pop()
    root = Path(env['JUSTRUST_HOME']) / 'runs' / run_id
    raw = (root / 'output.jsonl').read_bytes()
    artifact = prefix.with_suffix('.raw-output.jsonl')
    with artifact.open('xb') as stream:
        stream.write(raw)
    lines = []
    for line in raw.decode('utf-8').splitlines():
        entry = json.loads(line)
        if (not isinstance(entry, dict) or not isinstance(entry.get('l'), str)
                or entry.get('s') not in ('o', 'e')):
            raise ValueError('invalid recorded output line schema')
        lines.append(entry['l'])
    if not lines:
        raise ValueError('empty recorded output')
    return '\n'.join(lines) + '\n', {'run_id': run_id,
        'artifact': artifact.name, 'sha256': hashlib.sha256(raw).hexdigest(),
        'hotpatch': retained_hotpatch(root, prefix)}


def test_results(output):
    names = re.findall(r'^test ([^\s]+) \.\.\. (?:(?:[^\n]*\n)*?)(ok|FAILED|ignored)(?: <[0-9]+(?:\.[0-9]+)?s>)?[ \t]*$', output, re.M)
    totals = re.findall(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out', output)
    if len(totals) != 1 or not names or len(dict(names)) != len(names):
        return None
    state, passed, failed, ignored, measured, filtered = totals[0]
    counts = [int(passed), int(failed), int(ignored)]
    if counts != [sum(s == x for _, s in names) for x in ('ok', 'FAILED', 'ignored')]:
        return None
    if int(measured) != 0 or state != ('FAILED' if int(failed) else 'ok'):
        return None
    return dict(tests=sorted(names), passed=int(passed), failed=int(failed),
                ignored=int(ignored), filtered=int(filtered))


def accepted(row, output, expected=None, marker=None):
    parsed = test_results(output)
    return (base.succeeded(row) and parsed is not None and parsed['failed'] == 0
            and (expected is None or parsed == expected)
            and (marker is None or re.findall(r'JR_RUNTIME_EXECUTED_\d+', output) == [marker]))


def environment(args, stack):
    env = base.environment(args, 'owned-on' if stack == 'all3on' else
                           'blaze' if stack == 'blaze-llvm' else stack)
    # base uses stack-named isolation. Move LLVM's base directory separately by
    # supplying a nested output root rather than sharing the Blaze target.
    env.update(PROFILES)
    if stack in ('all4on', 'all4hotpatch'):
        env['RUSTFLAGS'] = ' '.join(f'-Z{flag}=yes' for flag in
                                  (*base.FLAGS, 'jr-cache-dead-code-items'))
    if stack == 'all4hotpatch':
        env['JUSTRUST_HOTPATCH'] = '1'
    if stack.startswith('blaze'):
        env.update(BLAZE_BACKEND='llvm' if stack == 'blaze-llvm' else 'cranelift',
                   BLAZE_OPT='0')
    return env


def seed_manifest(root):
    """Reject links and special files, including a linked source root."""
    root = Path(root)
    if any(p.is_symlink() for p in (root, *root.parents)) or not root.is_dir():
        raise ValueError('seed cache must be a real directory, not a symlink')
    manifest = {}
    for current, directories, files in os.walk(root, followlinks=False):
        for name in directories + files:
            path = Path(current) / name
            mode = path.lstat().st_mode
            if stat.S_ISLNK(mode) or not (stat.S_ISREG(mode) or stat.S_ISDIR(mode)):
                raise ValueError(f'seed cache refuses symlink or special file: {path}')
            if stat.S_ISREG(mode):
                manifest[str(path.relative_to(root))] = base.digest(path)
    return dict(sorted(manifest.items()))


def seed_cache(source, destination):
    """Copy only into a new private store, validating content before use.

    Copies preserve symlinks rather than following them if a concurrent writer
    introduces one. Post-copy validation then rejects that store before startup.
    Source mutations likewise fail closed. Never alter or delete the source.
    """
    source, destination = Path(source), Path(destination)
    src, dst = source.resolve(), destination.resolve()
    if src == dst or src.is_relative_to(dst) or dst.is_relative_to(src):
        raise ValueError('seed source and destination must be disjoint')
    if any(p.is_symlink() for p in destination.parents):
        raise ValueError('seed destination parents must not be symlinks')
    if destination.exists() or destination.is_symlink():
        raise ValueError('seed destination must be new')
    before = seed_manifest(source)
    shutil.copytree(source, destination, symlinks=True)
    copied, after = seed_manifest(destination), seed_manifest(source)
    if before != copied or before != after:
        raise ValueError('seed cache changed while copying')
    return {'source': str(src), 'destination': str(dst), 'files': before,
            'scope': 'warm-store seeded compatibility diagnostic, not cold-store performance'}


def source_hashes(source, output):
    return {str(p.relative_to(source)): base.digest(p) for p in sorted(source.rglob('*'))
            if p.is_file() and not p.is_symlink()
            and not set(p.relative_to(source).parts) & {'.git', 'target', '.wrangler'}
            and not p.is_relative_to(output)}


def execute(args):
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    pinned = args.source / args.edit_file
    original = pinned.read_bytes()
    backup = args.output / 'source.original'
    with backup.open('xb') as stream:
        stream.write(original)
        stream.flush()
        os.fsync(stream.fileno())
    directory_fd = os.open(args.output, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(directory_fd)
    finally:
        os.close(directory_fd)
    anchor = args.edit_anchor.encode()
    edit_bytes(original, anchor=anchor)
    stacks = ['owned-off', 'all3on', 'blaze'] + (['blaze-llvm'] if args.blaze_llvm else [])
    if args.owned_experiments:
        stacks += ['all4on', 'all4hotpatch']
    records, medians = [], {}
    metadata = {'scope': 'scoped filtered tests, not full suite or general speed claim',
                'source_before': source_hashes(args.source, args.output),
                'identities_before': identities(args), 'profiles': PROFILES,
                'samples': args.samples, 'stacks': stacks,
                'store_scope': 'warm-store seeded' if args.blaze_seed_cache else 'unseeded private stores',
                'blaze_seed_manifests': {}, 'wild_copies': {},
                'seed_source_before': seed_manifest(args.blaze_seed_cache) if args.blaze_seed_cache else None}
    expected = None
    def save():
        (args.output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
        (args.output / 'records.json').write_text(json.dumps(records, indent=2) + '\n')
    def invoke(stack, phase, rep, env, contents):
        if args.command == 'build' and phase == 'type-error':
            contents = original + b'\nconst _: u64 = "JR_RUNTIME_TYPE_ERROR";\n'
        pinned.write_bytes(contents)
        prefix = args.output / f'{phase}-{rep:03d}-{stack}'
        cmd = [str(args.blaze if stack.startswith('blaze') else args.justrust), args.command]
        cmd += (['--lib', '--', args.test_filter, '--test-threads=1', '--nocapture']
                if args.command == 'test' else ['--bin', args.binary])
        owned = not stack.startswith('blaze')
        before_runs = recorded_runs(env) if owned else None
        row = run_process(cmd, args.source, env, prefix, args.timeout)
        if owned:
            try:
                output, provenance = recorded_output(env, before_runs, prefix)
                row['raw_output'] = provenance
                if (stack == 'all4hotpatch' and phase in ('prepare', 'edit')
                        and base.succeeded(row) and not provenance['hotpatch']['present']):
                    row['error'] = 'hotpatch experiment produced no linker journal'
            except (OSError, ValueError, UnicodeError) as exc:
                output = ''
                row['raw_output_error'] = repr(exc)
                # Successful compilation without verifiable runtime output is
                # not a successful runtime benchmark, including build mode.
                row['error'] = row.get('error') or 'raw output retrieval failed'
        else:
            output = ''.join(prefix.with_suffix('.' + s).read_text(errors='replace') for s in ('stdout', 'stderr'))
            row['raw_output'] = {'source': 'direct Blaze stdout/stderr'}
        row.update(stack=stack, phase=phase, replicate=rep,
                   source_sha256=hashlib.sha256(contents).hexdigest(), parsed=test_results(output))
        if args.command == 'build' and base.succeeded(row):
            binary = Path(env['CARGO_TARGET_DIR']) / 'debug' / args.binary
            execution = run_process([str(binary), '--version'], args.source, env,
                                    args.output / f'{phase}-{rep:03d}-{stack}-version', args.timeout)
            row['binary_execution'] = execution
            row['runtime_ok'] = base.succeeded(execution)
        records.append(row)
        save()
        return row, output
    def good(row, output, marker=None):
        if args.command == 'build':
            return base.succeeded(row) and row.get('runtime_ok', False)
        return accepted(row, output, expected, marker)
    try:
        save()
        active, environments = [], {}
        for stack in stacks:
            if stack == 'blaze-llvm':
                nested = argparse.Namespace(**vars(args))
                nested.output = args.output / 'llvm'
                env = environment(nested, stack)
            else:
                env = environment(args, stack)
            environments[stack] = env
            if not stack.startswith('blaze') and args.wild:
                metadata['wild_copies'][stack] = install_wild(args.wild, env)
                if metadata['wild_copies'][stack]['sha256'] != metadata['identities_before']['wild']['sha256']:
                    raise ValueError('wild identity changed between stacks')
                save()
            if stack.startswith('blaze') and args.blaze_seed_cache:
                metadata['blaze_seed_manifests'][stack] = seed_cache(
                    args.blaze_seed_cache, Path(env['BLAZE_CACHE']))
                if metadata['blaze_seed_manifests'][stack]['files'] != metadata['seed_source_before']:
                    raise ValueError('seed source changed between stacks')
                save()
            row, output = invoke(stack, 'prepare', 0, env, original)
            if not good(row, output):
                continue
            if expected is None and args.command == 'test':
                expected = row['parsed']
                metadata['expected_tests'] = expected
            row, output = invoke(stack, 'type-error', 0, env,
                                 edit_bytes(original, anchor=anchor, kind='type-error'))
            type_ok = (row['status'] == 101 and not row['timed_out'] and not row['error']
                       and 'E0308' in output and 'JR_RUNTIME_TYPE_ERROR' in output)
            row['gate_passed'] = type_ok
            row, output = invoke(stack, 'type-recovery', 0, env, original)
            if not type_ok or not good(row, output):
                continue
            if args.command == 'test':
                row, output = invoke(stack, 'test-failure', 0, env,
                                     edit_bytes(original, anchor=anchor, kind='test-failure'))
                parsed = row['parsed']
                expected_failed = dict(expected)
                chosen = [name for name, _ in expected['tests'] if name == args.edit_test]
                if len(chosen) != 1:
                    continue
                expected_failed.update(passed=expected['passed'] - 1, failed=1,
                    tests=[(name, 'FAILED' if name == chosen[0] else state) for name, state in expected['tests']])
                fail_ok = (row['status'] == 101 and not row['timed_out'] and not row['error']
                           and parsed == expected_failed and 'JR_RUNTIME_FAILURE' in output)
                row['gate_passed'] = fail_ok
                row, output = invoke(stack, 'test-recovery', 0, env, original)
                if not fail_ok or not good(row, output):
                    continue
            active.append(stack)
        for rep in range(args.samples):
            contents = edit_bytes(original, rep, anchor)
            for stack in order(stacks, rep):
                if stack not in active:
                    continue
                row, output = invoke(stack, 'edit', rep, environments[stack], contents)
                row['accepted'] = good(row, output, f'JR_RUNTIME_EXECUTED_{rep + 1}')
        for stack in active:
            rows = [r for r in records if r['stack'] == stack and r['phase'] == 'edit']
            if len(rows) == args.samples and all(r['accepted'] for r in rows):
                medians[stack] = statistics.median(r['elapsed_seconds'] for r in rows)
    finally:
        pinned.write_bytes(original)
        metadata['source_after'] = source_hashes(args.source, args.output)
        metadata['source_restored'] = pinned.read_bytes() == original
        try:
            metadata['identities_after'] = identities(args)
            metadata['wild_copy_hashes_after'] = {stack: base.digest(item['path'])
                for stack, item in metadata['wild_copies'].items()}
            copies_valid = all(metadata['wild_copy_hashes_after'][stack] == item['sha256']
                               for stack, item in metadata['wild_copies'].items())
            metadata['seed_source_after'] = seed_manifest(args.blaze_seed_cache) if args.blaze_seed_cache else None
            valid = (copies_valid and metadata['source_before'] == metadata['source_after'] and
                     metadata['identities_before'] == metadata['identities_after'] and
                     metadata['seed_source_before'] == metadata['seed_source_after'])
        except Exception as exc:
            metadata['identity_error'] = repr(exc)
            valid = False
        if not valid:
            medians.clear()
        metadata['unchanged'] = valid
        save()
        (args.output / 'summary.json').write_text(json.dumps({
            'medians_seconds': medians, 'excluded': [s for s in stacks if s not in medians],
            'store_scope': metadata['store_scope'],
            'edit_hotpatch_outcomes': {stack: [outcome for row in records
                if row['stack'] == stack and row['phase'] == 'edit'
                for outcome in row.get('raw_output', {}).get('hotpatch', {}).get('outcomes', [])]
                for stack in stacks if stack == 'all4hotpatch'},
            'policy': 'Failed, stale, incomplete or identity-changed stacks excluded. Gates untimed. Descriptive scoped diagnostic only.'}, indent=2) + '\n')
    return 0 if len(medians) == len(stacks) else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('source', 'output', 'blaze', 'justrust', 'rustc', 'cargo'):
        parser.add_argument('--' + key, required=True, type=lambda s: Path(s).expanduser().resolve())
    parser.add_argument('--wild', type=lambda s: Path(s).expanduser().resolve(),
                        help='copy this linker into every private owned toolchain home')
    parser.add_argument('--samples', type=int, default=5)
    parser.add_argument('--command', choices=('test', 'build'), default='test')
    parser.add_argument('--binary', default='justrust')
    parser.add_argument('--test-filter', default='summary::tests::')
    parser.add_argument('--edit-test', default='summary::tests::shares_overlapping_time')
    parser.add_argument('--edit-file', default='src/summary.rs')
    parser.add_argument('--edit-anchor', default='fn shares_overlapping_time() {')
    parser.add_argument('--blaze-llvm', action='store_true')
    parser.add_argument('--owned-experiments', action='store_true',
                        help='add isolated all4on and all4hotpatch stacks')
    parser.add_argument('--blaze-seed-cache', type=lambda s: Path(s).expanduser().absolute(),
                        help='copy a private successful store into each fresh Blaze cache; warm-store scope only')
    parser.add_argument('--timeout', type=float, default=360)
    args = parser.parse_args()
    if args.owned_experiments and not args.wild:
        parser.error('--owned-experiments requires --wild so hotpatch is not inert')
    if args.samples < 1 or args.timeout <= 0:
        parser.error('samples and timeout must be positive')
    if Path(args.edit_file).is_absolute() or '..' in Path(args.edit_file).parts:
        parser.error('edit-file must be relative within source')
    raise SystemExit(execute(args))


if __name__ == '__main__':
    main()
