#!/usr/bin/env python3
"""Run the existing comprehensive synth gate with an explicit hybrid candidate.

This adds no benchmark semantics. suite.py owns generation, pinned snapshots,
measurement, recording, noise rejection and comparison eligibility. The adapter
only opts build/check/test/run into JUSTRUST_HYBRID=ready. No automatic promotion occurs.
Prepare and run on the host that owns the supplied executable paths.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
BUILD_COMMANDS = ('build', 'check', 'test', 'run')


def sha256(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def executable(value):
    path = Path(value).expanduser().resolve()
    if not path.is_file() or not os.access(path, os.X_OK):
        raise argparse.ArgumentTypeError(f'expected executable file: {path}')
    return path


def adapter_text(binary, engine=None, toolchain=None, fixture_xdg_home=False,
                 fixture_latex_unicode=False, fixture_no_tex=False):
    # Keep metadata, cache and toolchain commands on the ordinary client path.
    # Explicit engine selection lives inside the adapter, so both suite parents
    # inherit identical controls and the candidate engine is recorded separately.
    return f'''#!{sys.executable}
import os
import sys
binary = {str(binary)!r}
engine = {str(engine) if engine is not None else None!r}
toolchain = {str(toolchain) if toolchain is not None else None!r}
args = sys.argv[1:]
if {fixture_xdg_home!r} and args and args[0] in {BUILD_COMMANDS!r}:
    home = os.environ.get('HOME', '')
    private = (os.path.basename(home) == 'home'
               and os.path.basename(os.path.dirname(home)).startswith('justrust-runtime-')
               and os.environ.get('XDG_CONFIG_HOME') == os.path.join(home, '.config')
               and os.environ.get('JCODE_HOME') == os.path.join(home, '.jcode'))
    if not private:
        sys.exit('fixture XDG mode requires jcode-isolated-v2 private HOME runtime context')
    # The pinned fixtures override XDG_CONFIG_HOME themselves. A forced
    # JCODE_HOME takes precedence and defeats their own credential isolation.
    # HOME and XDG remain private under the suite runtime context.
    os.environ.pop('JCODE_HOME', None)
    if {fixture_latex_unicode!r}:
        os.environ['JCODE_LATEX_RENDERING'] = 'unicode'
    if {fixture_no_tex!r}:
        for command in ('LATEX', 'DVIPNG', 'PDFLATEX', 'PDFTOCAIRO'):
            os.environ['JCODE_' + command + '_COMMAND'] = os.path.join(home, '.benchmark-no-tex', command.lower())
os.environ.pop('JUSTRUST_HYBRID', None)
if engine is not None and args and args[0] in {BUILD_COMMANDS!r}:
    os.environ['JUSTRUST_NATIVE_ENGINE'] = engine
    os.environ['JUSTRUST_HYBRID_TOOLCHAIN'] = toolchain
    os.environ['JUSTRUST_HYBRID'] = 'ready'
os.execv(binary, [binary, *args])
'''


def identities(paths):
    return {name: {'path': str(path), 'sha256': sha256(path)} for name, path in paths.items()}


def save(output, report):
    temporary = output / 'gate.json.tmp'
    temporary.write_text(json.dumps(report, indent=2) + '\n')
    temporary.replace(output / 'gate.json')


def suite_command(output, stack, seed=1):
    return [sys.executable, str(HERE / 'suite.py'), 'run', '--synth', str(seed),
            '-n', '1', '--points', '40', '--stop-on-invalid', '-o', str(output / (stack + '.json'))]


def native_toolchain(value):
    if value is None:
        raise ValueError('set --native-toolchain or JUSTRUST_HYBRID_TOOLCHAIN, refusing an all-fallback gate')
    path = Path(value).expanduser()
    if not path.is_absolute() or not path.is_dir():
        raise ValueError('native toolchain must be an existing absolute directory')
    path = path.resolve()
    for name in ('rustc', 'cargo'):
        executable(path / 'bin' / name)
    return path


def prepare(args):
    output = args.output
    toolchain = native_toolchain(args.native_toolchain)
    if (getattr(args, 'fixture_latex_unicode', False) or getattr(args, 'fixture_no_tex', False)) and not getattr(args, 'fixture_xdg_home', False):
        raise ValueError('fixture Unicode rendering requires explicit fixture XDG private-home policy')
    for tool in (args.baseline, args.candidate, args.engine):
        executable(tool)
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    tools = dict(baseline=args.baseline, candidate=args.candidate, engine=args.engine,
                 suite=HERE / 'suite.py', synth=HERE / 'synth/run.py',
                 generator=HERE / 'synth/gen.py', versus=HERE / 'versus.py',
                 runtime=HERE / 'runtime_context.py', weights=HERE / 'synth/weights.json',
                 prerequisites=HERE / 'prerequisites.py', gate=Path(__file__).resolve(),
                 native_rustc=toolchain / 'bin/rustc', native_cargo=toolchain / 'bin/cargo')
    report = dict(schema=1, status='prepared', scope=__doc__, seed=args.seed,
                  points=40, reps=1, tools=identities(tools), runs={},
                  native_toolchain=str(toolchain),
                  fixture_xdg_home=bool(getattr(args, 'fixture_xdg_home', False)),
                  fixture_latex_unicode=bool(getattr(args, 'fixture_latex_unicode', False)),
                  fixture_no_tex=bool(getattr(args, 'fixture_no_tex', False)),
                  native_identity_scope='engine, rustc and cargo executable hashes, not loaded libraries or routing proof',
                  policy='No default promotion. Require complete valid suite comparison plus independent routing/correctness and regression gates.')
    for stack, binary in (('baseline', args.baseline), ('candidate', args.candidate)):
        adapter = output / (stack + '-exec')
        adapter.write_text(adapter_text(binary, args.engine if stack == 'candidate' else None,
                                        toolchain if stack == 'candidate' else None,
                                        fixture_xdg_home=report['fixture_xdg_home'],
                                        fixture_latex_unicode=report['fixture_latex_unicode'],
                                        fixture_no_tex=report['fixture_no_tex']))
        adapter.chmod(0o700)
        report.setdefault('adapters', {})[stack] = dict(path=str(adapter), sha256=sha256(adapter))
        report.setdefault('commands', {})[stack] = suite_command(output, stack, args.seed)
    report['commands']['compare'] = [sys.executable, str(HERE / 'suite.py'), 'compare', '--require-valid',
                                     str(output / 'baseline.json'), str(output / 'candidate.json')]
    save(output, report)
    print(json.dumps(report, indent=2))
    return 0


def load_verified(output):
    report = json.loads((output / 'gate.json').read_text())
    if report.get('schema') != 1:
        raise ValueError('unsupported gate manifest schema')
    for item in [*report['tools'].values(), *report['adapters'].values()]:
        if sha256(item['path']) != item['sha256']:
            raise ValueError(f"input changed since preparation: {item['path']}")
    return report


def invoke(command, env, log):
    start = time.monotonic()
    # Inherit real console output. tee is not used, and no errors are hidden.
    with log.open('x') as stream:
        process = subprocess.Popen(command, env=env, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, text=True, bufsize=1,
                                   start_new_session=True)
        try:
            for line in process.stdout:
                sys.stdout.write(line)
                sys.stdout.flush()
                stream.write(line)
                stream.flush()
            status = process.wait()
        except BaseException:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
            raise
        finally:
            process.stdout.close()
    return dict(command=command, returncode=status, wall_seconds=time.monotonic() - start,
                log=str(log), log_sha256=sha256(log))


def control_identity(env):
    """Bind performance-affecting ambient controls without publishing secrets."""
    prefixes = ('JUSTRUST_', 'JR_ENGINE_', 'RA_JIT_', 'BLAZE_', 'CARGO_', 'RUST', 'BENCH_')
    controls = {key: value for key, value in env.items()
                if key.startswith(prefixes) or key in ('PATH', 'HOME', 'JCODE_HOME', 'LD_LIBRARY_PATH')}
    for key in ('JUSTRUST_BIN', 'JUSTRUST_NATIVE_ENGINE'):
        controls.pop(key, None)
    return hashlib.sha256(json.dumps(controls, sort_keys=True).encode()).hexdigest()


def run(args):
    report = load_verified(args.output)
    controls = control_identity(os.environ)
    if report.get('controls_sha256', controls) != controls:
        raise ValueError('ambient performance controls changed between stacks')
    report['controls_sha256'] = controls
    stacks = ('baseline', 'candidate') if args.stack == 'both' else (args.stack,)
    for stack in stacks:
        if stack in report['runs'] or (args.output / (stack + '.json')).exists():
            raise ValueError(f'{stack} already attempted, prepare a fresh output rather than overwriting evidence')
        env = dict(os.environ)
        env.pop('JUSTRUST_BIN', None)
        env.pop('JUSTRUST_NATIVE_ENGINE', None)
        env.pop('JUSTRUST_HYBRID', None)
        env['JUSTRUST'] = report['adapters'][stack]['path']
        report['status'] = 'running-' + stack
        save(args.output, report)
        result = invoke(report['commands'][stack], env, args.output / (stack + '.log'))
        report['runs'][stack] = result
        try:
            load_verified(args.output)
            result['inputs_unchanged'] = True
        except ValueError:
            result['inputs_unchanged'] = False
        report['status'] = 'measured' if result['returncode'] == 0 and result['inputs_unchanged'] else 'blocked'
        save(args.output, report)
        if report['status'] == 'blocked':
            return result['returncode'] or 2
    return 0


def outcome_parity(baseline, candidate):
    """Do not credit a fast candidate that silently ran fewer tests."""
    issues = []
    left, right = baseline.get('synth', {}), candidate.get('synth', {})
    if len(left.get('stack_names', [])) != 1 or len(right.get('stack_names', [])) != 1:
        return ['expected exactly one stack in each comprehensive suite result']
    names = (left['stack_names'][0], right['stack_names'][0])
    a = {point.get('i'): point for point in left.get('results', [])}
    b = {point.get('i'): point for point in right.get('results', [])}
    if set(a) != set(b):
        return ['baseline/candidate point identities differ']
    fields = ('exit', 'compile_failed', 'tests_failed', 'tests_passed')
    for index in sorted(a):
        samples = (a[index].get('samples', {}).get(names[0], []),
                   b[index].get('samples', {}).get(names[1], []))
        if not samples[0] or len(samples[0]) != len(samples[1]):
            issues.append(f'point {index}: baseline/candidate replicate coverage differs')
            continue
        for rep, (before, after) in enumerate(zip(*samples)):
            for field in fields:
                if field not in before or field not in after or before[field] != after[field]:
                    issues.append(f'point {index} rep {rep}: {field} differs or is missing')
    return issues


def compare(args):
    report = load_verified(args.output)
    if 'comparison' in report:
        raise ValueError('comparison already attempted, evidence will not be overwritten')
    if any(stack not in report['runs'] or report['runs'][stack]['returncode'] != 0
           or not report['runs'][stack].get('inputs_unchanged') for stack in ('baseline', 'candidate')):
        raise ValueError('both complete unchanged successful suite runs are required')
    result = invoke(report['commands']['compare'], dict(os.environ), args.output / 'compare.log')
    try:
        load_verified(args.output)
        result['inputs_unchanged'] = True
    except ValueError:
        result['inputs_unchanged'] = False
    report['comparison'] = result
    if result['returncode'] == 0:
        baseline = json.loads((args.output / 'baseline.json').read_text())
        candidate = json.loads((args.output / 'candidate.json').read_text())
        result['outcome_parity_issues'] = outcome_parity(baseline, candidate)
        for issue in result['outcome_parity_issues']:
            print('Hybrid outcome comparison blocked: ' + issue)
    else:
        result['outcome_parity_issues'] = ['suite comparison ineligible, outcome parity not evaluated']
    accepted = result['returncode'] == 0 and result['inputs_unchanged'] and not result['outcome_parity_issues']
    report['status'] = 'comparison-eligible' if accepted else 'blocked'
    report['promotion_authorized'] = False
    save(args.output, report)
    return result['returncode'] or (0 if accepted else 2)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    subs = parser.add_subparsers(dest='operation', required=True)
    p = subs.add_parser('prepare', help='write adapters and provenance, no builds')
    p.add_argument('--baseline', required=True, type=executable)
    p.add_argument('--candidate', required=True, type=executable)
    p.add_argument('--engine', required=True, type=executable)
    p.add_argument('--native-toolchain', default=os.environ.get('JUSTRUST_HYBRID_TOOLCHAIN'),
                   help='existing matching engine toolchain, defaults to JUSTRUST_HYBRID_TOOLCHAIN')
    p.add_argument('--fixture-xdg-home', action='store_true',
                   help='both adapters remove forced JCODE_HOME, requires private jcode-isolated-v2 runtime context')
    p.add_argument('--fixture-latex-unicode', action='store_true',
                   help='both private-runtime adapters select supported Unicode math rendering for synchronous fixture expectations')
    p.add_argument('--fixture-no-tex', action='store_true',
                   help='both private-runtime adapters explicitly select unavailable TeX command paths, narrowing fixture runtime scope')
    p.add_argument('--seed', type=int, default=1)
    p.set_defaults(function=prepare)
    p = subs.add_parser('run', help='execute comprehensive suite, potentially hours')
    p.add_argument('--stack', choices=('baseline', 'candidate', 'both'), default='both')
    p.set_defaults(function=run)
    p = subs.add_parser('compare', help='run suite.py compare --require-valid, never promote')
    p.set_defaults(function=compare)
    for p in subs.choices.values():
        p.add_argument('--output', required=True, type=lambda value: Path(value).expanduser().resolve())
    args = parser.parse_args(argv)
    return args.function(args)


if __name__ == '__main__':
    raise SystemExit(main())
