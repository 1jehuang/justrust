#!/usr/bin/env python3
"""Private controlled-local three-stack experiment. Never runs native tools on import.

Run requires pristine f97fcc2 (63 files), explicit native executables and a
root-prepared Cargo registry-only public source seed. No fetch, install, global
cache copying or fourth-stack relabeling is performed. Replay is offline and
validates retained evidence, not a cryptographic attestation of the machine.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import statistics
import uuid

import resident_engine_compare as resident
import blaze_runtime_compare as runtime

STACKS = ('cargo', 'justrust', 'blaze')
PHASES = ('cold', 'warm-noop', 'body-edit', 'type-error', 'type-recovery',
          'assert-failure', 'assert-recovery')
SCHEMA = 'three-stack-v91-1'
POLICY = {
    'mode': 'controlled-local, matched compiler/profile, private raw paths',
    'cold': 'unseeded targets, daemon and sysroot; registry public sources preseeded only',
    'compiler': 'Cargo and shipping justrust use identical pinned LLVM rustc; Blaze uses Cranelift (intrinsic stack difference)',
    'cache_features': 'local caches enabled; speculative build daemon/prebuild, remote/peer/prebuilt downloads and telemetry explicitly disabled',
    'timing_boundary': 'full tool/seed verification before source mutation; no speculative precompile; source verification then command wall time',
    'proc_macro_coverage': 'BLAZE_PROC_MACRO_SERVER=0 and RA_JIT_PROC_MACRO_SERVER=0; rustc-built proc macros only, native/Wasm proc-macro-server coverage excluded',
    'network': 'Cargo/JR --locked --offline; Blaze flags omitted (not established for test CLI), CARGO_NET_OFFLINE=true; not an OS network sandbox',
    'claims': 'descriptive phase-specific only, no pooled cold/warm score, significance or broad speed claim',
}


def digest(path):
    return runtime.base.digest(path)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()


def seal(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def absolute(value):
    return resident.absolute(value)


def permutations(sample):
    # Three rotations followed by three reverse rotations, all six exactly once.
    order = STACKS if sample % 6 < 3 else tuple(reversed(STACKS))
    p = sample % 3
    return order[p:] + order[:p]


def command(args, stack):
    tool = getattr(args, stack)
    return [str(tool)] + (['cargo'] if stack == 'blaze' else []) + ['test'] + (
        [] if stack == 'blaze' else ['--locked', '--offline']) + resident.FILTER


def validate(args):
    if args.samples < 6 or args.samples % 6 or not math.isfinite(args.timeout) or args.timeout <= 0:
        raise ValueError('samples must be a positive multiple of six and timeout finite/positive')
    paths = [args.cargo, args.rustc, args.justrust, args.blaze, args.toolchain,
             args.source, args.output, args.cargo_seed]
    for p in paths:
        absolute(str(p))
    if args.output.exists() or len(os.fsencode(args.output)) > 70:
        raise ValueError('output must be new and <=70 bytes for private daemon sockets')
    for p in paths:
        if p != args.output and (p.is_relative_to(args.output) or args.output.is_relative_to(p)):
            raise ValueError('output collides with an input')
    if args.cargo != args.toolchain / 'bin/cargo' or args.rustc != args.toolchain / 'bin/rustc':
        raise ValueError('Cargo and rustc must be the pinned toolchain binaries')
    if len({args.cargo, args.rustc, args.justrust, args.blaze}) != 4:
        raise ValueError('distinct native tools required')
    for p in (args.cargo, args.rustc, args.justrust, args.blaze):
        if not p.is_file() or not os.access(p, os.X_OK) or p.read_bytes()[:4] != b'\x7fELF':
            raise ValueError('tools must be executable ELF files, not wrappers')
    resident.pinned_source(args.source)
    if len(resident.source_hashes(args.source)) != 63:
        raise ValueError('expected exactly 63 pinned workload files')
    public_seed(args.cargo_seed)


def public_seed(root):
    files = runtime.seed_manifest(root)
    if not files:
        raise ValueError('empty source seed')
    for name in files:
        parts = Path(name).parts
        if (len(parts) < 4 or parts[0] != 'registry' or parts[1] not in ('src', 'cache', 'index')
                or not re.fullmatch(r'(?:index\.crates\.io|github\.com)-[0-9a-f]+', parts[2])):
            raise ValueError('seed must contain only public crates.io registry source/cache/index files')
        if parts[1] == 'src':
            # Classify Cargo-home layout, not Rust module names. Public sources
            # legitimately contain Security/Credentials and src/target/*.rs.
            # Their provenance is the root-supplied public registry seed contract,
            # not inferred from arbitrary names inside a crate's source tree.
            crate = Path(*parts[:4])
            if (len(parts) < 5 or not re.fullmatch(
                    r'[A-Za-z0-9_-]+-[0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.+-]+)?', parts[3])
                    or str(crate / 'Cargo.toml') not in files):
                raise ValueError('registry source must use a crate-name-semver directory with Cargo.toml')
        if parts[1] == 'cache' and not name.endswith('.crate'):
            raise ValueError('registry cache must contain source .crate archives only')
        if parts[1] == 'index' and parts[-1] == 'config.json':
            config = json.loads((root / name).read_text())
            if config.get('dl') != 'https://static.crates.io/crates' or config.get('api') != 'https://crates.io':
                raise ValueError('seed registry is not public crates.io')
    return files


def identities(args):
    return {'tools': {k: {'path': str(getattr(args, k)), 'sha256': digest(getattr(args, k))}
                      for k in ('cargo', 'rustc', 'justrust', 'blaze')},
            'toolchain': resident.toolchain_identity(args.toolchain),
            'harness': {p.name: digest(p) for p in (Path(__file__), Path(resident.__file__),
                         Path(runtime.__file__), Path(runtime.base.__file__))}}


def environment(args, sample, stack):
    root = args.output / f'r{sample}' / stack
    root.mkdir(parents=True, mode=0o700, exist_ok=False)
    # Construct a minimal environment, not a copy of ambient secrets or loader flags.
    env = {'PATH': str(args.toolchain / 'bin') + ':/usr/bin:/bin', 'LANG': 'C.UTF-8',
           'LC_ALL': 'C.UTF-8', 'TZ': 'UTC', **runtime.PROFILES}
    dirs = {'HOME': 'home', 'CARGO_TARGET_DIR': 'target', 'JUSTRUST_HOME': 'jr',
            'BLAZE_HOME': 'blaze', 'BLAZE_CACHE': 'blaze-cache', 'RA_JIT_HOME': 'blaze',
            'RA_JIT_CACHE': 'blaze-cache', 'RUSTUP_HOME': 'rustup', 'XDG_CACHE_HOME': 'cache',
            'XDG_CONFIG_HOME': 'config', 'XDG_DATA_HOME': 'data', 'XDG_RUNTIME_DIR': 'runtime',
            'TMPDIR': 'tmp'}
    for key, name in dirs.items():
        (root / name).mkdir(mode=0o700, exist_ok=True)
        env[key] = str(root / name)
    runtime.seed_cache(args.cargo_seed, root / 'cargo')
    env.update(CARGO_HOME=str(root / 'cargo'), CARGO_NET_OFFLINE='true',
               RUSTC=str(args.rustc), JUSTRUST_REAL_RUSTC=str(args.rustc),
               JUSTRUST_REAL_CARGO=str(args.cargo), JUSTRUST_TOOLCHAIN='system',
               JUSTRUST_TOOLCHAIN_AUTO_INSTALL='0', JUSTRUST_REMOTE='off',
               JUSTRUST_PREBUILT='off', JUSTRUST_PREBUILT_ALWAYS='0', JUSTRUST_NO_TELEMETRY='1',
               JUSTRUST_BUILDD='0', JUSTRUST_BUILDD_PREBUILD='0',
               JUSTRUST_REMOTE_CACHE='0', JUSTRUST_PEER_CACHE='0',
               BLAZE_PROC_MACRO_SERVER='0', RA_JIT_PROC_MACRO_SERVER='0',
               BLAZE_REMOTE='off', RA_JIT_REMOTE='off', BLAZE_BACKEND='cranelift', BLAZE_OPT='0')
    links = root / 'blaze/toolchains'
    links.mkdir()
    (links / args.toolchain.name).symlink_to(args.toolchain, target_is_directory=True)
    return env


def mutation(original, phase, nonce):
    if phase in ('cold', 'warm-noop'):
        return original
    if phase == 'type-error':
        return runtime.edit_bytes(original, kind='type-error')
    if phase == 'assert-failure':
        return runtime.edit_bytes(original, kind='test-failure')
    # The original tuple-valued assertions remain unchanged. The fresh reachable
    # test body proves current-code execution, not just cached compilation status.
    anchor = b'fn shares_overlapping_time() {'
    if original.count(anchor) != 1:
        raise ValueError('unique reachable test anchor required')
    body = ('\nlet observed = std::hint::black_box(' + str(nonce) + 'u64); '
            'assert_eq!(observed, ' + str(nonce) + 'u64); '
            'println!("JR_RUNTIME_EXECUTED_' + str(nonce) + '");').encode()
    return original.replace(anchor, anchor + body, 1)


def accepted(row, text, expected):
    if row.get('error') or row.get('timed_out'):
        return False
    phase = row['phase']
    parsed = runtime.test_results(text)
    if phase == 'type-error':
        return (row['status'] == 101 and parsed is None and 'E0308' in text
                and 'JR_RUNTIME_TYPE_ERROR' in text and 'JR_RUNTIME_EXECUTED_' not in text)
    if phase == 'assert-failure':
        return (row['status'] == 101 and resident.valid_tests(parsed, expected, failing=True)
                and 'JR_RUNTIME_FAILURE' in text and 'JR_RUNTIME_EXECUTED_' not in text)
    markers = re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*', text)
    wanted = [] if row['nonce'] is None else ['JR_RUNTIME_EXECUTED_' + str(row['nonce'])]
    return (row['status'] == 0 and resident.valid_tests(parsed, expected)
            and (expected is None or parsed == expected) and markers == wanted)


def route_valid(meta, args):
    return (meta.get('real_cargo') == str(args.cargo) and meta.get('real_rustc') == str(args.rustc)
            and meta.get('place') == 'local' and meta.get('toolchain') is None)


class Journal:
    """An fsynced append-only prefix survives every validation and process error."""
    def __init__(self, output):
        self.path = output / 'journal.jsonl'
        self.previous = None

    def add(self, kind, data):
        entry = {'kind': kind, 'data': data, 'previous': self.previous}
        entry['sha256'] = seal(entry)
        with self.path.open('ab') as stream:
            stream.write(canonical(entry) + b'\n')
            stream.flush()
            os.fsync(stream.fileno())
        self.previous = entry['sha256']


def artifact(output, path):
    return {'file': str(path.relative_to(output)), 'sha256': digest(path)}


def execute(args):
    # Capture timestamps before pinned-source and full-manifest reads touch atime.
    absolute(str(args.source))
    timestamps = {str(p.relative_to(args.source)): p.stat() for p in args.source.rglob('*')
                  if p.is_file() and not p.is_symlink() and '.git' not in p.relative_to(args.source).parts}
    validate(args)
    args.output.mkdir(parents=True, mode=0o700)
    journal = Journal(args.output)
    baseline, source = identities(args), resident.source_hashes(args.source)
    seed = public_seed(args.cargo_seed)
    journal.add('header', {'schema': SCHEMA, 'policy': POLICY, 'samples': args.samples,
        'original_output': str(args.output),
        'source': str(args.source), 'pin': resident.PIN, 'source_before': source,
        'identities': baseline, 'seed': seed, 'profiles': runtime.PROFILES,
        'blaze_identity_policy': 'explicit root-supplied source-built reference ELF, hash retained; build provenance is root responsibility'})
    backups, environments, rows = {}, [], []
    expected, error = None, None
    pinned = args.source / 'src/summary.rs'
    try:
        for name in source:
            p = args.source / name
            st = timestamps[name]
            data = p.read_bytes()
            backup = args.output / 'backups' / name
            backup.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            with backup.open('xb') as f:
                f.write(data)
                f.flush()
                os.fsync(f.fileno())
            backups[name] = (backup, st.st_atime_ns, st.st_mtime_ns, st.st_mode)
        original = backups['src/summary.rs'][0].read_bytes()
        journal.add('backups', {name: {'artifact': artifact(args.output, v[0]),
                      'atime_ns': v[1], 'mtime_ns': v[2], 'mode': v[3]} for name, v in backups.items()})
        for sample in range(args.samples):
            for stack in permutations(sample):
                pinned.write_bytes(original)
                env = environment(args, sample, stack)
                environments.append((sample, stack, env))
                root = args.output / f'r{sample}' / stack
                journal.add('state', {'sample': sample, 'stack': stack, 'root': str(root),
                    'environment': env, 'seed_copied': runtime.seed_manifest(root / 'cargo'),
                    'empty': {name: not any((root / name).iterdir()) for name in
                              ('target', 'jr', 'blaze-cache', 'runtime')},
                    'blaze_home_only_toolchain_link': sorted(p.name for p in (root / 'blaze').iterdir()) == ['toolchains']})
                for phase in PHASES:
                    nonce = 1 + int(uuid.uuid4().hex[:15], 16) if phase in ('body-edit', 'type-recovery', 'assert-recovery') else None
                    contents = mutation(original, phase, nonce)
                    # Expensive identity work must not give a source watcher an
                    # untimed edit-to-command head start. Verify it before editing.
                    if identities(args) != baseline or public_seed(args.cargo_seed) != seed:
                        raise ValueError('tool or seed identity changed before mutation')
                    if pinned.read_bytes() != contents:
                        pinned.write_bytes(contents)
                    current_source = resident.source_hashes(args.source)
                    wanted = dict(source, **{'src/summary.rs': hashlib.sha256(contents).hexdigest()})
                    if current_source != wanted:
                        raise ValueError('input identity changed before invocation')
                    prefix = root / phase
                    before_runs = runtime.recorded_runs(env) if stack == 'justrust' else None
                    intent = {'sample': sample, 'stack': stack, 'phase': phase, 'nonce': nonce,
                              'command': command(args, stack), 'source': current_source,
                              'identities_sha256': seal(baseline), 'seed_sha256': seal(seed),
                              'environment_sha256': seal(env), 'prefix': str(prefix.relative_to(args.output))}
                    journal.add('intent', intent)
                    row = runtime.run_process(command(args, stack), args.source, env, prefix, args.timeout)
                    # Keep every full console and status before interpreting any output.
                    raw = {s: artifact(args.output, prefix.with_suffix('.' + s)) for s in ('stdout', 'stderr')}
                    row.update(intent, raw=raw)
                    journal.add('process', row)
                    text = '\n'.join(prefix.with_suffix('.' + s).read_text(errors='replace') for s in ('stdout', 'stderr'))
                    route = None
                    if stack == 'justrust':
                        text, receipt = runtime.recorded_output(env, before_runs, prefix)
                        meta_path = Path(env['JUSTRUST_HOME']) / 'runs' / receipt['run_id'] / 'meta.json'
                        route = json.loads(meta_path.read_text())
                        retained = prefix.with_suffix('.meta.json')
                        retained.write_bytes(meta_path.read_bytes())
                        row['recorded'] = {'output': artifact(args.output, prefix.with_suffix('.raw-output.jsonl')),
                                           'meta': artifact(args.output, retained)}
                        if not route_valid(route, args):
                            raise ValueError('shipping justrust route/compiler metadata unsupported or mismatched')
                    row['parsed'] = runtime.test_results(text)
                    if expected is None and stack == 'cargo' and phase == 'cold':
                        if not accepted(row, text, None):
                            raise ValueError('stock baseline failed independent known-name/counter oracle')
                        expected = row['parsed']
                        journal.add('expected', expected)
                    if expected is None:
                        raise ValueError('stock baseline must establish exact filtered counter first')
                    row['accepted'] = accepted(row, text, expected)
                    journal.add('outcome', row)
                    rows.append(row)
                    print(f"three-stack: sample={sample + 1}/{args.samples} stack={stack} phase={phase} "
                          f"status={row['status']} accepted={row['accepted']} elapsed={row['elapsed_seconds']:.6f}s",
                          flush=True)
                    if not row['accepted']:
                        raise ValueError(f'failed semantic gate: {stack}/{sample}/{phase}')
    except BaseException as exc:
        error = repr(exc)
        journal.add('failure', {'error': error})
    finally:
        restoration = {}
        for name, (backup, atime, mtime, mode) in backups.items():
            try:
                p = args.source / name
                absolute(str(p))
                if digest(backup) != source[name]:
                    raise ValueError('backup changed, refusing corrupt source restoration')
                p.write_bytes(backup.read_bytes())
                os.chmod(p, mode)
                os.utime(p, ns=(atime, mtime))
                restoration[name] = digest(p) == source[name] and p.stat().st_mtime_ns == mtime
                # Hashing itself reads the file, so reset atime after verification.
                os.utime(p, ns=(atime, mtime))
            except BaseException as exc:
                restoration[name] = repr(exc)
        for sample, stack, env in environments:
            if stack != 'blaze':
                continue
            try:
                if identities(args) != baseline:
                    raise ValueError('tool changed, refusing cleanup executable')
                prefix = args.output / f'r{sample}' / stack / 'stop'
                result = runtime.run_process([str(args.blaze), 'cargo', 'stop'], args.source, env, prefix, args.timeout)
                journal.add('cleanup', {'sample': sample, 'stack': stack, 'process': result,
                    'raw': {s: artifact(args.output, prefix.with_suffix('.' + s)) for s in ('stdout', 'stderr')}})
                if result['status'] != 0 or result['error'] or result['timed_out']:
                    error = error or 'private Blaze stop failed'
            except BaseException as exc:
                error = error or repr(exc)
                journal.add('cleanup-error', {'sample': sample, 'stack': stack, 'error': repr(exc)})
        try:
            after = identities(args)
            source_after = resident.source_hashes(args.source)
            unchanged = after == baseline and source_after == source and public_seed(args.cargo_seed) == seed
        except BaseException as exc:
            after, source_after, unchanged = None, None, False
            error = error or repr(exc)
        # Restore all timestamps again after the full source manifest read.
        for name, (_, atime, mtime, _) in backups.items():
            try:
                os.utime(absolute(str(args.source / name)), ns=(atime, mtime))
            except (OSError, ValueError, argparse.ArgumentTypeError) as exc:
                error = error or repr(exc)
        complete = (not error and unchanged and len(rows) == args.samples * 3 * len(PHASES)
                    and len(restoration) == 63 and all(v is True for v in restoration.values()))
        journal.add('footer', {'status': 'complete' if complete else 'failed/incomplete',
            'error': error, 'identities_after': after, 'source_after': source_after,
            'restoration': restoration, 'unchanged': unchanged})
    if complete:
        result = replay(args.output)
        (args.output / 'summary.json').write_bytes(canonical(result) + b'\n')
    return 0 if complete else 1


def read_artifact(output, ref):
    relative = Path(ref['file'])
    if relative.is_absolute() or '..' in relative.parts:
        raise ValueError('unsafe artifact path')
    path = absolute(str(output / relative))
    if digest(path) != ref['sha256']:
        raise ValueError('artifact changed: ' + str(relative))
    return path.read_text(errors='replace')


def stats(rows):
    result = {}
    for phase in PHASES:
        result[phase] = {}
        for stack in STACKS:
            values = [r['elapsed_seconds'] for r in rows if r['phase'] == phase and r['stack'] == stack]
            if not values or any(not isinstance(v, (int, float)) or not math.isfinite(v) or v <= 0 for v in values):
                raise ValueError('invalid elapsed evidence')
            result[phase][stack] = {'samples_seconds': values, 'median': statistics.median(values),
                                    'min': min(values), 'max': max(values)}
        cargo = result[phase]['cargo']['median']
        result[phase]['descriptive_cargo_over_stack_median'] = {
            s: cargo / result[phase][s]['median'] for s in STACKS}
    return result


def replay(output):
    """No subprocesses and no reads of installed executables or workload required."""
    output = absolute(str(output))
    entries, previous = [], None
    for line in (output / 'journal.jsonl').read_bytes().splitlines():
        entry = json.loads(line)
        saved = entry.pop('sha256')
        if entry['previous'] != previous or seal(entry) != saved:
            raise ValueError('journal hash chain changed')
        previous = saved
        entries.append(entry)
    if not entries or entries[0]['kind'] != 'header' or entries[-1]['kind'] != 'footer':
        raise ValueError('incomplete evidence prefix')
    header, footer = entries[0]['data'], entries[-1]['data']
    original_output = Path(header['original_output'])
    if not original_output.is_absolute() or '..' in original_output.parts:
        raise ValueError('invalid original output root')
    if (header['schema'] != SCHEMA or header['policy'] != POLICY or header['profiles'] != runtime.PROFILES
            or footer['status'] != 'complete' or footer['error'] or not footer['unchanged']
            or footer['identities_after'] != header['identities']
            or footer['source_after'] != header['source_before']
            or len(footer['restoration']) != 63 or not all(v is True for v in footer['restoration'].values())):
        raise ValueError('incomplete or changed identity/restoration evidence')
    samples = header['samples']
    if not isinstance(samples, int) or samples < 6 or samples % 6:
        raise ValueError('invalid round count')
    kinds = ['header', 'backups']
    for sample in range(samples):
        for stack in permutations(sample):
            kinds.append('state')
            for phase in PHASES:
                kinds += ['intent', 'process']
                if (sample, stack, phase) == (0, 'cargo', 'cold'):
                    kinds.append('expected')
                kinds.append('outcome')
    kinds += ['cleanup'] * samples + ['footer']
    if [e['kind'] for e in entries] != kinds:
        raise ValueError('invalid event chronology or failure in complete evidence')
    if header['pin'] != resident.PIN or len(header['source_before']) != 63:
        raise ValueError('invalid pinned source identity')
    backups = [e['data'] for e in entries if e['kind'] == 'backups']
    if len(backups) != 1 or set(backups[0]) != set(header['source_before']):
        raise ValueError('missing source backups')
    for name, ref in backups[0].items():
        read_artifact(output, ref['artifact'])
        if ref['artifact']['sha256'] != header['source_before'][name]:
            raise ValueError('backup differs from pinned source')
    original = (output / backups[0]['src/summary.rs']['artifact']['file']).read_bytes()
    baseline = [e['data'] for e in entries if e['kind'] == 'expected']
    if len(baseline) != 1:
        raise ValueError('missing independent known test baseline')
    expected = baseline[0]
    expected['tests'] = [tuple(v) for v in expected['tests']]
    if not resident.valid_tests(expected):
        raise ValueError('invalid independent known test baseline')
    rows = [e['data'] for e in entries if e['kind'] == 'outcome']
    wanted = [(n, s, p) for n in range(samples) for s in permutations(n) for p in PHASES]
    if [(r['sample'], r['stack'], r['phase']) for r in rows] != wanted:
        raise ValueError('incomplete or reordered phases')
    intents = [e['data'] for e in entries if e['kind'] == 'intent']
    processes = [e['data'] for e in entries if e['kind'] == 'process']
    if len(intents) != len(rows) or len(processes) != len(rows):
        raise ValueError('missing durable process prefix')
    states = [e['data'] for e in entries if e['kind'] == 'state']
    if [(s['sample'], s['stack']) for s in states] != [(n, s) for n in range(samples) for s in permutations(n)]:
        raise ValueError('missing fresh-state evidence')
    homes = set()
    environments = {}
    for state in states:
        root = original_output / f"r{state['sample']}" / state['stack']
        if (state['root'] != str(root) or state['seed_copied'] != header['seed']
                or set(state['empty']) != {'target', 'jr', 'blaze-cache', 'runtime'}
                or not all(v is True for v in state['empty'].values())
                or not state['blaze_home_only_toolchain_link']):
            raise ValueError('cold-state isolation mismatch')
        env = state['environment']
        environments[state['sample'], state['stack']] = env
        if any(k.startswith(('LD_', 'DYLD_')) for k in env) or any(env.get(k) != v for k, v in runtime.PROFILES.items()):
            raise ValueError('unsafe environment or profile')
        controls = {'JUSTRUST_TOOLCHAIN': 'system', 'JUSTRUST_REMOTE': 'off',
                    'JUSTRUST_PREBUILT': 'off', 'JUSTRUST_PREBUILT_ALWAYS': '0', 'JUSTRUST_NO_TELEMETRY': '1',
                    'JUSTRUST_BUILDD': '0', 'JUSTRUST_BUILDD_PREBUILD': '0',
                    'JUSTRUST_REMOTE_CACHE': '0', 'JUSTRUST_PEER_CACHE': '0',
                    'BLAZE_PROC_MACRO_SERVER': '0', 'RA_JIT_PROC_MACRO_SERVER': '0',
                    'CARGO_NET_OFFLINE': 'true', 'BLAZE_BACKEND': 'cranelift', 'BLAZE_OPT': '0'}
        if any(env.get(k) != v for k, v in controls.items()):
            raise ValueError('local isolation controls changed')
        for key in ('CARGO_HOME', 'CARGO_TARGET_DIR', 'JUSTRUST_HOME', 'BLAZE_HOME'):
            home = Path(env[key])
            if not home.is_relative_to(root) or str(home) in homes:
                raise ValueError('shared private cache')
            homes.add(str(home))
    nonces = set()
    tools = header['identities']['tools']
    args = argparse.Namespace(**{s: Path(tools[s]['path']) for s in STACKS}, rustc=Path(tools['rustc']['path']))
    for row, intent, process in zip(rows, intents, processes):
        env = environments[row['sample'], row['stack']]
        if (any(row.get(k) != v for k, v in intent.items()) or any(row.get(k) != v for k, v in process.items())
                or row['identities_sha256'] != seal(header['identities'])
                or row['seed_sha256'] != seal(header['seed'])
                or row['environment_sha256'] != seal(env)
                or row['environment'] != runtime.logged_environment(env)
                or env.get('RUSTC') != str(args.rustc)
                or env.get('JUSTRUST_REAL_RUSTC') != str(args.rustc)
                or env.get('JUSTRUST_REAL_CARGO') != str(args.cargo)
                or row['prefix'] != f"r{row['sample']}/{row['stack']}/{row['phase']}"
                or row['noise'].get('contamination') != 'unknown'
                or row['command'] != command(args, row['stack'])):
            raise ValueError('invocation identity/command mismatch')
        nonce = row['nonce']
        needs_nonce = row['phase'] in ('body-edit', 'type-recovery', 'assert-recovery')
        if needs_nonce and (not isinstance(nonce, int) or nonce <= 0 or nonce in nonces):
            raise ValueError('stale execution nonce')
        if not needs_nonce and nonce is not None:
            raise ValueError('unexpected nonce')
        if nonce is not None:
            nonces.add(nonce)
        contents = mutation(original, row['phase'], nonce)
        source = dict(header['source_before'], **{'src/summary.rs': hashlib.sha256(contents).hexdigest()})
        if row['source'] != source:
            raise ValueError('source mutation differs')
        text = '\n'.join(read_artifact(output, row['raw'][s]) for s in ('stdout', 'stderr'))
        if row['stack'] == 'justrust':
            raw = read_artifact(output, row['recorded']['output'])
            lines = [json.loads(line) for line in raw.splitlines()]
            if not lines or any(e.get('s') not in ('o', 'e') or not isinstance(e.get('l'), str) for e in lines):
                raise ValueError('invalid recorded raw output')
            text = '\n'.join(e['l'] for e in lines) + '\n'
            meta = json.loads(read_artifact(output, row['recorded']['meta']))
            if not route_valid(meta, args):
                raise ValueError('unverified shipping compiler/local route')
        if canonical(row['parsed']) != canonical(runtime.test_results(text)) or not row['accepted'] or not accepted(row, text, expected):
            raise ValueError('raw semantic oracle mismatch')
    cleanups = [e['data'] for e in entries if e['kind'] == 'cleanup']
    if [c['sample'] for c in cleanups] != list(range(samples)):
        raise ValueError('missing private daemon cleanup')
    for cleanup in cleanups:
        p = cleanup['process']
        if (cleanup['stack'] != 'blaze' or p['command'] != [tools['blaze']['path'], 'cargo', 'stop']
                or p['status'] != 0 or p['error'] or p['timed_out']):
            raise ValueError('daemon cleanup failed')
        for ref in cleanup['raw'].values():
            read_artifact(output, ref)
    return {'schema': SCHEMA, 'status': 'complete', 'policy': POLICY, 'statistics': stats(rows)}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    run = sub.add_parser('run')
    for name in ('cargo', 'rustc', 'justrust', 'blaze', 'toolchain', 'source', 'output', 'cargo-seed'):
        run.add_argument('--' + name, type=absolute, required=True)
    run.add_argument('--samples', type=int, default=6)
    run.add_argument('--timeout', type=float, default=360)
    audit = sub.add_parser('replay')
    audit.add_argument('--output', type=absolute, required=True)
    args = parser.parse_args(argv)
    if args.action == 'replay':
        print(json.dumps(replay(args.output), indent=2))
        return 0
    return execute(args)


if __name__ == '__main__':
    raise SystemExit(main())
