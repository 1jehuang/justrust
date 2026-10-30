#!/usr/bin/env python3
"""Broader pinned resident diagnostics, not the comprehensive/default gate."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import statistics

import resident_engine_compare as shared

runtime = shared.runtime
STACKS = ['reference', 'engine']
ENV_RECEIPT_ALLOWLIST = frozenset((
    'HOME', 'JR_ENGINE_HOME', 'RA_JIT_HOME', 'RA_JIT_CACHE', 'CARGO_HOME',
    'CARGO_TARGET_DIR', 'RUSTUP_HOME', 'BLAZE_HOME', 'BLAZE_CACHE',
    'BLAZE_BACKEND', 'BLAZE_OPT', 'BLAZE_REMOTE', 'RA_JIT_REMOTE',
    'BLAZE_PROC_MACRO_SERVER', 'RA_JIT_PROC_MACRO_SERVER',
    'XDG_RUNTIME_DIR', 'XDG_CACHE_HOME', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME',
)) | frozenset(runtime.PROFILES)


def receipt_environment(env):
    """Log only explicit build controls, never the inherited ambient environment."""
    return {key: env[key] for key in sorted(ENV_RECEIPT_ALLOWLIST) if key in env}


# Anchors inspected using git show f97fcc2:src/{summary,shim,record}.rs.
WORKLOADS = {
    'summary-test': ('src/summary.rs', 'summary::tests::',
                     'fn shares_overlapping_time() {', 'shares_overlapping_time', 'test'),
    'shim-test': ('src/shim.rs', 'shim::tests::',
                  'fn describes_cargo_rustc_args() {', 'describes_cargo_rustc_args', 'test'),
    'record-test': ('src/record.rs', 'record::tests::',
                    'fn finds_subcommand_after_global_flags() {',
                    'finds_subcommand_after_global_flags', 'test'),
    'shim-function': ('src/shim.rs', 'shim::tests::',
                      'fn parse_pass(json: &[u8]) -> Option<Pass> {',
                      'parses_time_passes_json', 'ordinary'),
}


def edited(original, workload, phase, rep=0):
    anchor = WORKLOADS[workload][2].encode()
    if original.count(anchor) != 1:
        raise ValueError('exact function anchor must occur once: ' + workload)
    marker = f'JR_WORKLOAD_EXECUTED_{workload.replace("-", "_")}_{rep + 1}'
    if phase == 'type-error':
        body = 'let _: u64 = "JR_WORKLOAD_TYPE_ERROR";'
    elif phase == 'assert-failure':
        body = 'assert_eq!(std::hint::black_box(1u64), 2u64, "JR_WORKLOAD_FAILURE");'
    elif phase == 'edit':
        body = (f'let observed = std::hint::black_box({rep + 1}u64); '
                f'assert_eq!(observed, {rep + 1}u64); println!("{marker}");')
    else:
        return original
    return original.replace(anchor, anchor + b'\n' + body.encode(), 1)


def valid(parsed, expected, failing=None):
    if not parsed or not expected:
        return False
    names = [name for name, _ in expected['tests']]
    wanted = [(n, 'FAILED' if n == failing else 'ok') for n in names]
    return (parsed['tests'] == wanted and parsed['passed'] == len(names) - bool(failing)
            and parsed['failed'] == bool(failing) and parsed['ignored'] == 0
            and parsed['filtered'] == expected['filtered'])


def discover(parsed, workload):
    prefix = WORKLOADS[workload][1]
    if (not parsed or not parsed['tests'] or parsed['failed'] or parsed['ignored']
            or not all(n.startswith(prefix) and s == 'ok' for n, s in parsed['tests'])
            or prefix + WORKLOADS[workload][3] not in dict(parsed['tests'])):
        return None
    return parsed


def execute(args):
    if args.samples < 1 or not math.isfinite(args.timeout) or args.timeout <= 0:
        raise ValueError('samples and finite timeout must be positive')
    selected = args.workload or list(WORKLOADS)
    if len(set(selected)) != len(selected) or any(w not in WORKLOADS for w in selected):
        raise ValueError('invalid or duplicate workload')
    if (args.source.name != 'source63' or args.output.exists()
            or args.output.is_relative_to(args.source)
            or args.source.is_relative_to(args.output) or len(os.fsencode(args.output)) > 70):
        raise ValueError('use pristine source63 and fresh short disjoint output')
    shared.pinned_source(args.source)
    tools = {'reference': args.reference_engine, 'engine': args.engine}
    if args.proc_macro_server:
        tools['proc-macro-server'] = args.proc_macro_server
    for tool in tools.values():
        with tool.open('rb') as stream:
            magic = stream.read(4)
        if not os.access(tool, os.X_OK) or magic != b'\x7fELF':
            raise ValueError('native executable ELF inputs required')
    tool_identity = lambda: {s: {'path': str(p), 'sha256': runtime.base.digest(p)}
                             for s, p in tools.items()}
    metadata = dict(pin=shared.PIN, scope=__doc__, samples=args.samples,
                    workloads={w: WORKLOADS[w] for w in selected}, status='incomplete',
                    source_before=shared.source_hashes(args.source),
                    tools_before=tool_identity(),
                    toolchain_before=shared.toolchain_identity(args.toolchain),
                    contracts={}, environments={}, profiles=runtime.PROFILES,
                    environment_receipt_policy='explicit allowlist of build controls and private paths; ambient variables omitted',
                    store_policy='unseeded private stores per stack, reused across workloads',
                    proc_macro_policy='same explicit frozen native server' if args.proc_macro_server
                    else 'both server variables explicitly 0, rustc-built proc macros')
    args.output.mkdir(parents=True, mode=0o700)
    originals = {WORKLOADS[w][0]: (args.source / WORKLOADS[w][0]).read_bytes()
                 for w in selected}
    backups = {}
    for index, (name, contents) in enumerate(originals.items()):
        backup = args.output / f'original-{index}.rs'
        with backup.open('xb') as stream:
            stream.write(contents)
            stream.flush()
            os.fsync(stream.fileno())
        backups[name] = backup
    metadata['backups'] = {name: {'path': str(backup),
                           'sha256': runtime.base.digest(backup)}
                           for name, backup in backups.items()}
    rows, medians, envs, version_ok = [], {}, {}, {}

    def save():
        for filename, value in [('metadata.json', metadata), ('records.json', rows)]:
            (args.output / filename).write_text(json.dumps(value, indent=2) + '\n')

    def invoke(stack, workload, phase, rep=0, contents=None, cmd=None):
        if contents is not None:
            (args.source / WORKLOADS[workload][0]).write_bytes(contents)
        prefix = args.output / f'{stack}-{workload}-{phase}-{rep}'
        arguments = ['--lib', '--', WORKLOADS[workload][1], '--test-threads=1', '--nocapture']
        command = cmd or shared.command(tools[stack], stack, arguments=arguments)
        input_digest = runtime.base.digest(args.source / WORKLOADS[workload][0])
        row = runtime.run_process(command, args.source, envs[stack], prefix, args.timeout)
        row['environment'] = receipt_environment(envs[stack])
        text = '\n'.join(prefix.with_suffix('.' + suffix).read_text(errors='replace')
                         for suffix in ('stdout', 'stderr'))
        row.update(stack=stack, workload=workload, phase=phase, replicate=rep,
                   parsed=runtime.test_results(text), raw_prefix=str(prefix),
                   input_sha256=input_digest,
                   source_sha256=hashlib.sha256((args.source / WORKLOADS[workload][0]).read_bytes()).hexdigest())
        row['input_unchanged'] = row['input_sha256'] == row['source_sha256']
        rows.append(row)
        save()
        return row, text

    def good(row, expected):
        return row['input_unchanged'] and runtime.base.succeeded(row) and valid(row['parsed'], expected)

    try:
        for stack in STACKS:
            envs[stack] = shared.environment(args.output, stack, args.toolchain,
                                             args.proc_macro_server)
            metadata['environments'][stack] = receipt_environment(envs[stack])
            row, text = invoke(stack, selected[0], 'version',
                               cmd=[str(tools[stack]), '--version'])
            pattern = (re.escape(shared.REFERENCE_VERSION) + '$' if stack == 'reference'
                       else r'jr-engine\s+\S+')
            version_ok[stack] = runtime.base.succeeded(row) and bool(re.search('^' + pattern, text, re.M))
        for workload in selected:
            path, prefix, _, failing, _ = WORKLOADS[workload]
            original = originals[path]
            expected, active = None, []
            for stack in STACKS:
                if not version_ok[stack]:
                    continue
                row, text = invoke(stack, workload, 'prepare', contents=original)
                if stack == 'reference' and row['input_unchanged'] and runtime.base.succeeded(row):
                    expected = discover(row['parsed'], workload)
                    metadata['contracts'][workload] = expected
                if not good(row, expected):
                    continue
                row, text = invoke(stack, workload, 'type-error',
                                   contents=edited(original, workload, 'type-error'))
                type_ok = (row['input_unchanged'] and row['status'] == 101 and not row['error'] and not row['timed_out']
                           and 'E0308' in text and 'JR_WORKLOAD_TYPE_ERROR' in text
                           and row['parsed'] is None)
                row['accepted'] = type_ok
                recovery, _ = invoke(stack, workload, 'type-recovery', contents=original)
                row, text = invoke(stack, workload, 'assert-failure',
                                   contents=edited(original, workload, 'assert-failure'))
                fail_ok = (row['input_unchanged'] and row['status'] == 101 and not row['error'] and not row['timed_out']
                           and valid(row['parsed'], expected, prefix + failing)
                           and 'JR_WORKLOAD_FAILURE' in text)
                row['accepted'] = fail_ok
                recovered, _ = invoke(stack, workload, 'assert-recovery', contents=original)
                if type_ok and fail_ok and good(recovery, expected) and good(recovered, expected):
                    active.append(stack)
            for rep in range(args.samples):
                contents = edited(original, workload, 'edit', rep)
                for stack in runtime.order(STACKS, rep):
                    if stack not in active:
                        continue
                    row, text = invoke(stack, workload, 'edit', rep, contents)
                    marker = f'JR_WORKLOAD_EXECUTED_{workload.replace("-", "_")}_{rep + 1}'
                    row['accepted'] = good(row, expected) and re.findall(r'JR_WORKLOAD_EXECUTED_[^\s]*', text) == [marker]
            for stack in active:
                measured = [r for r in rows if r['stack'] == stack
                            and r['workload'] == workload and r['phase'] == 'edit']
                if len(measured) == args.samples and all(r['accepted'] for r in measured):
                    medians.setdefault(workload, {})[stack] = statistics.median(
                        r['elapsed_seconds'] for r in measured)
            (args.source / path).write_bytes(backups[path].read_bytes())
    except BaseException as exc:
        metadata['error'] = repr(exc)
        raise
    finally:
        metadata['source_restored'] = False
        for name, backup in backups.items():
            (args.source / name).write_bytes(backup.read_bytes())
        try:
            metadata['source_after'] = shared.source_hashes(args.source)
            metadata['source_restored'] = metadata['source_before'] == metadata['source_after']
            metadata['tools_after'] = tool_identity()
            metadata['toolchain_after'] = shared.toolchain_identity(args.toolchain)
            unchanged = all(metadata[k + '_before'] == metadata[k + '_after']
                            for k in ('source', 'tools', 'toolchain'))
            links = [Path(e['BLAZE_HOME']) / 'toolchains' / args.toolchain.name
                     for e in envs.values()]
            unchanged = unchanged and all(link.is_symlink() and link.resolve() == args.toolchain
                                          for link in links)
        except (OSError, ValueError) as exc:
            metadata['identity_error'] = repr(exc)
            unchanged = False
        if not unchanged or 'error' in metadata:
            medians.clear()
        complete = unchanged and all(len(medians.get(w, {})) == 2 for w in selected)
        metadata.update(unchanged=unchanged, status='complete' if complete else 'failed/incomplete')
        save()
        (args.output / 'summary.json').write_text(json.dumps(dict(
            medians_seconds=medians, excluded={w: [s for s in STACKS if s not in medians.get(w, {})]
                                               for w in selected},
            policy='Descriptive scoped workloads only, not comprehensive/default promotion. Failed stacks excluded.'), indent=2) + '\n')
    return 0 if complete else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('reference-engine', 'engine', 'source', 'output', 'toolchain'):
        parser.add_argument('--' + name, type=shared.absolute, required=True)
    parser.add_argument('--proc-macro-server', type=shared.absolute)
    parser.add_argument('--workload', action='append', choices=WORKLOADS,
                        help='repeat to select workloads, default all four')
    parser.add_argument('--samples', type=int, default=5)
    parser.add_argument('--timeout', type=float, default=360)
    raise SystemExit(execute(parser.parse_args()))


if __name__ == '__main__':
    main()
