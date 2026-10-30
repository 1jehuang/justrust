#!/usr/bin/env python3
"""Bounded, descriptive pinned resident state transitions. Never a promotion gate."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import statistics
import uuid

import resident_engine_compare as shared
import resident_workload_compare as workload

runtime = shared.runtime
STACKS = ['reference', 'engine']
MAX_RAW_BYTES = 64 * 1024 * 1024
FILES = ('src/summary.rs', 'src/shim.rs', 'Cargo.toml')
PHASES = ('cold-prepare', 'repeated-noop-1', 'repeated-noop-2',
          'summary-testbody-edit', 'summary-edit-recovery', 'ordinary-shim-edit', 'signature-base',
          'signature-invalidation', 'manifest-feature-add', 'type-error',
          'type-recovery', 'assert-failure', 'assert-recovery', 'all-lib')
ANCHOR = b'fn shares_overlapping_time() {'
SHIM_ANCHOR = workload.WORKLOADS['shim-test'][2].encode()
PROBE = (b'\nprintln!("JR_MATRIX_RUNTIME_{}", '
         b'std::env::var("JR_MATRIX_NONCE").expect("matrix invocation nonce"));\n')
SIGNATURE = (b'\nfn jr_matrix_signature(value: u64) -> u64 { value }\n'
             b'assert_eq!(jr_matrix_signature(std::hint::black_box(7u64)), 7u64);\n')
SIGNATURE_CHANGED = (b'\nfn jr_matrix_signature(value: u32) -> u64 { u64::from(value) }\n'
                     b'assert_eq!(jr_matrix_signature(std::hint::black_box(7u32)), 7u64);\n')
POLICY = ('Descriptive per-state results only. No arbitrary aggregate weighting, '
          'significance claim, comprehensive synthetic-workload claim, general faster-than-Blaze '
          'claim, or default promotion. Mutations instrument runtime execution and affect timings. '
          'Daemon/background CPU and OS cache noise remain uncontrolled.')


def durable(path, data, exclusive=False):
    with path.open('xb' if exclusive else 'wb') as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def captured(prefix):
    """Retain complete raw files, refuse oversized text admission instead of truncating."""
    raw = {s: dict(path=str(prefix.with_suffix('.' + s)),
                   bytes=prefix.with_suffix('.' + s).stat().st_size,
                   sha256=runtime.base.digest(prefix.with_suffix('.' + s)))
           for s in ('stdout', 'stderr')}
    error = ('raw stream exceeds bounded parser limit of ' + str(MAX_RAW_BYTES) + ' bytes'
             if any(item['bytes'] > MAX_RAW_BYTES for item in raw.values()) else None)
    parts = []
    if not error:
        for suffix in ('stdout', 'stderr'):
            with Path(raw[suffix]['path']).open('rb') as stream:
                data = stream.read(MAX_RAW_BYTES + 1)
            if len(data) > MAX_RAW_BYTES:
                error = 'raw stream grew beyond bounded parser limit'
                break
            parts.append(data.decode(errors='replace'))
    return raw, '' if error else '\n'.join(parts), error


def baseline(originals):
    result = dict(originals)
    for name, anchor in ((FILES[0], ANCHOR), (FILES[1], SHIM_ANCHOR)):
        if result[name].count(anchor) != 1:
            raise ValueError('probe anchor must occur exactly once: ' + name)
        result[name] = result[name].replace(anchor, anchor + PROBE, 1)
    result[FILES[0]] = result[FILES[0]].replace(ANCHOR, ANCHOR + SIGNATURE, 1)
    return result


def inputs(originals, phase, rep, nonce):
    """Exact imported edit recipes, applied over fixed execution instrumentation."""
    result = baseline(originals)
    summary, shim, manifest = FILES
    if phase == 'summary-testbody-edit':
        result[summary] = runtime.edit_bytes(result[summary], rep)
    elif phase == 'ordinary-shim-edit':
        result[shim] = workload.edited(result[shim], 'shim-function', 'edit', rep)
    elif phase in ('signature-invalidation', 'manifest-feature-add'):
        result[summary] = result[summary].replace(SIGNATURE, SIGNATURE_CHANGED, 1)
        if phase == 'manifest-feature-add':
            if b'[features]' in result[manifest] or b'jr_matrix_unused' in result[manifest]:
                raise ValueError('feature fixture requires pinned feature-free manifest')
            result[manifest] += b'\n[features]\njr_matrix_unused = []\n'
    elif phase == 'type-error':
        result[summary] = runtime.edit_bytes(result[summary], kind='type-error').replace(
            b'JR_RUNTIME_TYPE_ERROR', ('JR_MATRIX_TYPE_' + nonce).encode())
    elif phase == 'assert-failure':
        result[summary] = runtime.edit_bytes(result[summary], kind='test-failure').replace(
            b'JR_RUNTIME_FAILURE', ('JR_MATRIX_FAILURE_' + nonce).encode())
        result[summary] = result[summary].replace(PROBE, b'', 1).replace(ANCHOR, ANCHOR + PROBE, 1)
    return result


def arguments(phase):
    if phase == 'all-lib':
        return ['--lib', '--', '--test-threads=1', '--nocapture']
    prefix = 'shim::tests::' if phase == 'ordinary-shim-edit' else 'summary::tests::'
    return ['--lib', '--', prefix, '--test-threads=1', '--nocapture']


def contract(parsed, phase):
    if not parsed or parsed['failed'] or not parsed['tests']:
        return None
    if phase == 'all-lib':
        required = set(shared.NAMES + ['shim::tests::describes_cargo_rustc_args',
                                      'shim::tests::parses_time_passes_json'])
        names = {n for n, _ in parsed['tests']}
        ignored = [n for n, status in parsed['tests'] if status == 'ignored']
        return parsed if (parsed['filtered'] == 0 and required <= names
            and len(names) == 148 and parsed['passed'] == 147 and parsed['ignored'] == 1
            and ignored == ['split_index::tests::attach_cost']
            and all(status in ('ok', 'ignored') for _, status in parsed['tests'])) else None
    if parsed['ignored'] or parsed['passed'] != len(parsed['tests']) or any(s != 'ok' for _, s in parsed['tests']):
        return None
    names = [n for n, _ in parsed['tests']]
    if phase == 'ordinary-shim-edit':
        return workload.discover(parsed, 'shim-function')
    return parsed if names == shared.NAMES else None


def oracle(row, text, expected, phase, nonce, rep, ordinary_count=None):
    """Every successful runtime needs an invocation-specific environment challenge."""
    failures = []
    if not row.get('input_unchanged'):
        failures.append('source inputs changed during invocation')
    if row.get('raw_error'):
        failures.append(row['raw_error'])
    if row.get('timed_out') or row.get('error'):
        failures.append('timeout or process launch error')
    parsed = row.get('parsed')
    markers = re.findall(r'JR_MATRIX_RUNTIME_[^\s]*', text)
    wanted = 'JR_MATRIX_RUNTIME_' + nonce
    if phase == 'type-error':
        if not (row.get('status') == 101 and parsed is None and 'E0308' in text
                and 'JR_MATRIX_TYPE_' + nonce in text and not markers):
            failures.append('fresh type-error diagnostic/no-runtime oracle failed')
    else:
        count = 2 if phase == 'all-lib' else 1
        if markers != [wanted] * count:
            failures.append('fresh invocation marker/count mismatch')
        failing = shared.EDIT_TEST if phase == 'assert-failure' else None
        matches = (expected is not None and parsed == expected) if phase == 'all-lib' else workload.valid(parsed, expected, failing)
        if not matches:
            failures.append('exact reference test names/status/counts/filtered mismatch')
        if phase == 'assert-failure':
            if row.get('status') != 101 or 'JR_MATRIX_FAILURE_' + nonce not in text:
                failures.append('fresh assertion failure oracle failed')
        elif not runtime.base.succeeded(row):
            failures.append('process was not successful')
        if phase == 'summary-testbody-edit':
            if re.findall(r'JR_RUNTIME_EXECUTED_[^\s]*', text) != [f'JR_RUNTIME_EXECUTED_{rep + 1}']:
                failures.append('imported testbody marker mismatch')
        if phase == 'ordinary-shim-edit':
            found = re.findall(r'JR_WORKLOAD_EXECUTED_[^\s]*', text)
            marker = f'JR_WORKLOAD_EXECUTED_shim_function_{rep + 1}'
            if not ordinary_count or found != [marker] * ordinary_count:
                failures.append('ordinary function invocation count not reference-validated')
    return failures


def summarize(rows, samples, unchanged):
    result = {}
    for phase in PHASES:
        result[phase] = {}
        for stack in STACKS:
            selected = [r for r in rows if r['phase'] == phase and r['stack'] == stack]
            errors = [dict(replicate=r['replicate'], reasons=r.get('failures', ['unvalidated']))
                      for r in selected if not r.get('accepted')]
            admitted = unchanged and len(selected) == samples and not errors
            reasons = []
            if not unchanged:
                reasons.append('source/tool/toolchain/raw receipt/restoration identity gate failed')
            if len(selected) != samples:
                reasons.append(f'incomplete phase: expected {samples} records, observed {len(selected)}')
            if errors:
                reasons.append('one or more explicit state oracles failed')
            result[phase][stack] = dict(expected_samples=samples, observed_samples=len(selected),
                exclusion_reasons=reasons,
                accepted_samples=sum(bool(r.get('accepted')) for r in selected), excluded=not admitted,
                failures=errors, median_seconds=statistics.median(
                    r['elapsed_seconds'] for r in selected) if admitted else None)
    return result


def execute(args):
    if not 5 <= args.samples <= 30 or not math.isfinite(args.timeout) or not 0 < args.timeout <= 3600:
        raise ValueError('5 to 30 samples and finite timeout in (0, 3600] required')
    if (args.source.name != 'source63' or args.output.exists()
            or args.output.is_relative_to(args.source) or args.source.is_relative_to(args.output)
            or len(os.fsencode(args.output)) > 60):
        raise ValueError('require pristine source63 and fresh disjoint short output (<=60 bytes)')
    shared.pinned_source(args.source)
    tools = dict(reference=args.reference_engine, engine=args.engine)
    if args.proc_macro_server:
        tools['proc-macro-server'] = args.proc_macro_server
    for tool in tools.values():
        with tool.open('rb') as stream:
            if stream.read(4) != b'\x7fELF' or not os.access(tool, os.X_OK):
                raise ValueError('existing native executable ELF tools required')
    identity = lambda: {name: dict(path=str(p), sha256=runtime.base.digest(p))
                        for name, p in tools.items()}
    metadata = dict(pin=shared.PIN, source=str(args.source), samples=args.samples, phases=list(PHASES), policy=POLICY,
        proc_macro_policy='same frozen native server' if args.proc_macro_server else 'both server variables 0, rustc-built proc macros',
        source_before=shared.source_hashes(args.source), tools_before=identity(),
        toolchain_before=shared.toolchain_identity(args.toolchain), contracts={},
        store_scope='fresh private per replicate/stack, reused through ordered phases; no seeding',
        cold_scope='actual elapsed first test command; private tool/dependency disk stores newly '
                   'created for this session, version queries and frozen toolchain links precede prepare; '
                   'not machine-cold: OS page cache, installed toolchain and ambient HOME not flushed',
        counts={phase: args.samples * 2 for phase in PHASES},
        planned_version_calls=args.samples * 2,
        planned_workload_calls=len(PHASES) * args.samples * 2,
        max_raw_parser_bytes_per_stream=MAX_RAW_BYTES,
        environments={}, versions={}, status='incomplete')
    args.output.mkdir(parents=True, mode=0o700)
    originals = {name: (args.source / name).read_bytes() for name in FILES}
    backups = {}
    rows, envs = [], {}

    def save():
        for name, value in (('metadata.json', metadata), ('records.json', rows)):
            (args.output / name).write_text(json.dumps(value, indent=2) + '\n')

    def restore():
        errors = []
        for name, backup in backups.items():
            try:
                data = backup.read_bytes()
                if hashlib.sha256(data).hexdigest() != metadata['backups'][name]['sha256']:
                    raise ValueError('durable backup changed: ' + name)
                durable(args.source / name, data)
            except (OSError, ValueError) as exc:
                errors.append(repr(exc))
        if errors:
            raise ValueError('restoration errors: ' + repr(errors))

    try:
        metadata['backups'] = {}
        for index, (name, data) in enumerate(originals.items()):
            backup = args.output / f'original-{index}'
            durable(backup, data, exclusive=True)
            metadata['backups'][name] = dict(path=str(backup), sha256=runtime.base.digest(backup))
            backups[name] = backup
        save()
        for rep in range(args.samples):
            session = args.output / f's{rep}'
            session.mkdir(mode=0o700)
            versions = {}
            for stack in runtime.order(STACKS, rep):
                key = f'{rep}:{stack}'
                env = shared.environment(session, stack, args.toolchain, args.proc_macro_server)
                envs[key] = env
                metadata['environments'][key] = workload.receipt_environment(env)
                prefix = session / f'{stack}-version'
                row = runtime.run_process([str(tools[stack]), '--version'], args.source, env, prefix, args.timeout)
                row['environment'] = workload.receipt_environment(env)
                raw, text, raw_error = captured(prefix)
                row.update(raw=raw, raw_error=raw_error)
                pattern = re.escape(shared.REFERENCE_VERSION) + '$' if stack == 'reference' else r'jr-engine\s+\S+'
                versions[stack] = (not raw_error and runtime.base.succeeded(row)
                                   and bool(re.search('^' + pattern, text, re.M)))
                metadata['versions'][key] = dict(process=row, valid=versions[stack], raw=raw)
            healthy = dict(versions)
            blocked = {stack: f'invalid native version, see versions[{rep}:{stack}] raw receipt'
                       for stack in STACKS}
            previous_inputs = {}
            expected_summary = None
            for phase in PHASES:
                pair = {}
                for stack in runtime.order(STACKS, rep):
                    key = f'{rep}:{stack}'
                    nonce = uuid.uuid4().hex
                    if not healthy[stack]:
                        row = dict(stack=stack, phase=phase, replicate=rep, accepted=False,
                                   failures=[blocked[stack]], elapsed_seconds=None)
                        rows.append(row)
                        continue
                    state = inputs(originals, phase, rep, nonce)
                    for name, data in state.items():
                        # Do not touch unchanged files, especially repeated noop inputs.
                        path = args.source / name
                        if path.read_bytes() != data:
                            durable(path, data)
                    before = shared.source_hashes(args.source)
                    env = dict(envs[key], JR_MATRIX_NONCE=nonce)
                    prefix = session / f'{stack}-{phase}'
                    row = dict(stack=stack, phase=phase, replicate=rep, nonce=nonce,
                               before_phase_inputs=before, cache_scope=key, cwd=str(args.source),
                               previous_stack_inputs=previous_inputs.get(stack),
                               changed_inputs_since_stack_phase=[name for name in before
                                   if before[name] != previous_inputs.get(stack, {}).get(name)],
                               environment=workload.receipt_environment(env))
                    # Persist the exact input receipt before launching the process.
                    rows.append(row)
                    save()
                    process = runtime.run_process(shared.command(tools[stack], stack,
                        arguments=arguments(phase)), args.source, env, prefix, args.timeout)
                    process['environment'] = workload.receipt_environment(env)
                    row.update(process)
                    raw, text, raw_error = captured(prefix)
                    row.update(raw=raw, raw_error=raw_error, parsed=runtime.test_results(text),
                               input_unchanged=before == shared.source_hashes(args.source))
                    previous_inputs[stack] = before
                    pair[stack] = (row, text, nonce)
                    if row['timed_out'] or row['error'] or raw_error:
                        healthy[stack] = False
                        blocked[stack] = f'previous timeout/launch/raw failure in {phase}: status={row["status"]}, error={row["error"]!r}, raw_error={raw_error!r}'
                reference = pair.get('reference')
                expected, ordinary_count = expected_summary, None
                if reference:
                    ref, text, nonce = reference
                    if phase in ('cold-prepare', 'ordinary-shim-edit', 'all-lib'):
                        expected = contract(ref['parsed'], phase) if runtime.base.succeeded(ref) else None
                        if phase == 'cold-prepare':
                            expected_summary = expected
                    if phase == 'ordinary-shim-edit':
                        found = re.findall(r'JR_WORKLOAD_EXECUTED_[^\s]*', text)
                        marker = f'JR_WORKLOAD_EXECUTED_shim_function_{rep + 1}'
                        if found and found == [marker] * len(found):
                            ordinary_count = len(found)
                    ref_errors = oracle(ref, text, expected, phase, nonce, rep, ordinary_count)
                    ref.update(failures=ref_errors, accepted=not ref_errors)
                    # Reference runtime, not just a parse, must establish each contract.
                    if ref_errors:
                        expected = None
                    if phase == 'cold-prepare' and ref_errors:
                        expected_summary = None
                    metadata['contracts'][f'{rep}:{phase}'] = dict(expected=expected,
                        ordinary_invocations=ordinary_count, reference_accepted=not ref_errors)
                for stack, (row, text, nonce) in pair.items():
                    if stack == 'reference':
                        continue
                    failures = oracle(row, text, expected, phase, nonce, rep, ordinary_count)
                    if not reference or not reference[0].get('accepted'):
                        failures.append('reference state oracle unavailable/failed')
                    row.update(failures=failures, accepted=not failures)
                save()
            restore()
    except BaseException as exc:
        metadata['error'] = repr(exc)
        raise
    finally:
        unchanged = False
        metadata['source_restored'] = False
        try:
            restore()
            metadata['source_after'] = shared.source_hashes(args.source)
            metadata['source_restored'] = metadata['source_before'] == metadata['source_after']
            metadata['tools_after'] = identity()
            metadata['toolchain_after'] = shared.toolchain_identity(args.toolchain)
            unchanged = all(metadata[k + '_before'] == metadata[k + '_after']
                            for k in ('source', 'tools', 'toolchain'))
            metadata['toolchain_links_valid'] = all(
                (Path(env['BLAZE_HOME']) / 'toolchains' / args.toolchain.name).is_symlink()
                and (Path(env['BLAZE_HOME']) / 'toolchains' / args.toolchain.name).resolve() == args.toolchain
                for env in envs.values())
            receipts = [r['raw'] for r in rows if 'raw' in r]
            receipts += [v['raw'] for v in metadata['versions'].values()]
            metadata['raw_receipts_unchanged'] = all(
                runtime.base.digest(Path(item['path'])) == item['sha256']
                for receipt in receipts for item in receipt.values())
            unchanged = (unchanged and metadata['toolchain_links_valid']
                         and metadata['raw_receipts_unchanged'] and 'error' not in metadata)
        except (OSError, ValueError) as exc:
            unchanged = False
            metadata['restoration_or_identity_error'] = repr(exc)
        results = summarize(rows, args.samples, unchanged)
        complete = unchanged and all(not s['excluded'] for p in results.values() for s in p.values())
        metadata.update(unchanged=unchanged, status='complete' if complete else 'failed/incomplete')
        save()
        (args.output / 'summary.json').write_text(json.dumps(dict(states=results, policy=POLICY,
            identities_unchanged=unchanged), indent=2) + '\n')
    return 0 if complete else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('reference-engine', 'engine', 'source', 'output', 'toolchain'):
        parser.add_argument('--' + name, type=shared.absolute, required=True)
    parser.add_argument('--proc-macro-server', type=shared.absolute)
    parser.add_argument('--samples', type=int, default=5)
    parser.add_argument('--timeout', type=float, default=360)
    raise SystemExit(execute(parser.parse_args()))


if __name__ == '__main__':
    main()
