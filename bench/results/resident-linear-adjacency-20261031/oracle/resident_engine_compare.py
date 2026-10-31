#!/usr/bin/env python3
"""Private, descriptive source-built ra-jit versus Blaze-derived jr-engine test parity."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import statistics
import subprocess

import blaze_runtime_compare as runtime

PIN = 'f97fcc2'
REFERENCE_VERSION = 'blaze 0.20261027.0 (5d4f78f7577cf7170993b74f735099dd03f5c6ca)'
TOOLCHAIN_NAME = 'nightly-2026-10-27-x86_64-unknown-linux-gnu'
TOOLCHAIN_MANIFEST = '34f3d9bf04640a082e561310f3f258846397f4cf64a2bcfebcea96ebf18e2572'
NAMES = sorted('summary::tests::' + name for name in (
    'shares_overlapping_time', 'splits_passes_without_double_counting',
    'parses_libtest_results', 'test_failures_are_not_compile_errors'))
EDIT_TEST = 'summary::tests::shares_overlapping_time'
FILTER = ['--lib', '--', 'summary::tests::', '--test-threads=1', '--nocapture']


def absolute(value):
    path = Path(value).expanduser()
    if not path.is_absolute() or '..' in path.parts:
        raise argparse.ArgumentTypeError('must be an absolute path without ..')
    if any(p.is_symlink() for p in (path, *path.parents)):
        raise argparse.ArgumentTypeError('symlink paths are not allowed')
    return path


def pinned_source(source):
    """Verify every tracked blob, not merely the edited file or a claimed revision."""
    repo = Path(__file__).resolve().parent.parent
    tree = subprocess.check_output(['git', 'ls-tree', '-rz', PIN], cwd=repo)
    tracked = set()
    for record in tree.split(b'\0'):
        if not record:
            continue
        info, filename = record.split(b'\t', 1)
        mode, kind, oid = info.split()
        path = source / os.fsdecode(filename)
        tracked.add(os.fsdecode(filename))
        if kind != b'blob' or mode not in (b'100644', b'100755'):
            raise ValueError('unsupported pinned tree entry')
        if any(p.is_symlink() for p in (path, *path.parents)):
            raise ValueError('source links are not permitted')
        contents = path.read_bytes()
        actual = hashlib.sha1(b'blob ' + str(len(contents)).encode() + b'\0' + contents).hexdigest()
        if actual != oid.decode():
            raise ValueError(f'source does not match {PIN}: {filename!r}')
    actual_paths = set(source_hashes(source))
    if actual_paths != tracked:
        raise ValueError('source must be a pristine pinned copy without extra files')


def source_hashes(source):
    """Full input file SHA256 manifest, excluding only Git administration."""
    result = {}
    for path in sorted(source.rglob('*')):
        relative = path.relative_to(source)
        if '.git' in relative.parts:
            continue
        if path.is_symlink():
            raise ValueError(f'source must not contain links: {relative}')
        if path.is_file():
            result[str(relative)] = runtime.base.digest(path)
        elif not path.is_dir():
            raise ValueError(f'source must not contain special files: {relative}')
    return result


def command(tool, stack, operation='test', arguments=None):
    if operation not in ('test', 'build', 'check', 'run'):
        raise ValueError('unsupported direct engine operation')
    return [str(tool)] + (['cargo'] if stack == 'reference' else []) + [operation] + (
        FILTER if arguments is None else arguments)


def toolchain_identity(root):
    """Read-only inspection. Refuse incomplete inputs rather than installing anything."""
    if root.name != TOOLCHAIN_NAME or not root.is_dir():
        raise ValueError('toolchain must be the existing pinned ' + TOOLCHAIN_NAME)
    manifest = root / 'lib/rustlib/multirust-channel-manifest.toml'
    if runtime.base.digest(manifest) != TOOLCHAIN_MANIFEST:
        raise ValueError('toolchain channel manifest does not match the pinned release')
    needed = {'rustc-x86_64-unknown-linux-gnu', 'cargo-x86_64-unknown-linux-gnu',
              'rust-std-x86_64-unknown-linux-gnu', 'rust-std-wasm32-wasip1', 'rust-src'}
    if not needed <= set((root / 'lib/rustlib/components').read_text().splitlines()):
        raise ValueError('toolchain components are incomplete, refusing downloads')
    for name in ('bin/cargo', 'bin/rustc', 'lib/rustlib/src/rust/library',
                 'lib/rustlib/wasm32-wasip1/lib/self-contained/crt1-reactor.o'):
        if not (root / name).exists():
            raise ValueError('toolchain is incomplete: ' + name)
    files = {}
    for path in sorted(root.rglob('*')):
        if path.is_symlink() and (not path.resolve().is_relative_to(root) or path.is_dir()):
            raise ValueError('toolchain links must be internal regular files')
        if path.is_file():
            files[str(path.relative_to(root))] = {'sha256': runtime.base.digest(path),
                'link': os.readlink(path) if path.is_symlink() else None}
        elif not path.is_dir():
            raise ValueError('toolchain contains a special file')
    return {'path': str(root), 'files': files}


def environment(output, stack, toolchain=None, proc_macro_server=None):
    # HOME is intentionally unchanged. Engines must honor their native state roots.
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(('JUSTRUST_', 'JR_ENGINE_', 'RA_JIT_', 'BLAZE_', 'CARGO_',
                                'RUST', 'XDG_'))}
    root = output / stack
    for name in ('blaze-home', 'blaze-cache', 'engine-home', 'ra-home', 'ra-cache',
                 'target', 'cargo-home', 'rustup-home', 'runtime', 'cache', 'config', 'data'):
        (root / name).mkdir(parents=True, mode=0o700)
    env.update(runtime.PROFILES)
    env.update(BLAZE_HOME=str(root / 'blaze-home'), BLAZE_CACHE=str(root / 'blaze-cache'),
               BLAZE_BACKEND='cranelift', BLAZE_OPT='0',
               RA_JIT_HOME=str(root / 'ra-home'), RA_JIT_CACHE=str(root / 'ra-cache'),
               CARGO_HOME=str(root / 'cargo-home'), CARGO_TARGET_DIR=str(root / 'target'),
               RUSTUP_HOME=str(root / 'rustup-home'),
               BLAZE_REMOTE='off', RA_JIT_REMOTE='off',
               BLAZE_PROC_MACRO_SERVER=str(proc_macro_server) if proc_macro_server else '0',
               RA_JIT_PROC_MACRO_SERVER=str(proc_macro_server) if proc_macro_server else '0',
               XDG_RUNTIME_DIR=str(root / 'runtime'), XDG_CACHE_HOME=str(root / 'cache'),
               XDG_CONFIG_HOME=str(root / 'config'), XDG_DATA_HOME=str(root / 'data'))
    if stack == 'engine':
        env['JR_ENGINE_HOME'] = str(root / 'engine-home')
        namespace = root / 'engine-home/resident-v1'
        namespace.mkdir(mode=0o700)
        (namespace / 'runtime').mkdir(mode=0o700)
        env['BLAZE_HOME'] = str(namespace / 'runtime')
        env['RA_JIT_HOME'] = env['BLAZE_HOME']
    if toolchain:
        links = Path(env['BLAZE_HOME']) / 'toolchains'
        links.mkdir(mode=0o700)
        (links / toolchain.name).symlink_to(toolchain, target_is_directory=True)
    return env


def valid_tests(parsed, expected=None, failing=False):
    if parsed is None or [name for name, _ in parsed['tests']] != NAMES:
        return False
    wanted = [(name, 'FAILED' if failing and name == EDIT_TEST else 'ok') for name in NAMES]
    if (parsed['tests'] != wanted or parsed['passed'] != (3 if failing else 4)
            or parsed['failed'] != int(failing) or parsed['ignored'] != 0):
        return False
    return expected is None or parsed['filtered'] == expected['filtered']


def execute(args):
    if args.samples != 5 or args.timeout <= 0:
        raise ValueError('exactly five samples and a positive timeout are required')
    if (args.output.exists() or args.output.is_relative_to(args.source)
            or args.source.is_relative_to(args.output) or len(os.fsencode(args.output)) > 70):
        raise ValueError('output must be fresh, short (<=70 bytes), and disjoint from source')
    pinned_source(args.source)
    args.output.mkdir(mode=0o700, parents=True)
    tools = {'reference': args.reference_engine, 'engine': args.engine}
    if args.blaze:
        tools['installed-blaze'] = args.blaze
    proc_macro_server = getattr(args, 'proc_macro_server', None)
    if proc_macro_server:
        tools['proc-macro-server'] = proc_macro_server
    for name, tool in tools.items():
        with tool.open('rb') as stream:
            if name != 'installed-blaze' and stream.read(4) != b'\x7fELF':
                raise ValueError('source-built engines must be native ELF executables, not wrappers')
    stacks = [s for s in tools if s != 'proc-macro-server']
    pinned = args.source / 'src/summary.rs'
    original = pinned.read_bytes()
    backup = args.output / 'source.original'
    with backup.open('xb') as stream:
        stream.write(original)
        stream.flush()
        os.fsync(stream.fileno())
    metadata = {'pin': PIN, 'reference_version_required': REFERENCE_VERSION,
                'scope': 'actual filtered libtest, descriptive only',
                'store_scope': 'unseeded private cold stores, no automatic seeding',
                'source_before': source_hashes(args.source),
                'tools_before': {s: {'path': str(t), 'sha256': runtime.base.digest(t)} for s, t in tools.items()},
                'toolchain_before': toolchain_identity(args.toolchain),
                'proc_macro_policy': 'same explicit frozen native server' if proc_macro_server else 'compatibility control: rustc-built proc macros, both server variables explicitly 0',
                'versions': {}, 'environments': {}, 'toolchain_links': {},
                'profiles': runtime.PROFILES,
                'native_contract': 'Blaze-derived native jr-engine with JR_ENGINE_HOME and direct test/build/check/run required',
                'native_state_policy': 'Native entry may override requested BLAZE_HOME/CACHE beneath private JR_ENGINE_HOME/resident-v1; environment receipts are launcher inputs',
                'installed_reference': 'separate descriptive only, excluded from source-built parity',
                'status': 'incomplete'}
    rows, medians, active, environments = [], {}, [], {}
    expected = None

    def save():
        (args.output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
        (args.output / 'records.json').write_text(json.dumps(rows, indent=2) + '\n')

    def invoke(stack, phase, rep, env, contents=None, cmd=None):
        if contents is not None:
            pinned.write_bytes(contents)
        prefix = args.output / f'{stack}-{phase}-{rep}'
        row = runtime.run_process(cmd or command(tools[stack], stack), args.source,
                                  env, prefix, args.timeout)
        text = '\n'.join(prefix.with_suffix('.' + suffix).read_text(errors='replace')
                         for suffix in ('stdout', 'stderr'))
        row.update(stack=stack, phase=phase, replicate=rep,
                   parsed=runtime.test_results(text), raw_output='direct stdout/stderr',
                   source_sha256=hashlib.sha256(pinned.read_bytes()).hexdigest())
        rows.append(row)
        save()
        return row, text

    def good(row, text, marker=None):
        return (runtime.base.succeeded(row) and valid_tests(row['parsed'], expected)
                and (marker is None or re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*', text) == [marker]))

    try:
        save()
        for stack in stacks:
            tool = tools[stack]
            env = environment(args.output, stack, args.toolchain, proc_macro_server)
            environments[stack] = env
            metadata['toolchain_links'][stack] = str(
                Path(env['BLAZE_HOME']) / 'toolchains' / args.toolchain.name)
            metadata['environments'][stack] = {k: v for k, v in env.items()
                if k in ('HOME', 'JR_ENGINE_HOME', 'RA_JIT_HOME', 'RA_JIT_CACHE',
                         'CARGO_HOME', 'CARGO_TARGET_DIR', 'BLAZE_HOME', 'BLAZE_CACHE',
                         'BLAZE_BACKEND', 'BLAZE_OPT', 'RUSTUP_HOME', 'BLAZE_REMOTE',
                         'RA_JIT_REMOTE', 'BLAZE_PROC_MACRO_SERVER',
                         'RA_JIT_PROC_MACRO_SERVER') or k in runtime.PROFILES}
            row, text = invoke(stack, 'version', 0, env, cmd=[str(tool), '--version'])
            identity = (re.escape(REFERENCE_VERSION) + r'$' if stack == 'reference' else
                        r'jr-engine\s+\S+' if stack == 'engine' else r'blaze\s+\S+')
            version_ok = runtime.base.succeeded(row) and bool(re.search(
                r'^' + identity, text, re.M))
            metadata['versions'][stack] = {'text': text, 'valid': version_ok}
            if not version_ok:
                continue
            row, text = invoke(stack, 'prepare', 0, env, original)
            if not good(row, text):
                continue
            if expected is None:
                # Only the unmodified source-built reference establishes parity.
                if stack != 'reference':
                    continue
                expected = row['parsed']
                metadata['expected_tests'] = expected
            row, text = invoke(stack, 'type-error', 0, env,
                               runtime.edit_bytes(original, kind='type-error'))
            type_ok = (row['status'] == 101 and not row['error'] and not row['timed_out']
                       and 'E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text)
            row['gate_passed'] = type_ok
            row, text = invoke(stack, 'type-recovery', 0, env, original)
            if not type_ok or not good(row, text):
                continue
            row, text = invoke(stack, 'assert-failure', 0, env,
                               runtime.edit_bytes(original, kind='test-failure'))
            fail_ok = (row['status'] == 101 and not row['error'] and not row['timed_out']
                       and valid_tests(row['parsed'], expected, failing=True)
                       and 'JR_RUNTIME_FAILURE' in text)
            row['gate_passed'] = fail_ok
            row, text = invoke(stack, 'assert-recovery', 0, env, original)
            if fail_ok and good(row, text):
                active.append(stack)
        for rep in range(args.samples):
            contents = runtime.edit_bytes(original, rep)
            for stack in runtime.order(stacks, rep):
                if stack not in active:
                    continue
                row, text = invoke(stack, 'edit', rep, environments[stack], contents)
                row['accepted'] = good(row, text, f'JR_RUNTIME_EXECUTED_{rep + 1}')
        for stack in active:
            measured = [r for r in rows if r['stack'] == stack and r['phase'] == 'edit']
            if len(measured) == 5 and all(r['accepted'] for r in measured):
                medians[stack] = statistics.median(r['elapsed_seconds'] for r in measured)
    except BaseException as exc:
        metadata['error'] = repr(exc)
        raise
    finally:
        pinned.write_bytes(backup.read_bytes())
        metadata['source_restored'] = pinned.read_bytes() == original
        try:
            metadata['source_after'] = source_hashes(args.source)
            metadata['tools_after'] = {s: {'path': str(t), 'sha256': runtime.base.digest(t)} for s, t in tools.items()}
            metadata['toolchain_after'] = toolchain_identity(args.toolchain)
            unchanged = (metadata['source_before'] == metadata['source_after']
                         and metadata['tools_before'] == metadata['tools_after']
                         and metadata['toolchain_before'] == metadata['toolchain_after'])
            metadata['toolchain_links_valid'] = all(
                Path(link).is_symlink() and Path(link).resolve() == args.toolchain
                for link in metadata['toolchain_links'].values())
            unchanged = unchanged and metadata['toolchain_links_valid']
        except (OSError, ValueError) as exc:
            metadata['identity_error'] = repr(exc)
            unchanged = False
        if not unchanged or 'error' in metadata:
            medians.clear()
        metadata['unchanged'] = unchanged
        metadata['status'] = 'complete' if len(medians) == len(stacks) else 'failed/incomplete'
        save()
        (args.output / 'summary.json').write_text(json.dumps({
            'medians_seconds': medians, 'excluded': [s for s in stacks if s not in medians],
            'source_built_parity': all(s in medians for s in ('reference', 'engine')),
            'proc_macro_policy': metadata['proc_macro_policy'],
            'toolchain': str(args.toolchain), 'store_scope': metadata['store_scope'],
            'policy': 'Descriptive only. No general speed claim or independent optimization proof. Cold failures retained, never silently seeded.'}, indent=2) + '\n')
    return 0 if len(medians) == len(stacks) else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('reference-engine', 'engine', 'source', 'output', 'toolchain'):
        parser.add_argument('--' + name, type=absolute, required=True)
    parser.add_argument('--blaze', type=absolute, help='optional installed descriptive reference only')
    parser.add_argument('--proc-macro-server', type=absolute,
                        help='same native ELF server for all stacks, default 0/rustc-built proc macros')
    parser.add_argument('--samples', type=int, default=5)
    parser.add_argument('--timeout', type=float, default=360)
    args = parser.parse_args()
    for tool in (args.reference_engine, args.engine, args.blaze, args.proc_macro_server):
        if tool is not None and (not tool.is_file() or not os.access(tool, os.X_OK)):
            parser.error('tools must be absolute executable files')
    raise SystemExit(execute(args))


if __name__ == '__main__':
    main()
