#!/usr/bin/env python3
"""Native declarative macro differential acceptance, not a performance benchmark.

Every build uses identical source/output paths. Only incremental directories vary.
No diagnostic normalization is performed. Run only against a frozen new compiler.
"""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

FLAG = '-Zjr-cache-declarative-macros'
MARKERS = ('jr_declarative_macro_eligible', 'jr_declarative_macro_ineligible',
           'jr_declarative_macro_cache_lookup', 'jr_declarative_macro_cache_provider',
           'jr_declarative_macro_cache_success')
LITERAL = '''macro_rules! literal { () => { 42 }; }
fn body() -> i32 { 1 }
fn main() { if literal!() + body() != 43 { std::process::exit(9); } }
'''
BASE = '''#![warn(unused_macros, unused_macro_rules)]
macro_rules! unused { () => { 0 }; }
macro_rules! value { (never) => { 99 }; ($e:expr) => {{ let private = 10; $e + private }}; }
macro_rules! nested { () => { 3 }; }
macro_rules! forward { ($e:expr) => { opaque!($e) }; }
macro_rules! opaque { (3) => { 99 }; ($e:expr) => { $e }; }
fn body() -> i32 { 1 }
fn main() {
    let private = 2;
    let outer = nested!();
    let inner = { macro_rules! nested { () => { 4 }; } nested!() };
    println!("{} {} {} {} {}", value!(private), outer, inner, forward!(3), body());
}
'''
DEPENDENCY = '''pub fn helper() -> i32 { 7 }
#[macro_export] macro_rules! exported { () => { $crate::helper() }; }
'''


def case(name, source=BASE, expected='12 3 4 3 1\n', dependency=None,
         edition='2024', trace=False):
    return dict(name=name, source=source, expected=expected, dependency=dependency,
                edition=edition, trace=trace)


def cases():
    exported = BASE.replace('value!(private)', 'macro_dep::exported!()')
    return [
        case('literal-eligible', LITERAL, ''),
        case('literal-eligible-unchanged', LITERAL, ''),
        case('literal-eligible-body-edit', LITERAL.replace('{ 1 }', '{ 2 }').replace('!= 43', '!= 44'), ''),
        case('initial'), case('unchanged'),
        case('definition-edit', BASE.replace('$e + private', '$e + private + 1'), '13 3 4 3 1\n'),
        case('input-edit', BASE.replace('value!(private)', 'value!(private + 1)'), '13 3 4 3 1\n'),
        case('body-edit', BASE.replace('fn body() -> i32 { 1 }', 'fn body() -> i32 { 2 }'), '12 3 4 3 2\n'),
        case('nested-local-shadowing', BASE.replace('() => { 4 }', '() => { 8 }'), '12 3 8 3 1\n'),
        case('hygiene', BASE.replace('let private = 2;', 'let private = 5;'), '15 3 4 3 1\n'),
        case('dollar-crate', exported, '7 3 4 3 1\n', DEPENDENCY),
        case('dollar-crate-renamed-and-shadowed',
             exported.replace('fn body()', 'use macro_dep as renamed;\nfn helper() -> i32 { 700 }\nfn body()')
             .replace('macro_dep::exported!()', 'renamed::exported!()'),
             '7 3 4 3 1\n', DEPENDENCY),
        case('exported-dependency-definition-edit', exported, '8 3 4 3 1\n', DEPENDENCY.replace('$crate::helper()', '$crate::helper() + 1')),
        case('exported-dependency-body-edit', exported, '9 3 4 3 1\n', DEPENDENCY.replace('{ 7 }', '{ 9 }')),
        case('exported-dependency-failure', exported, None, DEPENDENCY.replace('$crate::helper()', '$crate::missing()')),
        case('exported-dependency-recovery', exported, '7 3 4 3 1\n', DEPENDENCY),
        case('span-relocation', '// moved invocation and definition spans\n\n' + BASE.replace('    println!', '\n\n    println!')),
        case('unused-macro-diagnostic', BASE.replace('macro_rules! unused', 'macro_rules! unused_renamed')),
        case('unused-rule-diagnostic', BASE.replace('(never) => { 99 }', '(another_unused) => { 100 }')),
        case('unused-rule-live', BASE.replace('value!(private)', 'value!(never)'), '99 3 4 3 1\n'),
        case('deny-unused-macro', '#![deny(unused_macros)]\n' + BASE.replace('unused_macros, ', ''), None),
        case('diagnostic-recovery'),
        case('expansion-failure', BASE.replace('value!(private)', 'value!(@)'), None),
        case('expansion-recovery'),
        case('interpolation-opaque', BASE.replace('forward!(3)', 'opaque!(3)'), '12 3 4 99 1\n'),
        case('interpolation-recovery'),
        case('edition-2021-expr', BASE.replace('value!(private)', 'value!(const { 2 })'), None, edition='2021'),
        case('edition-2024-expr', BASE.replace('value!(private)', 'value!(const { 2 })')),
        case('edition-2018-pattern', 'macro_rules! p { ($p:pat) => { matches!(1, $p) }; }\nfn main() { println!("{}", p!(1 | 2)); }\n', None, edition='2018'),
        case('edition-2021-pattern', 'macro_rules! p { ($p:pat) => { matches!(1, $p) }; }\nfn main() { println!("{}", p!(1 | 2)); }\n', 'true\n', edition='2021'),
        case('macro-tracing', trace=True),
        case('final-recovery'),
    ]


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def run(command, timeout=180):
    command = [str(x) for x in command]
    started = time.monotonic()
    result = subprocess.run(command, capture_output=True, timeout=timeout)
    return dict(command=command, status=result.returncode,
                stdout_b64=base64.b64encode(result.stdout).decode('ascii'),
                stderr_b64=base64.b64encode(result.stderr).decode('ascii'),
                elapsed_seconds=time.monotonic() - started)


def observable(record):
    return {key: record[key] for key in ('status', 'stdout_b64', 'stderr_b64')}


def compare(reference, actual, label):
    if observable(reference) != observable(actual):
        raise AssertionError(label + ': exact status/diagnostic/output bytes differ\n' +
                             json.dumps(dict(reference=reference, actual=actual), indent=2))


def schedule(index):
    return [('fresh-no', False, f'fresh/{index}/no'),
            ('fresh-yes', True, f'fresh/{index}/yes'),
            ('persistent-no', False, 'persistent/no'),
            ('persistent-yes', True, 'persistent/yes'),
            ('transition-no-before', False, 'transition'),
            ('transition-yes', True, 'transition'),
            ('transition-no-after', False, 'transition')]


def compile_case(rustc, root, fixture, enabled, cache, profile=None):
    source = root / 'fixture.rs'
    binary = root / 'fixture'
    source.write_text(fixture['source'])
    binary.unlink(missing_ok=True)
    flags = [f'{FLAG}={"yes" if enabled else "no"}', '-Zincremental-verify-ich',
             '-Cdebuginfo=0', f'--edition={fixture["edition"]}']
    if profile is not None:
        profile.mkdir(parents=True, exist_ok=False)
        flags += [f'-Zself-profile={profile}', '-Zself-profile-events=default,function-args']
    result = {}
    dependency = root / 'libmacro_dep.rlib'
    dependency.unlink(missing_ok=True)
    if fixture['dependency'] is not None:
        dep_source = root / 'macro_dep.rs'
        dep_source.write_text(fixture['dependency'])
        result['dependency'] = run([rustc, dep_source, '--crate-name=macro_dep',
                                    '--crate-type=rlib', *flags, '-C',
                                    f'incremental={root / "incremental" / cache / "dependency"}',
                                    '-o', dependency])
        if result['dependency']['status'] != 0:
            raise AssertionError('Dependency compile failed: ' + json.dumps(result))
    command = [rustc, source, '--crate-name=declarative_macro_accept', *flags,
               '-C', f'incremental={root / "incremental" / cache / "binary"}', '-o', binary]
    if fixture['dependency'] is not None:
        command += ['--extern', f'macro_dep={dependency}']
    if fixture['trace']:
        command += ['-Ztrace-macros=yes']
    result['compile'] = run(command)
    if result['compile']['status'] == 0:
        result['runtime'] = run([binary])
        result['artifact_sha256'] = digest(binary)
    return result


def validate_expected(record, expected, label):
    status = record['compile']['status']
    if expected is None:
        if status != 1:
            raise AssertionError(f'{label}: expected ordinary compiler error (1), got {status}')
        return
    if status != 0:
        raise AssertionError(f'{label}: unexpected compiler failure\n' + json.dumps(record, indent=2))
    wanted = dict(status=0, stdout_b64=base64.b64encode(expected.encode()).decode('ascii'), stderr_b64='')
    if observable(record['runtime']) != wanted:
        raise AssertionError(f'{label}: wrong runtime result\n' + json.dumps(record, indent=2))


def identity(rustc):
    version = run([rustc, '-vV'])
    sysroot = run([rustc, '--print', 'sysroot'])
    if version['status'] or sysroot['status']:
        raise AssertionError('Compiler identity query failed')
    sysroot_path = Path(base64.b64decode(sysroot['stdout_b64']).decode().strip())
    drivers = sorted((sysroot_path / 'lib').glob('*rustc_driver*'))
    libraries = [dict(path=str(p.resolve()), sha256=digest(p)) for p in drivers if p.is_file()]
    if not libraries:
        raise AssertionError('No compiler driver library found in sysroot/lib')
    return dict(path=str(rustc), sha256=digest(rustc), version=observable(version),
                sysroot=observable(sysroot), drivers=libraries)


class UnsupportedFeature(RuntimeError):
    pass


def probe(rustc, root):
    source = root / 'fixture.rs'
    source.write_text('fn main() {}\n')
    records = {}
    for value in ('no', 'yes'):
        records[value] = run([rustc, source, '--crate-name=declarative_macro_accept',
                              f'{FLAG}={value}', '-Zincremental-verify-ich',
                              '--emit=metadata', '-o', root / 'probe.rmeta'])
    return records


def marker_counts(records):
    """Never assign an unresolved numeric/virtual query label to a marker."""
    counts = dict.fromkeys(MARKERS, 0)
    unresolved = 0
    total = 0
    for event in records:
        total += 1
        if not event['label_resolved']:
            unresolved += 1
            continue
        if event['label'] in counts:
            if event['kind'] != 'GenericActivity' or event['record_type'] != 'interval':
                raise AssertionError('Native marker has an unexpected event kind/type')
            counts[event['label']] += 1
    return dict(markers=counts, records=total, unresolved_records=unresolved)


def profile_evidence(directory):
    profiles = sorted(directory.glob('*.mm_profdata'))
    if len(profiles) != 1:
        raise AssertionError(f'Expected exactly one native profile in {directory}, found {len(profiles)}')
    spec = importlib.util.spec_from_file_location('macro_accept_query_profile',
                                                Path(__file__).with_name('query_profile.py'))
    qprof = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(qprof)
    data = qprof.read_input(profiles[0])
    events, strings = qprof.prepare(data, allow_unmapped_event_labels=True)
    evidence = marker_counts(qprof.iter_prepared_records(events, strings, allow_unmapped_event_labels=True))
    evidence.update(path=str(profiles[0]), sha256=hashlib.sha256(data).hexdigest(),
                    bytes=len(data), metadata=qprof.metadata_from_strings(strings))
    return evidence


def validate_native_markers(observations):
    """Lookup minus provider is reuse only after positive cold calibration."""
    by_name = {o['name']: o['profile']['markers'] for o in observations}
    for name in ('off-before', 'off-after'):
        if any(by_name[name].values()):
            raise AssertionError(f'{name}: default-off path unexpectedly emitted native cache markers')
    cold = by_name['cold-yes']
    eligible, rejected, lookup, provider, success = (cold[m] for m in MARKERS)
    if not (eligible > 0 and rejected == 0 and eligible == lookup == provider == success):
        raise AssertionError('Cold literal fixture did not positively exercise native eligible/provider/success paths')
    avoided = {}
    for name in ('warm-yes', 'body-edit-yes'):
        eligible, rejected, lookup, provider, success = (by_name[name][m] for m in MARKERS)
        if not (eligible > 0 and rejected == 0 and eligible == lookup == success and 0 <= provider < lookup):
            raise AssertionError(f'{name}: no positive native cache reuse evidence, or marker contract violated')
        avoided[name] = lookup - provider
    return dict(schema=1, proof='positive cold calibration then successful lookups without provider execution',
                inferred_provider_avoided=avoided,
                limitation='Proves reuse of the native literal match/transcribe query, not speedup or full macro-expansion caching')


def native_evidence(rustc, root, evidence):
    """Independent profiled processes and cache, still exact semantic comparisons."""
    evidence.update(schema=1, profile_events='default,function-args', marker_schema=list(MARKERS))
    original = case('literal-evidence', LITERAL, '')
    edited = case('literal-evidence-body-edit', LITERAL.replace('{ 1 }', '{ 2 }').replace('!= 43', '!= 44'), '')
    modes = [('off-before', False, 'native-proof/off-reference', original),
             ('cold-yes', True, 'native-proof/shared', original),
             ('warm-yes', True, 'native-proof/shared', original),
             ('body-edit-yes', True, 'native-proof/shared', edited),
             ('off-after', False, 'native-proof/shared', edited)]
    reference = None
    for name, enabled, cache, fixture in modes:
        observation = dict(name=name, fixture=fixture)
        evidence['observations'].append(observation)
        profile = root / 'native-profiles' / name
        observation['result'] = compile_case(rustc, root, fixture, enabled, cache, profile=profile)
        validate_expected(observation['result'], '', name)
        if reference is None:
            reference = observation['result']
        else:
            for component in ('compile', 'runtime'):
                compare(reference[component], observation['result'][component], f'native-proof/{name}/{component}')
        observation['profile'] = profile_evidence(profile)
    evidence['proof'] = validate_native_markers(evidence['observations'])


def capture(rustc, root, require_native_hit=False):
    root.mkdir(mode=0o700, parents=False, exist_ok=False)
    report = dict(schema=1, flag=FLAG, scope='correctness-only-no-speedup-claim',
                  verify_ich=True, diagnostics='exact-bytes-no-normalization',
                  native_hit_required=require_native_hit, native_hit_verdict='not-requested',
                  semantic_verdict='incomplete',
                  planned_cases=len(cases()), completed_cases=0, verdict='incomplete', observations=[])
    try:
        report['identity_before'] = identity(rustc)
        report['feature_probe'] = probe(rustc, root)
        if any(r['status'] for r in report['feature_probe'].values()):
            unsupported = all(r['status'] in (0, 1) for r in report['feature_probe'].values()) and any(
                b'unknown unstable option' in base64.b64decode(r['stderr_b64'])
                for r in report['feature_probe'].values())
            if unsupported:
                raise UnsupportedFeature('Required native yes/no flag or verify-ich unsupported. See raw feature_probe diagnostics.')
            raise AssertionError('Required native yes/no flag or verify-ich probe failed unexpectedly')
        # Trace is optional only when rustc explicitly rejects the option itself.
        trace_source = root / 'fixture.rs'
        trace_source.write_text('macro_rules! p { () => {} } fn main() { p!(); }\n')
        trace_probe = run([rustc, trace_source, '-Ztrace-macros=yes',
                           '--emit=metadata', '-o', root / 'probe.rmeta'])
        report['trace_probe'] = trace_probe
        trace_stderr = base64.b64decode(trace_probe['stderr_b64'])
        trace_supported = trace_probe['status'] == 0
        if not trace_supported and not (trace_probe['status'] == 1 and
                                        b'unknown unstable option' in trace_stderr and
                                        b'trace-macros' in trace_stderr):
            raise AssertionError('Macro tracing probe failed for an unexpected reason')
        if trace_supported and b'trace_macro' not in trace_stderr:
            raise AssertionError('Tracing accepted but emitted no trace_macro diagnostic')
        for index, fixture in enumerate(cases()):
            observation = dict(fixture=fixture, modes={})
            report['observations'].append(observation)
            if fixture['trace'] and not trace_supported:
                observation['unsupported'] = 'compiler explicitly rejects -Ztrace-macros'
                continue
            reference = None
            for mode, enabled, cache in schedule(index):
                result = compile_case(rustc, root, fixture, enabled, cache)
                observation['modes'][mode] = result
                validate_expected(result, fixture['expected'], f'{fixture["name"]}/{mode}')
                if reference is None:
                    reference = result
                else:
                    for component in ('dependency', 'compile', 'runtime'):
                        if component in reference:
                            compare(reference[component], result[component], f'{fixture["name"]}/{mode}/{component}')
            report['completed_cases'] += 1
        report['semantic_verdict'] = 'passed'
        if require_native_hit:
            report['native_hit_verdict'] = 'incomplete'
            report['native_evidence'] = dict(observations=[])
            native_evidence(rustc, root, report['native_evidence'])
            report['native_hit_verdict'] = 'passed'
        report['verdict'] = 'passed'
    except Exception as error:
        report['verdict'] = 'unsupported' if isinstance(error, UnsupportedFeature) else 'failed'
        if report['native_hit_verdict'] == 'incomplete':
            report['native_hit_verdict'] = 'failed'
        report['error'] = str(error)
        raise
    finally:
        try:
            report['identity_after'] = identity(rustc)
            if report.get('identity_before') != report['identity_after']:
                report['verdict'] = 'failed'
                report['identity_error'] = 'Compiler/driver identity changed during acceptance'
                raise AssertionError(report['identity_error'])
        except Exception as error:
            report['verdict'] = 'failed'
            report['identity_error'] = str(error)
            raise
        finally:
            (root / 'acceptance.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rustc', '--compiler', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='New directory whose parent exists')
    parser.add_argument('--require-native-hit', action='store_true',
                        help='Also require cold provider execution and cross-process warm/body-edit cache-hit profile evidence')
    args = parser.parse_args()
    os.umask(0o077)
    report = capture(args.rustc.resolve(), args.output.resolve(), args.require_native_hit)
    print(json.dumps(dict(output=str(args.output), verdict=report['verdict'],
                          semantic_verdict=report['semantic_verdict'],
                          native_hit_verdict=report['native_hit_verdict'], cases=report['completed_cases'])))


if __name__ == '__main__':
    try:
        main()
    except UnsupportedFeature as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
