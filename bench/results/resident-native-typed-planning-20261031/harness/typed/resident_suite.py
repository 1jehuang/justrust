#!/usr/bin/env python3
"""Scoped native resident measurements. Never a comprehensive promotion gate."""
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess
import time
import uuid
from dataclasses import dataclass

import resident_engine_compare as harness

SCHEMA = 'native_resident/v1'
RETOUCH_SCHEMA = 'native_resident_manifest_retouch/v1'
RETOUCH_FLAG = 'RA_JIT_MANIFEST_CONTENT_GENERATION'
RETOUCH_NONCE = 'JR_MANIFEST_RETOUCH_NONCE'
RETOUCH_ORDER = ('off', 'on', 'on', 'off')
RETOUCH_FILES = ('Cargo.toml', 'Cargo.lock')
RETOUCH_MARKER = 'ra-jit: manifest-content-generation-v1 entered'
WATCHED_FLAG = 'RA_JIT_WATCHED_MANIFEST_BYTES'
WATCHED_SCHEMA = 'native_resident_watched_manifest_retouch/v1'
PARALLEL_FLAG = 'RA_JIT_NATIVE_TYPED_PLANNING'
PARALLEL_SCHEMA = 'native_resident_typed_planning_retouch/v1'
PARALLEL_MARKER = 'ra-jit: native-typed-planning-v83 entered'
PARALLEL_RULES = {'workload': 'combined same-byte manifest+lock retouch',
                  'scope': 'noop/clientplanning factor only',
                  'first_touch': 'retain all five samples per segment',
                  'candidate_fixed_flags': {RETOUCH_FLAG: '1', WATCHED_FLAG: '1'},
                  'factor': PARALLEL_FLAG,
                  'reference_flags': {RETOUCH_FLAG: '0', WATCHED_FLAG: '0', PARALLEL_FLAG: '0'},
                  'actual_conversion_stderr_marker': PARALLEL_MARKER,
                  'marker_counts': 'ON timed exactly 1, OFF/reference/version 0, other ON at most 1',
                  'worker': 'same PID/start ticks/exe/socket within each candidate segment'}


@dataclass(frozen=True)
class RetouchPolicy:
    mode: str
    schema: str
    watched: bool = False
    parallel: bool = False

    def flags(self, arm, reference=False):
        if self.parallel:
            return {RETOUCH_FLAG: '0' if reference else '1',
                    WATCHED_FLAG: '0' if reference else '1',
                    PARALLEL_FLAG: '1' if arm == 'on' and not reference else '0'}
        if self.watched:
            return {RETOUCH_FLAG: '0' if reference else '1',
                    WATCHED_FLAG: '1' if arm == 'on' and not reference else '0'}
        return {RETOUCH_FLAG: '1' if arm == 'on' else '0'}

    def reuse_worker(self, arm):
        return self.parallel or self.watched or arm == 'on'


RETOUCH_POLICY = RetouchPolicy('manifest-retouch', RETOUCH_SCHEMA)
WATCHED_POLICY = RetouchPolicy('watched-manifest-retouch', WATCHED_SCHEMA, True)
PARALLEL_POLICY = RetouchPolicy('typed-planning-retouch', PARALLEL_SCHEMA, parallel=True)


def retouch_policy(schema):
    for policy in (RETOUCH_POLICY, WATCHED_POLICY, PARALLEL_POLICY):
        if schema == policy.schema:
            return policy
    raise ValueError('native resident: unsupported retouch schema')


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError('native resident: ' + message)


def pinned_hashes():
    repo = Path(__file__).resolve().parent.parent
    names = subprocess.check_output(['git', 'ls-tree', '-rz', harness.PIN], cwd=repo)
    result = {}
    for entry in names.split(b'\0'):
        if entry:
            info, name = entry.split(b'\t', 1)
            oid = info.split()[2].decode()
            blob = subprocess.check_output(['git', 'cat-file', 'blob', oid], cwd=repo)
            result[os.fsdecode(name)] = hashlib.sha256(blob).hexdigest()
    require(len(result) == 63, 'pinned source must contain exactly 63 files')
    return result


def host_identity():
    cpu = Path('/proc/cpuinfo').read_text()
    models = sorted(set(re.findall(r'^model name\s*:\s*(.+)$', cpu, re.M)))
    return {'host': platform.node(), 'cpu': models, 'ncpu': os.cpu_count(),
            'machine': platform.machine(), 'kernel': platform.release(), 'execution': 'local native direct'}


def replay(root, expected_source):
    """Recompute gates and samples from records and raw logs, not summary booleans."""
    metadata = json.loads((root / 'metadata.json').read_text())
    rows = json.loads((root / 'records.json').read_text())
    require(metadata.get('pin') == harness.PIN, 'wrong source pin')
    require(metadata.get('source_before') == expected_source == metadata.get('source_after'), 'source63 hashes differ')
    for key in ('tools', 'toolchain'):
        require(metadata.get(key + '_before') and metadata[key + '_before'] == metadata.get(key + '_after'), key + ' changed')
    require(metadata.get('source_restored') is True and metadata.get('toolchain_links_valid') is True, 'source/tools not restored')
    require(not metadata.get('error') and not metadata.get('identity_error'), 'harness error')
    require(metadata.get('profiles') == harness.runtime.PROFILES, 'profile mismatch')
    require(metadata.get('store_scope') == 'unseeded private cold stores, no automatic seeding', 'seeded or unknown stores')
    require(set(metadata['tools_before']) in ({'reference', 'engine'}, {'reference', 'engine', 'proc-macro-server'}), 'missing or unexpected engine identities')
    for identity in metadata['tools_before'].values():
        require(isinstance(identity, dict) and Path(identity.get('path', '')).is_absolute()
                and re.fullmatch(r'[0-9a-f]{64}', identity.get('sha256', '')), 'malformed executable identity')
    files = metadata.get('toolchain_before', {}).get('files')
    require(isinstance(files, dict) and {'bin/cargo', 'bin/rustc', 'lib/rustlib/components',
            'lib/rustlib/multirust-channel-manifest.toml',
            'lib/rustlib/wasm32-wasip1/lib/self-contained/crt1-reactor.o'} <= set(files), 'missing full toolchain identity')
    require(any(name.startswith('lib/rustlib/src/rust/library/') for name in files), 'missing toolchain library sources')
    require(all(isinstance(value, dict) and re.fullmatch(r'[0-9a-f]{64}', value.get('sha256', ''))
                and 'link' in value for value in files.values()), 'malformed toolchain file identity')
    manifest = metadata['toolchain_before']['files'].get('lib/rustlib/multirust-channel-manifest.toml', {})
    require(manifest.get('sha256') == harness.TOOLCHAIN_MANIFEST, 'wrong toolchain manifest')
    original = (root / 'source.original').read_bytes()
    require(hashlib.sha256(original).hexdigest() == expected_source['src/summary.rs'], 'stale backup')
    samples = {}
    artifacts = ['metadata.json', 'records.json', 'summary.json', 'source.original']
    expected_tests = None
    for stack in ('reference', 'engine'):
        selected = [r for r in rows if r.get('stack') == stack]
        wanted = [('version', 0), ('prepare', 0), ('type-error', 0), ('type-recovery', 0),
                  ('assert-failure', 0), ('assert-recovery', 0)] + [('edit', n) for n in range(5)]
        require([(r.get('phase'), r.get('replicate')) for r in selected] == wanted, 'missing/duplicate/out-of-order phases: ' + stack)
        env = metadata.get('environments', {}).get(stack, {})
        require(env.get('BLAZE_BACKEND') == 'cranelift' and env.get('BLAZE_OPT') == '0', 'backend mismatch')
        require(all(env.get(k) == v for k, v in harness.runtime.PROFILES.items()), 'effective profile mismatch')
        for key in ('BLAZE_REMOTE', 'RA_JIT_REMOTE'):
            require(env.get(key) == 'off', 'remote execution not allowed')
        for key in ('BLAZE_PROC_MACRO_SERVER', 'RA_JIT_PROC_MACRO_SERVER'):
            server = metadata['tools_before'].get('proc-macro-server', {}).get('path', '0')
            require(env.get(key) == server, 'proc macro policy mismatch')
        values = []
        for row in selected:
            phase, rep = row['phase'], row['replicate']
            prefix = f'{stack}-{phase}-{rep}'
            logs = [prefix + '.' + suffix for suffix in ('stdout', 'stderr')]
            artifacts.extend(logs)
            text = '\n'.join((root / name).read_text(errors='replace') for name in logs)
            require(row.get('error') is None and row.get('timed_out') is False, 'process error/timeout')
            elapsed = row.get('elapsed_seconds')
            require(type(elapsed) in (int, float) and math.isfinite(elapsed) and elapsed > 0, 'invalid timing')
            tool = Path(metadata['tools_before'][stack]['path'])
            command = [str(tool), '--version'] if phase == 'version' else harness.command(tool, stack)
            require(row.get('command') == command, 'stale command identity')
            contents = original
            if phase == 'type-error':
                contents = harness.runtime.edit_bytes(original, kind='type-error')
            elif phase == 'assert-failure':
                contents = harness.runtime.edit_bytes(original, kind='test-failure')
            elif phase == 'edit':
                contents = harness.runtime.edit_bytes(original, rep)
            require(row.get('source_sha256') == hashlib.sha256(contents).hexdigest(), 'stale edit identity')
            parsed = harness.runtime.test_results(text)
            if phase == 'version':
                pattern = '^' + (re.escape(harness.REFERENCE_VERSION) + '$' if stack == 'reference' else r'jr-engine\s+\S+')
                require(row.get('status') == 0 and re.search(pattern, text, re.M), 'wrong executable version')
            elif phase == 'type-error':
                require(row.get('status') == 101 and 'E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text, 'missing real type failure')
            else:
                failing = phase == 'assert-failure'
                require(row.get('status') == (101 if failing else 0) and harness.valid_tests(parsed, expected_tests, failing), 'libtest parity failure')
                if expected_tests is None:
                    expected_tests = parsed
                if failing:
                    require('JR_RUNTIME_FAILURE' in text, 'missing assertion failure')
                if phase == 'edit':
                    require(re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*', text) == [f'JR_RUNTIME_EXECUTED_{rep + 1}'], 'missing/duplicate runtime marker')
                    values.append(elapsed)
        samples[stack] = {'samples_seconds': values, 'median_seconds': statistics.median(values)}
    require(all(r.get('stack') in ('reference', 'engine') for r in rows), 'unexpected stack')
    return metadata, samples, {name: digest(root / name) for name in artifacts}


def validate(result):
    if result.get('schema') in (RETOUCH_SCHEMA, WATCHED_SCHEMA, PARALLEL_SCHEMA):
        return validate_retouch(result)
    require(result.get('schema') == SCHEMA, 'unsupported schema')
    root = Path(result['artifacts_root'])
    metadata, samples, receipts = replay(root, pinned_hashes())
    require(receipts == result.get('artifacts_sha256'), 'raw artifact receipt mismatch')
    require(samples == result.get('measurements'), 'sample receipt mismatch')
    require(metadata == result.get('metadata'), 'metadata receipt mismatch')
    host = result.get('host')
    require(isinstance(host, dict) and all(host.get(k) for k in ('host', 'cpu', 'ncpu', 'machine', 'kernel', 'execution')), 'missing host identity')
    require(host['execution'] == 'local native direct', 'wrong execution mode')
    return metadata, samples


def cmd_resident(args):
    if getattr(args, 'mode', 'edit') == PARALLEL_POLICY.mode:
        return cmd_manifest_retouch(args, PARALLEL_POLICY)
    if getattr(args, 'mode', 'edit') == 'manifest-retouch':
        return cmd_manifest_retouch(args)
    if getattr(args, 'mode', 'edit') == WATCHED_POLICY.mode:
        return cmd_manifest_retouch(args, WATCHED_POLICY)
    args.blaze = None
    before_host = host_identity()
    status = harness.execute(args)
    require(status == 0, 'harness failed, raw evidence retained')
    require(host_identity() == before_host, 'host changed')
    metadata, samples, receipts = replay(args.output, pinned_hashes())
    result = {'schema': SCHEMA, 'artifacts_root': str(args.output),
              'artifacts_sha256': receipts, 'metadata': metadata, 'measurements': samples,
              'host': before_host, 'noise': 'unknown', 'scope': 'scoped diagnostic, not comprehensive or default promotion'}
    target = args.output / 'suite.json'
    target.write_text(json.dumps(result, indent=2) + '\n')
    print(target)


def compare(a, b, floor=10.0):
    if any(r.get('schema') in (RETOUCH_SCHEMA, WATCHED_SCHEMA, PARALLEL_SCHEMA) for r in (a, b)):
        return compare_retouch(a, b, floor)
    require(a.get('schema') == SCHEMA and b.get('schema') == SCHEMA, 'cannot mix native and non-native results')
    ma, sa = validate(a)
    mb, sb = validate(b)
    require(a['host'] == b['host'], 'host/cpu/execution differs')
    for key in ('source_before', 'toolchain_before', 'profiles', 'proc_macro_policy', 'store_scope'):
        require(ma[key] == mb[key], 'different ' + key)
    for key in ('reference', 'proc-macro-server'):
        require(ma['tools_before'].get(key) == mb['tools_before'].get(key), 'different source-built reference or server')
    require(math.isfinite(floor) and floor >= 0, 'invalid floor')
    old, new = sa['engine']['median_seconds'], sb['engine']['median_seconds']
    delta = (new / old - 1) * 100
    ra = old / sa['reference']['median_seconds']
    rb = new / sb['reference']['median_seconds']
    print('Native resident scoped diagnostic: descriptive/uncontrolled, noise unknown.')
    print(f'Engine median: {old:.6f}s -> {new:.6f}s, delta {new-old:+.6f}s ({delta:+.2f}%).')
    print(f'Reference-normalized engine/reference ratio: {ra:.6f} -> {rb:.6f}, relative delta {(rb/ra-1)*100:+.2f}%.')
    print(f'Absolute median delta {"exceeds" if abs(delta) > floor else "does not exceed"} requested {floor:g}% floor. No significance or speedup verdict.')
    print('Not a comprehensive workload or default-promotion gate.')


def retouch_fixture(original):
    """One fixed overlay, never a per-sample source edit or compile-time env! read."""
    anchor = b'fn shares_overlapping_time() {'
    require(original.count(anchor) == 1, 'runtime fixture anchor must occur exactly once')
    body = (b'\nlet nonce = std::env::var("JR_MANIFEST_RETOUCH_NONCE").unwrap();'
            b'\nassert_eq!(nonce.len(), 32);'
            b'\nprintln!("JR_MANIFEST_RETOUCH_{}", nonce);\n')
    return original.replace(anchor, anchor + body, 1)


def retouch_snapshot(path):
    require(not path.is_symlink() and path.is_file(), 'retouch input must be a regular non-symlink file')
    before = path.stat()
    contents = path.read_bytes()
    after = path.stat()
    require((before.st_size, before.st_mtime_ns, before.st_ino) ==
            (after.st_size, after.st_mtime_ns, after.st_ino), 'input changed while hashing')
    return {'sha256': hashlib.sha256(contents).hexdigest(), 'size': len(contents),
            'mtime_ns': after.st_mtime_ns}


def retouch_inputs(source, originals):
    """Untimed same-byte writes with an explicit, observed mtime change."""
    receipts = []
    for name in RETOUCH_FILES:
        path = source / name
        before = retouch_snapshot(path)
        require(path.read_bytes() == originals[name], 'retouch input differs from pin')
        atime = path.stat().st_atime_ns
        path.write_bytes(originals[name])
        os.utime(path, ns=(atime, max(time.time_ns(), before['mtime_ns'] + 1_000_000)))
        after = retouch_snapshot(path)
        require(before['sha256'] == after['sha256'] and before['size'] == after['size']
                and before['mtime_ns'] != after['mtime_ns'], 'same-byte retouch was not observed')
        receipts.append({'file': name, 'before': before, 'after': after})
    return receipts


def proc_identity(pid, engine):
    """PID plus kernel start tick and executable, outside the measured command."""
    proc = Path('/proc') / str(pid)
    first = (proc / 'stat').read_text()
    argv = (proc / 'cmdline').read_bytes()
    exe = os.readlink(proc / 'exe')
    hashed = digest(proc / 'exe')
    second = (proc / 'stat').read_text()
    require(first.rsplit(') ', 1)[1].split()[19] == second.rsplit(') ', 1)[1].split()[19],
            'worker PID changed while inspecting')
    require(argv == (proc / 'cmdline').read_bytes(), 'worker argv changed while inspecting')
    require(Path(exe) == engine.resolve(), 'worker executable is not the selected engine')
    return {'pid': pid, 'stat': second, 'exe': exe, 'exe_sha256': hashed, 'cmdline_hex': argv.hex()}


def worker_key(receipt, engine, home):
    require(isinstance(receipt, dict), 'missing worker identity')
    pid = receipt.get('pid')
    require(type(pid) is int and pid > 0, 'invalid worker PID')
    stat = receipt.get('stat', '')
    require(isinstance(stat, str) and stat.startswith(str(pid) + ' (') and ') ' in stat,
            'invalid worker stat')
    fields = stat.rsplit(') ', 1)[1].split()
    require(len(fields) >= 20 and fields[0] not in ('Z', 'X', 'x') and fields[19].isdigit(),
            'dead or unknown worker start identity')
    require(receipt.get('exe') == engine['path'] and receipt.get('exe_sha256') == engine['sha256'],
            'worker executable identity differs')
    raw = receipt.get('cmdline_hex')
    require(isinstance(raw, str) and re.fullmatch(r'(?:[0-9a-f]{2})+', raw), 'missing native worker argv')
    argv = bytes.fromhex(raw).split(b'\0')
    sockets = [argv[i + 1] for i, arg in enumerate(argv[:-1]) if arg == b'--socket']
    require(len(sockets) == 1, 'missing/ambiguous native worker socket argv')
    socket = Path(os.fsdecode(sockets[0]))
    require(socket.name == 'sock' and socket.is_relative_to(Path(home) / 'workers')
            and '..' not in socket.parts, 'worker socket escapes private namespace')
    return pid, fields[19]


def check_retouch_process(row, command):
    require(row.get('command') == command and row.get('status') == 0
            and row.get('error') is None and row.get('timed_out') is False,
            'process command/status/timeout failure')
    elapsed = row.get('elapsed_seconds')
    require(type(elapsed) in (int, float) and math.isfinite(elapsed) and elapsed > 0,
            'invalid elapsed time')


def check_retouch_environment(env, metadata, arm, policy=RETOUCH_POLICY, reference=False):
    require(all(env.get(key) == value for key, value in policy.flags(arm, reference).items()),
            'wrong exact flag receipt')
    require(RETOUCH_NONCE not in env, 'runtime nonce leaked into persistent environment')
    require(env.get('BLAZE_BACKEND') == 'cranelift' and env.get('BLAZE_OPT') == '0', 'backend mismatch')
    require(all(env.get(k) == v for k, v in harness.runtime.PROFILES.items()), 'profile mismatch')
    server = metadata['tools_before'].get('proc-macro-server', {}).get('path', '0')
    for key in ('BLAZE_REMOTE', 'RA_JIT_REMOTE'):
        require(env.get(key) == 'off', 'remote execution not allowed')
    for key in ('BLAZE_PROC_MACRO_SERVER', 'RA_JIT_PROC_MACRO_SERVER'):
        require(env.get(key) == server, 'proc macro policy differs')


def check_parallel_activation(stdout, stderr, flag, phase):
    """A fixed actual-batch stderr receipt, never an eligibility/summary boolean."""
    prefix = 'ra-jit: native-typed-planning-'
    count = stderr.splitlines().count(PARALLEL_MARKER)
    require(prefix not in stdout and stderr.count(prefix) == count
            and stderr.count(PARALLEL_MARKER + '\n') == count,
            'wrong parallel batch marker location/format')
    expected = 0 if flag == '0' or phase == 'version' else 1 if phase == 'retouch' else None
    require(count == expected if expected is not None else count <= 1,
            'wrong parallel batch marker count')


def replay_retouch(root, expected_source, policy=RETOUCH_POLICY):
    """Strict new schema. Never reinterpret the legacy unique-edit measurements."""
    metadata = json.loads((root / 'metadata.json').read_text())
    rows = json.loads((root / 'records.json').read_text())
    require(metadata.get('schema') == policy.schema and metadata.get('pin') == harness.PIN,
            'wrong retouch schema/pin')
    if policy.watched:
        require(metadata.get('mode') == policy.mode and metadata.get('factor') == WATCHED_FLAG,
                'wrong watched retouch mode/factor')
    if policy.parallel:
        require(metadata.get('mode') == policy.mode and metadata.get('factor') == PARALLEL_FLAG,
                'wrong parallel retouch mode/factor')
        require(metadata.get('rules') == PARALLEL_RULES
                and re.fullmatch('[0-9a-f]{32}', metadata.get('run_id', '')),
                'wrong parallel rules/run identity')
    require(metadata.get('order') == list(RETOUCH_ORDER) and metadata.get('samples') == 5,
            'wrong ABBA order/sample count')
    require(len(expected_source) == 63 and metadata.get('source_before') == expected_source
            == metadata.get('source_after'), 'pristine source63 hashes differ')
    require(metadata.get('source_restored') is True and metadata.get('toolchain_links_valid') is True,
            'source/toolchain links not restored')
    require(metadata.get('host_before') == metadata.get('host_after') and metadata.get('host_before'),
            'host changed')
    require(not metadata.get('error') and not metadata.get('identity_error')
            and not metadata.get('cleanup_error') and metadata.get('status') == 'complete', 'retouch harness error')
    require(metadata.get('store_scope') == 'unseeded private cold stores, no automatic seeding',
            'seeded or unknown stores')
    require(metadata.get('profiles') == harness.runtime.PROFILES, 'effective profiles differ')
    for kind in ('tools', 'toolchain'):
        require(metadata.get(kind + '_before') and metadata[kind + '_before'] == metadata.get(kind + '_after'),
                kind + ' identity changed')
    tools = metadata['tools_before']
    require(set(tools) in ({'reference', 'engine'}, {'reference', 'engine', 'proc-macro-server'}),
            'wrong tool identities')
    for tool in tools.values():
        require(Path(tool.get('path', '')).is_absolute() and re.fullmatch(r'[0-9a-f]{64}', tool.get('sha256', '')),
                'malformed tool identity')
    files = metadata['toolchain_before'].get('files', {})
    require({'bin/cargo', 'bin/rustc', 'lib/rustlib/components',
             'lib/rustlib/multirust-channel-manifest.toml',
             'lib/rustlib/wasm32-wasip1/lib/self-contained/crt1-reactor.o'} <= set(files)
            and any(name.startswith('lib/rustlib/src/rust/library/') for name in files),
            'incomplete toolchain identity')
    require(all(isinstance(value, dict) and re.fullmatch(r'[0-9a-f]{64}', value.get('sha256', ''))
                and 'link' in value for value in files.values()), 'malformed toolchain identity')
    require(files['lib/rustlib/multirust-channel-manifest.toml']['sha256'] == harness.TOOLCHAIN_MANIFEST,
            'wrong toolchain manifest')
    original = (root / 'source.original').read_bytes()
    require(hashlib.sha256(original).hexdigest() == expected_source['src/summary.rs'], 'stale source backup')
    overlay = retouch_fixture(original)
    require((root / 'source.fixture').read_bytes() == overlay, 'runtime fixture differs')
    overlaid = dict(expected_source, **{'src/summary.rs': hashlib.sha256(overlay).hexdigest()})
    require(metadata.get('fixture_source') == overlaid, 'fixture source manifest differs')
    for name in RETOUCH_FILES:
        require(digest(root / (name + '.original')) == expected_source[name], 'stale manifest/lock backup')
    phases = ('version', 'prepare', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery')
    wanted = [('reference', phase, 0) for phase in phases]
    for segment in range(4):
        wanted.extend((f'segment-{segment}', phase, 0) for phase in phases)
        wanted.append((f'segment-{segment}', 'warmup', 0))
        wanted.extend((f'segment-{segment}', 'retouch', n) for n in range(5))
    require([(r.get('group'), r.get('phase'), r.get('replicate')) for r in rows] == wanted,
            'missing/duplicate/out-of-order phases')
    artifacts = ['metadata.json', 'records.json', 'source.original', 'source.fixture'] + [n + '.original' for n in RETOUCH_FILES]
    samples = {'off': [], 'on': []}
    nonces, worker = set(), {}
    expected_tests = None
    states = metadata.get('environments', {})
    require(set(states) == {'reference', *(f'segment-{i}' for i in range(4))}, 'wrong private state set')
    state_roots = [states[group].get('RA_JIT_HOME') for group in states]
    require(all(isinstance(p, str) and Path(p).is_absolute() and Path(p).is_relative_to(root) for p in state_roots)
            and len(set(state_roots)) == 5, 'private states alias or escape output')
    for key in ('CARGO_HOME', 'CARGO_TARGET_DIR', 'RA_JIT_CACHE', 'BLAZE_CACHE', 'RUSTUP_HOME'):
        places = [states[group].get(key) for group in states]
        require(all(isinstance(p, str) and Path(p).is_absolute() and Path(p).is_relative_to(root) for p in places)
                and len(set(places)) == 5, 'private cache/target states alias or escape output')
    stops = metadata.get('stops', [])
    require([r.get('group') for r in stops] == ['reference'] + [f'segment-{i}' for i in range(4)],
            'missing/duplicate private-state stop')
    for receipt in stops:
        group = receipt['group']
        tool = tools['reference' if group == 'reference' else 'engine']
        check_retouch_process(receipt.get('process', {}), [tool['path'], 'stop'])
        artifacts.extend([group + '-stop.stdout', group + '-stop.stderr'])
    for index, row in enumerate(rows):
        group, phase = row['group'], row['phase']
        segment = None if group == 'reference' else int(group[-1])
        arm = 'off' if segment is None else RETOUCH_ORDER[segment]
        env = states[group]
        check_retouch_environment(env, metadata, arm, policy, segment is None)
        require(row.get('flag') == env[RETOUCH_FLAG], 'missing per-call exact flag receipt')
        if policy.watched or policy.parallel:
            require(row.get('flags') == policy.flags(arm, segment is None), 'wrong per-call flags receipt')
        if policy.parallel:
            require(row.get('schema') == policy.schema and row.get('run_id') == metadata['run_id']
                    and row.get('rules') == PARALLEL_RULES, 'cross-schema/run/rules row mixture')
        tool = tools['reference' if segment is None else 'engine']
        command = [tool['path'], '--version'] if phase == 'version' else harness.command(Path(tool['path']), 'reference' if segment is None else 'engine')
        prefix = f'call-{index:03}'
        logs = [prefix + '.stdout', prefix + '.stderr']
        artifacts.extend(logs)
        text = '\n'.join((root / name).read_text(errors='replace') for name in logs)
        if policy.parallel:
            check_parallel_activation((root / logs[0]).read_text(errors='replace'),
                                      (root / logs[1]).read_text(errors='replace'), env[PARALLEL_FLAG], phase)
        if env[RETOUCH_FLAG] == '0' or phase == 'version':
            require(RETOUCH_MARKER not in text, 'unexpected opt-in entry marker')
        require(row.get('command') == command and row.get('error') is None and row.get('timed_out') is False,
                'wrong command/error/timeout')
        elapsed = row.get('elapsed_seconds')
        require(type(elapsed) in (int, float) and math.isfinite(elapsed) and elapsed > 0, 'invalid elapsed time')
        contents = original
        if phase == 'type-error':
            contents = harness.runtime.edit_bytes(original, kind='type-error')
        elif phase == 'assert-failure':
            contents = harness.runtime.edit_bytes(original, kind='test-failure')
        elif phase in ('warmup', 'retouch'):
            contents = overlay
        inputs = dict(expected_source, **{'src/summary.rs': hashlib.sha256(contents).hexdigest()})
        require(row.get('source_before') == inputs == row.get('source_after'), 'per-call source hashes differ')
        parsed = harness.runtime.test_results(text)
        if phase == 'version':
            pattern = '^' + (re.escape(harness.REFERENCE_VERSION) + '$' if segment is None else r'jr-engine\s+\S+')
            require(row.get('status') == 0 and re.search(pattern, text, re.M), 'wrong native executable version')
        elif phase == 'type-error':
            require(row.get('status') == 101 and 'E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text,
                    'missing real type failure')
        else:
            failing = phase == 'assert-failure'
            require(row.get('status') == (101 if failing else 0)
                    and harness.valid_tests(parsed, expected_tests, failing), 'libtest semantic parity failure')
            if expected_tests is None:
                expected_tests = parsed
            if failing:
                require('JR_RUNTIME_FAILURE' in text, 'missing assertion failure')
        if phase in ('warmup', 'retouch'):
            nonce = row.get('nonce')
            require(isinstance(nonce, str) and re.fullmatch('[0-9a-f]{32}', nonce) and nonce not in nonces,
                    'missing/stale/duplicate runtime nonce')
            nonces.add(nonce)
            require(row.get('nonce_environment') == {RETOUCH_NONCE: nonce}, 'wrong runtime nonce environment receipt')
            require(re.findall(r'JR_MANIFEST_RETOUCH_[^\s]+', text) == ['JR_MANIFEST_RETOUCH_' + nonce],
                    'missing/duplicate/stale reachable runtime nonce')
            require(text.splitlines().count(RETOUCH_MARKER) == int(env[RETOUCH_FLAG] == '1'), 'wrong opt-in entry marker count')
            status_prefix = prefix + '-status'
            artifacts.extend([status_prefix + '.stdout', status_prefix + '.stderr'])
            status_text = '\n'.join((root / (status_prefix + suffix)).read_text(errors='replace') for suffix in ('.stdout', '.stderr'))
            check_retouch_process(row.get('worker_status', {}), [tool['path'], 'status'])
            pids = re.findall(r'worker: pid (\d+)', status_text)
            require(len(pids) == 1, 'missing/ambiguous native worker PID')
            identity = worker_key(row.get('worker'), tool, env['RA_JIT_HOME'])
            if policy.parallel:
                identity += (row['worker']['cmdline_hex'],)
            require(identity[0] == int(pids[0]), 'status and proc PID differ')
            if phase == 'retouch':
                require((identity == worker[group]) == policy.reuse_worker(arm), 'worker PID/starttime did not match arm behavior')
                touches = row.get('touches', [])
                require([r.get('file') for r in touches] == list(RETOUCH_FILES), 'wrong retouched file set')
                for touch in touches:
                    before, after = touch.get('before', {}), touch.get('after', {})
                    name = touch['file']
                    size = (root / (name + '.original')).stat().st_size
                    require(before.get('sha256') == after.get('sha256') == expected_source[name]
                            and before.get('size') == after.get('size') == size,
                            'retouch changed bytes/size')
                    require(type(before.get('mtime_ns')) is int and type(after.get('mtime_ns')) is int
                            and after['mtime_ns'] > before['mtime_ns'], 'retouch mtime did not increase')
                samples[arm].append(elapsed)
            else:
                require(row.get('touches') == [], 'warmup must not retouch inputs')
            worker[group] = identity
        else:
            require(row.get('nonce') is None and row.get('touches') == []
                    and row.get('nonce_environment') == {}, 'gate must not contain measured edits')
    require(all(len(v) == 10 for v in samples.values()), 'missing arm samples')
    measurements = {arm: {'samples_seconds': values, 'median_seconds': statistics.median(values)} for arm, values in samples.items()}
    return metadata, measurements, {name: digest(root / name) for name in artifacts}


def validate_retouch(result):
    policy = retouch_policy(result.get('schema'))
    require(result.get('arm') in ('off', 'on'), 'wrong retouch result arm/schema')
    metadata, samples, receipts = replay_retouch(Path(result['artifacts_root']), pinned_hashes(), policy)
    require(result.get('metadata') == metadata and result.get('artifacts_sha256') == receipts,
            'raw retouch artifact receipt mismatch')
    require(result.get('measurements') == samples and result.get('host') == metadata['host_before'],
            'retouch sample/host receipt mismatch')
    return metadata, samples


def check_retouch_output_length(output):
    # Native worker sockets add a long suffix. No silent socket-root override.
    require(len(os.fsencode(output)) <= 30, 'retouch output must be short (<=30 bytes) for native sockets')


def cmd_manifest_retouch(args, policy=RETOUCH_POLICY):
    """Native commands execute only when root explicitly requests this CLI mode."""
    require(args.samples == 5 and math.isfinite(args.timeout) and args.timeout > 0,
            'exactly five samples and a finite positive timeout required')
    require(not args.output.exists() and not args.output.is_relative_to(args.source)
            and not args.source.is_relative_to(args.output), 'output must be fresh and disjoint from source')
    check_retouch_output_length(args.output)
    harness.pinned_source(args.source)
    expected_source = pinned_hashes()
    require(harness.source_hashes(args.source) == expected_source, 'source63 pin mismatch')
    tools = {'reference': args.reference_engine, 'engine': args.engine}
    if args.proc_macro_server:
        tools['proc-macro-server'] = args.proc_macro_server
    for tool in tools.values():
        require(tool.is_absolute() and not tool.is_symlink() and os.access(tool, os.X_OK),
                'selected tools must be absolute native executable files')
        with tool.open('rb') as stream:
            require(stream.read(4) == b'\x7fELF', 'native ELF tools required, no wrappers')
    args.output.mkdir(mode=0o700, parents=True)
    original = (args.source / 'src/summary.rs').read_bytes()
    overlay = retouch_fixture(original)
    originals = {name: (args.source / name).read_bytes() for name in RETOUCH_FILES}
    times = {name: ((args.source / name).stat().st_atime_ns, (args.source / name).stat().st_mtime_ns)
             for name in (*RETOUCH_FILES, 'src/summary.rs')}
    (args.output / 'source.original').write_bytes(original)
    (args.output / 'source.fixture').write_bytes(overlay)
    for name, data in originals.items():
        (args.output / (name + '.original')).write_bytes(data)
    identify = lambda: {name: {'path': str(tool), 'sha256': digest(tool)} for name, tool in tools.items()}
    metadata = {'schema': policy.schema, 'pin': harness.PIN, 'order': list(RETOUCH_ORDER),
                'samples': 5, 'source_before': expected_source,
                'fixture_source': dict(expected_source, **{'src/summary.rs': hashlib.sha256(overlay).hexdigest()}),
                'tools_before': identify(), 'toolchain_before': harness.toolchain_identity(args.toolchain),
                'host_before': host_identity(), 'profiles': harness.runtime.PROFILES,
                'store_scope': 'unseeded private cold stores, no automatic seeding',
                'scope': 'combined same-byte manifest+lock retouch only, descriptive, no default promotion',
                'environments': {}, 'toolchain_links': {}, 'status': 'incomplete'}
    if policy.watched:
        metadata.update(mode=policy.mode, factor=WATCHED_FLAG,
                        scope='WATCHED manifest+lock factor, whole-generation fixed ON, descriptive, no default promotion')
    if policy.parallel:
        metadata.update(mode=policy.mode, factor=PARALLEL_FLAG,
                        run_id=uuid.uuid4().hex, rules=PARALLEL_RULES,
                        scope='noop/clientplanning typed planning factor, combined same-byte manifest+lock retouch, '
                              'whole-generation+watcher fixed ON, descriptive, no default promotion')
    rows, environments, stops = [], {}, []
    expected_tests = None

    def save():
        (args.output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
        (args.output / 'records.json').write_text(json.dumps(rows, indent=2) + '\n')

    def invoke(group, phase, replicate, contents, touches=None):
        nonlocal expected_tests
        env = dict(environments[group])
        segment = None if group == 'reference' else int(group[-1])
        stack = 'reference' if segment is None else 'engine'
        tool = tools[stack]
        pinned = args.source / 'src/summary.rs'
        # No measured-call write to Rust sources, even with identical bytes.
        if pinned.read_bytes() != contents:
            require(phase not in ('retouch',), 'measured Rust input changed')
            pinned.write_bytes(contents)
        nonce = uuid.uuid4().hex if phase in ('warmup', 'retouch') else None
        if nonce:
            env[RETOUCH_NONCE] = nonce
        before = harness.source_hashes(args.source)
        require(identify() == metadata['tools_before'], 'tool identity changed before call')
        command = [str(tool), '--version'] if phase == 'version' else harness.command(tool, stack)
        prefix = args.output / f'call-{len(rows):03}'
        row = {'group': group, 'phase': phase, 'replicate': replicate,
               'flag': env[RETOUCH_FLAG], 'nonce': nonce,
               'nonce_environment': {RETOUCH_NONCE: nonce} if nonce else {},
               'touches': touches or [], 'source_before': before}
        if policy.watched or policy.parallel:
            row['flags'] = {key: env[key] for key in policy.flags('off')}
        if policy.parallel:
            row.update(schema=policy.schema, run_id=metadata['run_id'], rules=PARALLEL_RULES)
        rows.append(row)
        save()
        row.update(harness.runtime.run_process(command, args.source, env, prefix, args.timeout))
        row['source_after'] = harness.source_hashes(args.source)
        require(row['source_after'] == before and identify() == metadata['tools_before'],
                'inputs/tools changed during command')
        text = '\n'.join(prefix.with_suffix(suffix).read_text(errors='replace') for suffix in ('.stdout', '.stderr'))
        if policy.parallel:
            check_parallel_activation(prefix.with_suffix('.stdout').read_text(errors='replace'),
                                      prefix.with_suffix('.stderr').read_text(errors='replace'), env[PARALLEL_FLAG], phase)
        require(row.get('error') is None and row.get('timed_out') is False, 'native process failed/timed out')
        if phase == 'version':
            version = re.escape(harness.REFERENCE_VERSION) + '$' if stack == 'reference' else r'jr-engine\s+\S+'
            require(row['status'] == 0 and re.search('^' + version, text, re.M), 'native version mismatch')
        elif phase == 'type-error':
            require(row['status'] == 101 and 'E0308' in text and 'JR_RUNTIME_TYPE_ERROR' in text,
                    'original semantic type-error gate failed')
        else:
            failing = phase == 'assert-failure'
            parsed = harness.runtime.test_results(text)
            require(row['status'] == (101 if failing else 0) and harness.valid_tests(parsed, expected_tests, failing),
                    'original semantic libtest parity failed')
            if expected_tests is None:
                expected_tests = parsed
            if failing:
                require('JR_RUNTIME_FAILURE' in text, 'original assertion-failure gate failed')
        if nonce:
            require(re.findall(r'JR_MANIFEST_RETOUCH_[^\s]+', text) == ['JR_MANIFEST_RETOUCH_' + nonce],
                    'reachable runtime nonce failed')
            status_prefix = prefix.with_name(prefix.name + '-status')
            row['worker_status'] = harness.runtime.run_process([str(tool), 'status'], args.source, env,
                                                               status_prefix, args.timeout)
            check_retouch_process(row['worker_status'], [str(tool), 'status'])
            status_text = '\n'.join(status_prefix.with_suffix(suffix).read_text(errors='replace') for suffix in ('.stdout', '.stderr'))
            pids = re.findall(r'worker: pid (\d+)', status_text)
            require(len(pids) == 1, 'expected exactly one native worker')
            row['worker'] = proc_identity(int(pids[0]), tool)
            worker_key(row['worker'], metadata['tools_before']['engine'], env['RA_JIT_HOME'])
        save()

    def stop(group):
        env = environments[group]
        tool = tools['reference' if group == 'reference' else 'engine']
        prefix = args.output / (group + '-stop')
        receipt = harness.runtime.run_process([str(tool), 'stop'], args.source, env, prefix, args.timeout)
        stops.append({'group': group, 'process': receipt})
        metadata['stops'] = stops
        check_retouch_process(receipt, [str(tool), 'stop'])

    try:
        for group, arm in [('reference', 'off')] + [(f'segment-{i}', arm) for i, arm in enumerate(RETOUCH_ORDER)]:
            (args.source / 'src/summary.rs').write_bytes(original)
            envroot = args.output / ('r' if group == 'reference' else 's' + group[-1])
            stack = 'reference' if group == 'reference' else 'engine'
            env = harness.environment(envroot, stack, args.toolchain, args.proc_macro_server)
            env.update(policy.flags(arm, group == 'reference'))
            # Prevent a parent runtime nonce from leaking into pristine semantic gates.
            env.pop(RETOUCH_NONCE, None)
            environments[group] = env
            keys = ('HOME', 'JR_ENGINE_HOME', 'RA_JIT_HOME', 'RA_JIT_CACHE', 'CARGO_HOME', 'CARGO_TARGET_DIR',
                    'BLAZE_HOME', 'BLAZE_CACHE', 'BLAZE_BACKEND', 'BLAZE_OPT', 'RUSTUP_HOME', 'BLAZE_REMOTE',
                    'RA_JIT_REMOTE', 'BLAZE_PROC_MACRO_SERVER', 'RA_JIT_PROC_MACRO_SERVER', RETOUCH_FLAG)
            if policy.watched:
                keys += (WATCHED_FLAG,)
            if policy.parallel:
                keys += (WATCHED_FLAG, PARALLEL_FLAG)
            metadata['environments'][group] = {key: env[key] for key in keys if key in env}
            metadata['environments'][group].update(harness.runtime.PROFILES)
            metadata['toolchain_links'][group] = str(Path(env['BLAZE_HOME']) / 'toolchains' / args.toolchain.name)
            for phase in ('version', 'prepare', 'type-error', 'type-recovery', 'assert-failure', 'assert-recovery'):
                contents = (harness.runtime.edit_bytes(original, kind=phase) if phase in ('type-error',) else
                            harness.runtime.edit_bytes(original, kind='test-failure') if phase == 'assert-failure' else original)
                invoke(group, phase, 0, contents)
            if group != 'reference':
                invoke(group, 'warmup', 0, overlay)
                for replicate in range(5):
                    touches = retouch_inputs(args.source, originals)
                    invoke(group, 'retouch', replicate, overlay, touches)
            stop(group)
            save()
        metadata['status'] = 'complete'
    except BaseException as exc:
        metadata['error'] = repr(exc)
        raise
    finally:
        (args.source / 'src/summary.rs').write_bytes(original)
        for name, data in originals.items():
            (args.source / name).write_bytes(data)
        for name, timestamps in times.items():
            os.utime(args.source / name, ns=timestamps)
        # Stop only private native states owned by this invocation, also on failure.
        stopped = {item['group'] for item in stops}
        for group in environments:
            if group not in stopped:
                try:
                    stop(group)
                except Exception as exc:
                    metadata['cleanup_error'] = repr(exc)
        try:
            metadata.update(source_after=harness.source_hashes(args.source), tools_after=identify(),
                            toolchain_after=harness.toolchain_identity(args.toolchain), host_after=host_identity())
            metadata['source_restored'] = metadata['source_after'] == expected_source
            metadata['toolchain_links_valid'] = all(Path(link).is_symlink() and Path(link).resolve() == args.toolchain
                                                    for link in metadata['toolchain_links'].values())
        except Exception as exc:
            metadata['identity_error'] = repr(exc)
        save()
    metadata, samples, receipts = replay_retouch(args.output, expected_source, policy)
    for arm in ('off', 'on'):
        result = {'schema': policy.schema, 'artifacts_root': str(args.output), 'arm': arm,
                  'metadata': metadata, 'measurements': samples, 'artifacts_sha256': receipts,
                  'host': metadata['host_before'], 'noise': 'unknown', 'scope': metadata['scope']}
        target = args.output / (arm + '.json')
        target.write_text(json.dumps(result, indent=2) + '\n')
        print(target)


def compare_retouch(a, b, floor):
    require(a.get('schema') == b.get('schema') and a.get('schema') in (RETOUCH_SCHEMA, WATCHED_SCHEMA, PARALLEL_SCHEMA),
            'cannot mix retouch and edit schemas or retouch modes')
    policy = retouch_policy(a['schema'])
    ma, sa = validate_retouch(a)
    mb, sb = validate_retouch(b)
    require(a['arm'] == 'off' and b['arm'] == 'on', 'retouch compare requires OFF then ON')
    require(a['artifacts_sha256'] == b['artifacts_sha256'] and ma == mb and sa == sb,
            'retouch compare requires the same frozen same-engine ABBA run')
    require(math.isfinite(floor) and floor >= 0, 'invalid floor')
    old, new = sa['off']['median_seconds'], sb['on']['median_seconds']
    delta = (new / old - 1) * 100
    if policy.parallel:
        print('Native resident noop/clientplanning typed planning factor, whole-generation+watcher fixed ON.')
        print('Typed planning flag explicitly 0/1; reference all three flags 0. All first-touch samples retained.')
    elif policy.watched:
        print('Native resident WATCHED manifest+lock factor scoped diagnostic, whole-generation fixed ON.')
        print('Watcher flag explicitly 0/1; reference both flags 0. All first-touch samples retained.')
    else:
        print('Native resident combined same-byte manifest+lock retouch scoped diagnostic.')
    print('Same candidate ELF, OFF/ON/ON/OFF; 5 samples per segment, 10 per arm.')
    print(f'OFF median: {old:.6f}s -> ON median: {new:.6f}s, delta {new-old:+.6f}s ({delta:+.2f}%).')
    print(f'Absolute median delta {"exceeds" if abs(delta) > floor else "does not exceed"} requested {floor:g}% floor.')
    print('Descriptive/uncontrolled, noise unknown. No significance or general speedup verdict.')
    print('Combined retouch only, not separate manifest/lock attribution or default promotion.')


def add_parser(sub):
    parser = sub.add_parser('resident', help=__doc__)
    for name in ('reference-engine', 'engine', 'source', 'output', 'toolchain'):
        parser.add_argument('--' + name, type=harness.absolute, required=True)
    parser.add_argument('--proc-macro-server', type=harness.absolute)
    parser.add_argument('--samples', type=int, choices=(5,), default=5)
    parser.add_argument('--timeout', type=float, default=360)
    parser.add_argument('--mode', choices=('edit', 'manifest-retouch', WATCHED_POLICY.mode, PARALLEL_POLICY.mode), default='edit',
                        help='manifest-retouch runs fixed same-engine OFF/ON/ON/OFF, five samples each')
    parser.set_defaults(fn=cmd_resident)
