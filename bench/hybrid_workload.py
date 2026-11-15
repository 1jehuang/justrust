#!/usr/bin/env python3
"""Real-process hybrid correctness matrix. No timing or promotion claims from mocks.

Use fresh --output on the machine owning the supplied absolute executable paths.
The generated fixture has a local dependency, proc macro, build script, and an
append-only test side effect. Setup and edits happen outside timed subprocesses.
"""
import argparse
import fcntl
import hashlib
import json
import os
import math
import re
from pathlib import Path
import signal
import subprocess
import time


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def absolute(value):
    path = Path(value).expanduser()
    if not path.is_absolute() or '..' in path.parts:
        raise argparse.ArgumentTypeError('expected absolute path without ..')
    return path


def environment_receipt(env):
    allowed = {'PATH', 'RUSTC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'RUSTFLAGS',
               'RUSTUP_TOOLCHAIN', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_HOME', 'RUSTUP_HOME',
               'CARGO_TARGET_DIR', 'JUSTRUST_REMOTE', 'JUSTRUST_NATIVE_ENGINE',
               'JUSTRUST_HYBRID_TOOLCHAIN', 'JR_ENGINE_HOME', 'HYBRID_SIDE_EFFECT',
               'JUSTRUST_HYBRID_PREWARM',
               'HYBRID_PROGRAM_SIDE_EFFECT'}
    return {key: value for key, value in sorted(env.items())
            if key in allowed or key.startswith('CARGO_PROFILE_')}


def put(root, name, text):
    path = root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists() or path.read_text() != text:
        path.write_text(text)


def fixture(root, value=7):
    root.mkdir(parents=True, exist_ok=True)
    put(root, 'Cargo.toml', '[package]\nname="hybrid-probe"\nversion="0.1.0"\nedition="2021"\nbuild="build.rs"\n[dependencies]\nprobe-dep={path="dep"}\nprobe-macro={path="macro"}\n[workspace]\n')
    manifest = root / 'Cargo.toml'
    manifest.write_text(manifest.read_text() + '[features]\ndefault=[]\nchanged=[]\n')
    put(root, 'dep/Cargo.toml', '[package]\nname="probe-dep"\nversion="0.1.0"\nedition="2021"\n')
    put(root, 'dep/src/lib.rs', f'pub fn value() -> u32 {{ {value} }}\n')
    put(root, 'macro/Cargo.toml', '[package]\nname="probe-macro"\nversion="0.1.0"\nedition="2021"\n[lib]\nproc-macro=true\n')
    put(root, 'macro/src/lib.rs', 'extern crate proc_macro;\n#[proc_macro]\npub fn expected(_: proc_macro::TokenStream) -> proc_macro::TokenStream { "7u32".parse().unwrap() }\n')
    put(root, 'build.rs', 'fn main() { println!("cargo:rustc-env=PROBE_BUILD=ready"); println!("cargo:rerun-if-changed=build.rs"); }\n')
    put(root, '.cargo/config.toml', '[env]\nPROBE_CONFIG={value="one",force=true}\n')
    source(root, value)


def source(root, expected=7, nonce=0, broken=False, config='one', macro=7, build='ready', feature=False):
    put(root, 'src/lib.rs', ('this is intentionally invalid Rust\n' if broken else f'''pub fn value() -> u32 {{ probe_dep::value() }}
#[test]
fn hybrid_side_effect() {{
    use std::io::Write;
    let path = std::env::var("HYBRID_SIDE_EFFECT").unwrap();
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    writeln!(file, "nonce={nonce}").unwrap();
    assert_eq!(env!("PROBE_BUILD"), "{build}");
    assert_eq!(env!("PROBE_CONFIG"), "{config}");
    println!("HYBRID_OBSERVED nonce={nonce} dependency={{}} config={{}}", value(), env!("PROBE_CONFIG"));
    assert_eq!(probe_macro::expected!(), {macro});
    assert_eq!(cfg!(feature="changed"), {str(feature).lower()});
    assert_eq!(value(), {expected});
}}
'''))


def side_lines(path):
    return path.read_text().splitlines() if path.exists() else []


def pss_kib(pid):
    """Best effort Linux process-tree PSS, sampled not a claimed exact peak."""
    parents = {}
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            status = (entry / 'status').read_text()
            parents[int(entry.name)] = int(next(x.split()[1] for x in status.splitlines() if x.startswith('PPid:')))
        except (OSError, ValueError, StopIteration):
            continue
    owned = {pid}
    while True:
        new = {child for child, parent in parents.items() if parent in owned}
        if new <= owned:
            break
        owned |= new
    total = 0
    for child in owned:
        try:
            total += sum(int(x.split()[1]) for x in Path(f'/proc/{child}/smaps_rollup').read_text().splitlines() if x.startswith('Pss:'))
        except (OSError, ValueError):
            pass
    return total


def namespace_pss(home):
    """Outside-timing snapshot of same-user processes inheriting this exact home.

    Resident daemons may detach from the launcher tree. This is not peak memory
    and inaccessible or vanished processes are listed rather than estimated.
    """
    marker = b'JR_ENGINE_HOME=' + os.fsencode(home)
    processes, unavailable, nice, starts = {}, [], {}, {}
    for path in Path('/proc').iterdir():
        if not path.name.isdigit():
            continue
        try:
            if path.stat().st_uid != os.geteuid():
                continue
            if marker not in (path / 'environ').read_bytes().split(b'\0'):
                continue
        except (OSError, ValueError):
            if path.exists():
                unavailable.append(int(path.name))
            continue
        try:
            processes[path.name] = sum(int(line.split()[1]) for line in
                                      (path / 'smaps_rollup').read_text().splitlines()
                                      if line.startswith('Pss:'))
        except (OSError, ValueError):
            processes[path.name] = None
        try:
            # The command name may contain spaces and parentheses. Fields after
            # its final ')' begin at stat field 3. Nice is field 19.
            fields = (path / 'stat').read_text().rsplit(')', 1)[1].split()
            nice[path.name] = int(fields[16])
            starts[path.name] = fields[19]
        except (OSError, ValueError, IndexError):
            nice[path.name] = None
    return {'pss_kib': sum(value for value in processes.values() if value is not None), 'process_pss_kib': processes,
            'process_nice': nice,
            'process_start_ticks': starts,
            'unavailable_pids': unavailable, 'scope': 'outside-timing exact JR_ENGINE_HOME environment match, not peak'}


def background_offset(home):
    try:
        return (Path(home) / 'prewarm.log').stat().st_size
    except FileNotFoundError:
        return 0


def preparation_active(home):
    if not home:
        return False
    try:
        fd = os.open(Path(home) / 'prewarm.lock', os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    except FileNotFoundError:
        return False
    except OSError:
        return None
    try:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return True
        except OSError:
            return None
        fcntl.flock(fd, fcntl.LOCK_UN)
        return False
    finally:
        os.close(fd)


def wait_background(home, offset, side, expected, timeout, pattern, evidence, extra_effects=()):
    """Wait outside foreground timing, and require NEW successful preparation output."""
    started = time.monotonic()
    raw = b''
    ready = False
    protected = ((side, expected), *extra_effects)
    def unchanged():
        return all(side_lines(path) == lines for path, lines in protected)
    while True:
        try:
            with (Path(home) / 'prewarm.log').open('rb') as stream:
                stream.seek(offset)
                raw = stream.read()
        except FileNotFoundError:
            pass
        if not unchanged():
            break
        if re.search(pattern, raw.decode(errors='replace')):
            ready = True
            break
        if time.monotonic() - started >= timeout:
            break
        time.sleep(0.01)
    evidence.write_bytes(raw)
    finished = time.monotonic()
    return {'ok': ready and unchanged(),
            'wait_seconds_outside_foreground_timing': finished - started,
            'ready_observed_monotonic': finished if ready else None,
            'timed_out': not ready and unchanged() and finished - started >= timeout,
            'readiness_pattern': pattern, 'readiness_verified': ready,
            'side_effects_unchanged': unchanged(),
            'log': str(evidence), 'log_sha256': digest(evidence), 'source_log_offset': offset}


def run(command, cwd, env, log, timeout):
    started = time.monotonic()
    peak = 0
    timed_out = False
    error = None
    preparation_samples = {'active': 0, 'inactive': 0, 'unavailable': 0}
    with log.open('xb') as stream:
        try:
            proc = subprocess.Popen(command, cwd=cwd, env=env, stdout=stream,
                                    stderr=subprocess.STDOUT, start_new_session=True)
        except OSError as exc:
            error = repr(exc)
            status = None
            stream.write((error + '\n').encode())
        else:
            try:
                while proc.poll() is None:
                    active = preparation_active(env.get('JR_ENGINE_HOME'))
                    if proc.poll() is None:
                        preparation_samples['unavailable' if active is None else 'active' if active else 'inactive'] += 1
                    peak = max(peak, pss_kib(proc.pid))
                    if time.monotonic() - started > timeout:
                        timed_out = True
                        os.killpg(proc.pid, signal.SIGKILL)
                        break
                    time.sleep(0.02)
            except BaseException:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
                raise
            status = proc.wait()
    finished = time.monotonic()
    return {'command': command, 'cwd': str(cwd), 'returncode': status,
            'timeout': timed_out, 'error': error, 'wall_seconds': finished - started,
            'started_monotonic': started, 'finished_monotonic': finished,
            'preparation_lock_samples_during_foreground': preparation_samples,
            'preparation_overlap_observed': preparation_samples['active'] > 0,
            'sampled_process_tree_peak_pss_kib': peak,
            'measurement_scope': 'includes polling observer overhead, excludes detached resident workers',
            'log': str(log), 'log_sha256': digest(log)}


def full_output(tool, console, cwd, env, evidence, timeout):
    """The shipping console is compact. Read only its explicit recording, untimed."""
    ids = set(re.findall(r'justrust log (\d{8}-\d{9}-\d+)', console))
    if not ids:
        return console, {'source': 'direct console', 'ok': True}, True
    if len(ids) != 1:
        return '', {'source': 'ambiguous recorded run IDs', 'ok': False,
                    'run_ids': sorted(ids)}, False
    run_id = ids.pop()
    receipt = run([str(tool), 'log', run_id], cwd, env, evidence, timeout)
    good = receipt['returncode'] == 0 and not receipt['timeout'] and not receipt['error']
    receipt.update(source='exact recorded invocation, retrieved outside timing', run_id=run_id, ok=good)
    # Never concatenate: passing test summaries could otherwise be counted twice.
    return evidence.read_text(errors='replace') if good else '', receipt, good


def cleanup_namespace(client, cwd, env, evidence, timeout):
    """Use lifecycle stop only. Never kill processes selected by name or broad pattern."""
    home = env['JR_ENGINE_HOME']
    before = namespace_pss(home)
    observed = dict(before.get('process_start_ticks', {}))
    def remaining_observed():
        remaining = []
        for pid, started in observed.items():
            try:
                fields = Path('/proc', pid, 'stat').read_text().rsplit(')', 1)[1].split()
                if fields[19] == started:
                    remaining.append(pid)
            except FileNotFoundError:
                pass
            except (OSError, ValueError, IndexError):
                remaining.append(pid)
        return remaining
    effects = {key: side_lines(Path(env[key])) for key in
               ('HYBRID_SIDE_EFFECT', 'HYBRID_PROGRAM_SIDE_EFFECT') if key in env}
    local = dict(env)
    local.pop('JUSTRUST_HYBRID_PREWARM', None)
    row = run([str(client), 'hybrid', 'stop'], cwd, local, evidence, timeout)
    deadline = time.monotonic() + timeout
    after = namespace_pss(home)
    observed.update(after.get('process_start_ticks', {}))
    while (after['process_pss_kib'] or remaining_observed()) and time.monotonic() < deadline:
        time.sleep(0.05)
        after = namespace_pss(home)
        observed.update(after.get('process_start_ticks', {}))
    effects_unchanged = all(side_lines(Path(env[key])) == lines for key, lines in effects.items())
    row.update(namespace=home, namespace_before=before, namespace_after=after,
               process_disappearance_verified=not after['process_pss_kib'] and not remaining_observed(),
               observed_process_start_ticks=observed, remaining_observed_pids=remaining_observed(),
               inaccessible_unattributed_pids=after['unavailable_pids'],
               side_effects_unchanged=effects_unchanged,
               scope='all observed exact JR_ENGINE_HOME members disappeared; unrelated unreadable processes are not attributed; lifecycle stop only, no broad kill')
    row['ok'] = (row['returncode'] == 0 and not row['timeout'] and not row['error']
                 and row['process_disappearance_verified'] and effects_unchanged)
    return row


def cleanup_after_execute(function):
    def execute_with_cleanup(args):
        args._cleanup_states = []
        args._cleanup_report = None
        cleanup_rows = []
        try:
            result = function(args)
        finally:
            for state in args._cleanup_states:
                try:
                    row = cleanup_namespace(state['client'], state['cwd'], state['env'],
                                            state['log'], min(args.timeout, 30))
                    row['stack'] = state['stack']
                except Exception as exc:
                    row = {'ok': False, 'stack': state['stack'], 'error': repr(exc)}
                cleanup_rows.append(row)
            report = args._cleanup_report
            if report is not None:
                report['cleanup'] = cleanup_rows
                report['cleanup_passed'] = all(row['ok'] for row in cleanup_rows)
                if not report['cleanup_passed']:
                    if 'correctness_passed' in report:
                        report['correctness_passed'] = False
                    if 'status' in report:
                        report['status'] = 'failed'
                    report['focused_edit_medians_seconds'] = {}
                (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
                final_status = report.get('status')
                if final_status is None:
                    final_status = ('passed' if report['correctness_passed'] else 'failed') if 'correctness_passed' in report else 'incomplete'
                print(json.dumps({'status': final_status,
                                  'cleanup_passed': report['cleanup_passed'],
                                  'report': str(args.output / 'report.json')}))
        return result if all(row['ok'] for row in cleanup_rows) else 1
    return execute_with_cleanup


TEST_ARGS = ['test', '--lib', '--', 'hybrid_side_effect', '--exact', '--nocapture']


def contract(row, text, effects, nonce, dependency, config, failure=None):
    summaries = re.findall(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;', text)
    observations = re.findall(r'HYBRID_OBSERVED nonce=(\d+) dependency=(\d+) config=(\S+)', text)
    healthy = not row['timeout'] and row.get('error') is None
    if failure == 'compile':
        ok = healthy and row['returncode'] == 101 and not effects and not summaries and not observations and 'error' in text.lower()
    else:
        failed = failure == 'test'
        ok = (healthy and row['returncode'] == (101 if failed else 0)
              and effects == [f'nonce={nonce}']
              and summaries == [('FAILED' if failed else 'ok', '0' if failed else '1', '1' if failed else '0', '0')]
              and observations == [(str(nonce), str(dependency), config)])
    return {'ok': bool(ok), 'summaries': summaries, 'observations': observations,
            'side_effect_delta': effects}


def input_hashes(root):
    return {str(p.relative_to(root)): digest(p) for p in sorted(root.rglob('*'))
            if p.is_file() and 'target' not in p.relative_to(root).parts
            and p.name not in ('Cargo.lock', 'side-effects.txt', 'program-side-effects.txt')}


@cleanup_after_execute
def execute(args):
    automatic = getattr(args, 'automatic', False)
    automatic_pattern = getattr(args, 'automatic_ready_regex', None)
    if automatic and (args.prewarm or not automatic_pattern):
        raise ValueError('--automatic requires --automatic-ready-regex and cannot combine with --prewarm')
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        raise ValueError('timeout must be finite and positive')
    if args.output.exists():
        raise ValueError('output must not exist, never reuse worker state')
    shipping = getattr(args, 'shipping_justrust', None) or args.justrust
    tools = (args.justrust, args.engine, shipping)
    for tool in tools:
        if not tool.is_file() or not os.access(tool, os.X_OK):
            raise ValueError(f'not an executable: {tool}')
    args.output.mkdir(parents=True, mode=0o700)
    env = dict(os.environ)
    for key in list(env):
        if key.startswith(('JUSTRUST_', 'JR_ENGINE_', 'RA_JIT_', 'BLAZE_')):
            del env[key]
    for item in args.env:
        key, sep, value = item.partition('=')
        if not sep or not key:
            raise ValueError('--env requires NAME=VALUE')
        env[key] = value
    rows = []
    if args.toolchain:
        env['JUSTRUST_HYBRID_TOOLCHAIN'] = str(args.toolchain)
    report = {'schema': 1, 'scope': 'generated differential correctness matrix, not promotion evidence',
              'tools': {str(p): digest(p) for p in tools},
              'roles': {'shipping': str(shipping), 'hybrid': str(args.justrust), 'engine': str(args.engine)},
              'environment': environment_receipt(env),
              'rows': rows}
    args._cleanup_report = report
    def save():
        (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    states = {}
    for stack in ('shipping', 'hybrid'):
        state = args.output / stack
        state.mkdir(mode=0o700)
        project = state / 'project'
        fixture(project)
        local = dict(env, JR_ENGINE_HOME=str(state / 'engine-home'),
                     JUSTRUST_NATIVE_ENGINE=str(args.engine))
        if automatic and stack == 'hybrid':
            local['JUSTRUST_HYBRID_PREWARM'] = 'after-success'
        Path(local['JR_ENGINE_HOME']).mkdir(mode=0o700)
        states[stack] = (state, project, local)
        args._cleanup_states.append({'client': args.justrust, 'cwd': project,
                                     'env': dict(local, HYBRID_SIDE_EFFECT=str(project / 'side-effects.txt'),
                                                 HYBRID_PROGRAM_SIDE_EFFECT=str(project / 'program-side-effects.txt')),
                                     'log': args.output / ('cleanup-' + stack + '.log'), 'stack': stack})

    def step(name, stack, root, nonce=0, dependency=7, config='one', failure=None):
        side = root / 'side-effects.txt'
        before = side_lines(side)
        local_env = dict(states[stack][2], HYBRID_SIDE_EFFECT=str(side), CARGO_TARGET_DIR=str(root / 'target'))
        command = [str(args.justrust if stack == 'hybrid' else shipping)] + (['hybrid'] if stack == 'hybrid' else []) + TEST_ARGS
        inputs = input_hashes(root)
        memory_before = namespace_pss(local_env['JR_ENGINE_HOME'])
        offset = background_offset(local_env['JR_ENGINE_HOME'])
        row = run(command, root, local_env, args.output / (name + '-' + stack + '.log'), args.timeout)
        first_after = side_lines(side)
        followup = None
        followup_after = first_after
        if automatic and name == 'cold-test' and row['returncode'] == 0:
            # No recording lookup, readiness wait, source edit, or /proc scan is
            # allowed between the two foreground invocations.
            followup = run(command, root, local_env,
                           args.output / ('automatic-loading-' + stack + '.log'), args.timeout)
            followup_after = side_lines(side)
            followup['immediate_start_gap_seconds'] = followup['started_monotonic'] - row['finished_monotonic']
            row['readiness_wait_deferred_until_after'] = 'automatic-loading'
        row.update(namespace_pss_before=memory_before,
                   namespace_pss_after=namespace_pss(local_env['JR_ENGINE_HOME']),
                   namespace_pss_after_scope='after immediate followup' if followup is not None else 'after foreground')
        text = Path(row['log']).read_text(errors='replace')
        semantic_text, receipt, recovered = full_output(
            command[0], text, root, local_env,
            args.output / (name + '-' + stack + '-recorded.log'), args.timeout)
        row['full_output'] = receipt
        after = first_after if followup is not None else side_lines(side)
        row.update(contract(row, semantic_text, after[len(before):], nonce, dependency, config, failure))
        row.update(name=name, stack=stack, input_sha256=inputs,
                   environment=environment_receipt(local_env),
                   inputs_unchanged=inputs == input_hashes(root), side_prefix_preserved=after[:len(before)] == before)
        row['ok'] = row['ok'] and recovered and row['inputs_unchanged'] and row['side_prefix_preserved']
        route = args.cold_route_regex if name in ('cold-test', 'evicted-test') else args.warm_route_regex if name in ('warm-success', 'warm-edit') else None
        row['route_verified'] = bool(re.search(route, text)) if route and stack == 'hybrid' else None
        if route and stack == 'hybrid':
            row['ok'] = row['ok'] and row['route_verified']
        if automatic and stack == 'hybrid' and row['returncode'] == 0:
            queued = 'justrust hybrid: queued background preparation' in text
            row['automatic_preparation_queued'] = queued
            if queued:
                row['automatic_preparation'] = wait_background(
                    local_env['JR_ENGINE_HOME'], offset, side, followup_after if followup is not None else after, args.timeout,
                    automatic_pattern, args.output / (name + '-background.log'))
                row['ok'] = row['ok'] and row['automatic_preparation']['ok']
                if row['automatic_preparation']['ready_observed_monotonic'] is not None:
                    row['readiness_observation_delay_after_foreground_seconds'] = row['automatic_preparation']['ready_observed_monotonic'] - row['finished_monotonic']
                row['namespace_pss_prepared'] = namespace_pss(local_env['JR_ENGINE_HOME'])
            elif name in ('cold-test', 'evicted-test'):
                row['ok'] = False
        rows.append(row)
        if followup is not None:
            followup_console = Path(followup['log']).read_text(errors='replace')
            followup_text, followup_receipt, followup_recovered = full_output(
                command[0], followup_console, root, local_env,
                args.output / ('automatic-loading-' + stack + '-recorded.log'), args.timeout)
            followup.update(contract(followup, followup_text,
                                     followup_after[len(first_after):], nonce, dependency, config))
            followup.update(name='automatic-loading', stack=stack, full_output=followup_receipt,
                            environment=environment_receipt(local_env), input_sha256=inputs,
                            inputs_unchanged=inputs == input_hashes(root),
                            side_prefix_preserved=followup_after[:len(first_after)] == first_after,
                            setup_scope='immediately after cold foreground, before readiness wait or recording retrieval')
            followup['ok'] = (followup['ok'] and followup_recovered and followup['inputs_unchanged']
                              and followup['side_prefix_preserved'])
            rows.append(followup)
        save()
        return row

    phases = [('cold-test', 0, 7, 'one', None), ('warm-success', 0, 7, 'one', None),
              ('warm-edit', 1, 7, 'one', None), ('test-failure', 2, 7, 'one', 'test'),
              ('compile-error', 3, 7, 'one', 'compile'), ('recovery', 4, 7, 'one', None),
              ('dependency-edit', 5, 9, 'one', None), ('config-switch', 6, 9, 'two', None),
              ('project-switch', 7, 7, 'one', None), ('project-return', 8, 9, 'two', None),
              ('macro-edit', 9, 9, 'two', None), ('build-script-edit', 10, 9, 'two', None),
              ('manifest-edit', 11, 9, 'two', None)]
    if args.evict:
        phases.append(('evicted-test', 12, 9, 'two', None))
    for index, (name, nonce, dependency, config, failure) in enumerate(phases):
        pair = []
        for stack in (('shipping', 'hybrid') if index % 2 == 0 else ('hybrid', 'shipping')):
            state, project, local = states[stack]
            if name == 'project-switch':
                project = state / 'second'
                fixture(project)
            if name not in ('cold-test', 'warm-success'):
                put(project, 'dep/src/lib.rs', f'pub fn value() -> u32 {{ {dependency} }}\n')
                put(project, '.cargo/config.toml', f'[env]\nPROBE_CONFIG={{value="{config}",force=true}}\n')
                macro = 11 if index >= 10 else 7
                build = 'changed' if index >= 11 else 'ready'
                if name == 'macro-edit':
                    put(project, 'macro/src/lib.rs', 'extern crate proc_macro;\n#[proc_macro]\npub fn expected(_: proc_macro::TokenStream) -> proc_macro::TokenStream { "11u32".parse().unwrap() }\n')
                if name == 'build-script-edit':
                    put(project, 'build.rs', 'fn main() { println!("cargo:rustc-env=PROBE_BUILD=changed"); println!("cargo:rerun-if-changed=build.rs"); }\n')
                if name == 'manifest-edit':
                    manifest = project / 'Cargo.toml'
                    manifest.write_text(manifest.read_text().replace('default=[]', 'default=["changed"]'))
                source(project, expected=99 if failure == 'test' else dependency,
                       nonce=nonce, broken=failure == 'compile', config=config,
                       macro=macro, build=build, feature=index >= 12)
            if name == 'evicted-test' and stack == 'hybrid':
                side = project / 'side-effects.txt'
                before = side_lines(side)
                row = run([str(args.justrust), 'hybrid', 'stop'], project,
                          dict(local, HYBRID_SIDE_EFFECT=str(side)), args.output / 'stop.log', args.timeout)
                row.update(name='stop', stack='hybrid', ok=row['returncode'] == 0 and not row['timeout'] and side_lines(side) == before)
                rows.append(row)
                save()
            if name == 'warm-success' and stack == 'hybrid' and args.prewarm:
                side = project / 'side-effects.txt'
                before = side_lines(side)
                row = run([str(args.justrust), 'hybrid', 'prewarm', *TEST_ARGS], project,
                          dict(local, HYBRID_SIDE_EFFECT=str(side), CARGO_TARGET_DIR=str(project / 'target')),
                          args.output / 'prewarm.log', args.timeout)
                row.update(name='prewarm', stack='hybrid', ok=row['returncode'] == 0 and not row['timeout'] and side_lines(side) == before)
                rows.append(row)
                save()
            pair.append(step(name, stack, project, nonce, dependency, config, failure))
        equal = all(pair[0][key] == pair[1][key] for key in ('returncode', 'summaries', 'observations', 'side_effect_delta'))
        for row in pair:
            row['differential_equal'] = equal
            row['ok'] = row['ok'] and equal
        save()
    if args.operations:
        for operation in ('check', 'build', 'run-prewarm', 'run'):
            pair = []
            for stack in ('shipping', 'hybrid'):
                if operation == 'run-prewarm' and (stack == 'shipping' or not args.prewarm):
                    continue
                state, project, local = states[stack]
                put(project, 'src/main.rs', '''fn main() {
    use std::io::Write;
    let path = std::env::var("HYBRID_PROGRAM_SIDE_EFFECT").unwrap();
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    writeln!(file, "program-nonce=13").unwrap();
    assert_eq!(hybrid_probe::value(), 9);
    assert_eq!(env!("PROBE_CONFIG"), "two");
    assert_eq!(env!("PROBE_BUILD"), "changed");
    assert_eq!(probe_macro::expected!(), 11);
    println!("HYBRID_PROGRAM_EXECUTED_13");
}
''')
                side = project / 'program-side-effects.txt'
                tests = project / 'side-effects.txt'
                before, tests_before = side_lines(side), side_lines(tests)
                command = [str(args.justrust if stack == 'hybrid' else shipping)] + (['hybrid'] if stack == 'hybrid' else [])
                command += ['prewarm', 'run'] if operation == 'run-prewarm' else [operation]
                inputs = input_hashes(project)
                memory_before = namespace_pss(local['JR_ENGINE_HOME'])
                offset = background_offset(local['JR_ENGINE_HOME'])
                row = run(command, project,
                          dict(local, CARGO_TARGET_DIR=str(project / 'target'),
                               HYBRID_SIDE_EFFECT=str(tests), HYBRID_PROGRAM_SIDE_EFFECT=str(side)),
                          args.output / (operation + '-' + stack + '.log'), args.timeout)
                row.update(namespace_pss_before=memory_before,
                           namespace_pss_after=namespace_pss(local['JR_ENGINE_HOME']))
                text = Path(row['log']).read_text(errors='replace')
                semantic_text, receipt, recovered = full_output(
                    command[0], text, project,
                    dict(local, CARGO_TARGET_DIR=str(project / 'target'),
                         HYBRID_SIDE_EFFECT=str(tests), HYBRID_PROGRAM_SIDE_EFFECT=str(side)),
                    args.output / (operation + '-' + stack + '-recorded.log'), args.timeout)
                row['full_output'] = receipt
                after = side_lines(side)
                expected_effects = ['program-nonce=13'] if operation == 'run' else []
                markers = re.findall(r'HYBRID_PROGRAM_EXECUTED_\d+', semantic_text)
                good = (recovered and row['returncode'] == 0 and not row['timeout'] and not row['error']
                        and after == before + expected_effects and side_lines(tests) == tests_before
                        and markers == (['HYBRID_PROGRAM_EXECUTED_13'] if operation == 'run' else []))
                if automatic and stack == 'hybrid' and row['returncode'] == 0:
                    queued = 'justrust hybrid: queued background preparation' in text
                    row['automatic_preparation_queued'] = queued
                    if queued:
                        row['automatic_preparation'] = wait_background(
                            local['JR_ENGINE_HOME'], offset, side, after, args.timeout,
                            automatic_pattern, args.output / (operation + '-background.log'),
                            extra_effects=((tests, tests_before),))
                        good = good and row['automatic_preparation']['ok']
                        row['namespace_pss_prepared'] = namespace_pss(local['JR_ENGINE_HOME'])
                row.update(name=operation, stack=stack, ok=good, side_effect_delta=after[len(before):],
                           program_markers=markers, input_sha256=inputs,
                           inputs_unchanged=inputs == input_hashes(project))
                row['ok'] = row['ok'] and row['inputs_unchanged']
                rows.append(row)
                pair.append(row)
                save()
            if len(pair) == 2:
                equal = all(pair[0][key] == pair[1][key] for key in ('returncode', 'side_effect_delta', 'program_markers'))
                for row in pair:
                    row.update(differential_equal=equal, ok=row['ok'] and equal)
                save()
    if automatic:
        loading = [row for row in rows if row['name'] == 'automatic-loading']
        equal = len(loading) == 2 and all(loading[0][key] == loading[1][key]
                    for key in ('returncode', 'summaries', 'observations', 'side_effect_delta'))
        for row in loading:
            row.update(differential_equal=equal, ok=row['ok'] and equal)
        report['automatic_interference'] = {
            'paired_foregrounds_validated': equal and all(row['ok'] for row in loading),
            'actual_loading_overlap_observed': any(row['preparation_overlap_observed'] for row in loading if row['stack'] == 'hybrid'),
            'scope': 'sampled background prewarm lock held while immediate foreground process was running; no readiness waits folded into foreground latency'}
    report['correctness_passed'] = all(row['ok'] for row in rows)
    report['tools_after'] = {str(p): digest(p) for p in tools}
    report['tools_unchanged'] = report['tools_after'] == report['tools']
    report['correctness_passed'] = report['correctness_passed'] and report['tools_unchanged']
    report['routing_evidence_complete'] = bool(args.cold_route_regex and args.warm_route_regex and (args.prewarm or automatic) and all(row['route_verified'] for row in rows if row['stack'] == 'hybrid' and row['name'] in ('cold-test', 'warm-success', 'warm-edit', 'evicted-test')))
    save()
    return 0 if report['correctness_passed'] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--justrust', required=True, type=absolute)
    parser.add_argument('--shipping-justrust', type=absolute, help='explicit published baseline, default plain candidate commands')
    parser.add_argument('--engine', required=True, type=absolute)
    parser.add_argument('--toolchain', type=absolute, help='native-only JUSTRUST_HYBRID_TOOLCHAIN')
    parser.add_argument('--output', required=True, type=absolute)
    parser.add_argument('--timeout', type=float, default=600)
    parser.add_argument('--env', action='append', default=[])
    parser.add_argument('--prewarm', action='store_true', help='run hybrid prewarm test ARGS before warm measurements')
    parser.add_argument('--evict', action='store_true', help='stop the private resident worker, then verify fallback recovery')
    parser.add_argument('--operations', action='store_true', help='also check build/check/run and run-prewarm side effects')
    parser.add_argument('--automatic', action='store_true', help='opt into after-success background preparation, with untimed readiness wait')
    parser.add_argument('--automatic-ready-regex', help='required NEW success receipt in private prewarm.log for --automatic')
    parser.add_argument('--cold-route-regex', help='telemetry pattern required to prove cold fallback')
    parser.add_argument('--warm-route-regex', help='telemetry pattern required to prove resident warm route')
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error('timeout must be positive')
    raise SystemExit(execute(args))


if __name__ == '__main__':
    main()
