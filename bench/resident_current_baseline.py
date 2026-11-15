#!/usr/bin/env python3
"""Current justrust same-host baseline for the scoped resident tier experiment.

Real runs compile only when explicitly invoked. Keeps original source untouched.
Not a general performance gate or a controlled attribution of compiler architecture.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import uuid

import resident_tier_compare as tier

shared = tier.shared
runtime = tier.runtime
matrix = tier.matrix


def environment(output):
    env = {k: v for k, v in os.environ.items() if not k.startswith(
        ('JUSTRUST_', 'JR_ENGINE_', 'RA_JIT_', 'BLAZE_', 'CARGO_', 'RUST', 'XDG_'))}
    env.update(runtime.PROFILES)
    env.update(JUSTRUST_REMOTE='off',
               JUSTRUST_LOCAL_REASON='controlled same-host resident-tier benchmark on hosted guest',
               JUSTRUST_SLOTS='0', JUSTRUST_PREBUILT='off',
               CARGO_TARGET_DIR=str(output / 'target'))
    return env


def recorded_run_id(text):
    ids = set(re.findall(r'justrust log (\d{8}-\d{9}-\d+)', text))
    if len(ids) != 1:
        raise ValueError('expected exactly one explicit recorded baseline run ID')
    return ids.pop()


def command(tool):
    return [str(tool), 'test', '--local=controlled same-host resident-tier benchmark', *shared.FILTER]


def summarize(rows, samples, identity_ok):
    result = {}
    for phase in tier.PHASES:
        selected = [r for r in rows if r['phase'] == phase]
        accepted = identity_ok and len(selected) == samples and all(r['accepted'] for r in selected)
        result[phase] = dict(samples=len(selected), accepted=accepted,
            median_seconds=statistics.median(r['elapsed_seconds'] for r in selected) if accepted else None,
            samples_seconds=[r['elapsed_seconds'] for r in selected],
            failures=[r['failures'] for r in selected if not r['accepted']])
    return result


def execute(args):
    if not 5 <= args.samples <= 30 or not math.isfinite(args.timeout) or not 0 < args.timeout <= 3600:
        raise ValueError('require 5..30 samples and finite timeout in (0,3600]')
    if args.output.exists() or args.output.is_relative_to(args.source) or args.source.is_relative_to(args.output):
        raise ValueError('output must be fresh and disjoint from source')
    shared.pinned_source(args.source)
    before = shared.source_hashes(args.source)
    tool_hash = runtime.base.digest(args.justrust)
    args.output.mkdir(parents=True, mode=0o700)
    project = args.output / 'project'
    shutil.copytree(args.source, project)
    source = project / 'src/summary.rs'
    original = source.read_bytes()
    env = environment(args.output)
    rows, receipts = [], []
    metadata = dict(schema=1, status='incomplete', pin=shared.PIN,
        scope='Current published justrust same-host filtered-test baseline, not client network latency',
        policy='Current owned compiler defaults with remote routing disabled for controlled host-only timing. '
               'Same profiles and edit recipes as resident experiment. Different compiler/backend architecture; '
               'ratios cannot be attributed solely to residency. Dependencies prepared outside measurements. '
               'Original source untouched. No claimed full-heap persistence or comprehensive workload gate.',
        tool=dict(path=str(args.justrust), sha256=tool_hash), source_before=before,
        profiles=runtime.PROFILES, environment={k: v for k, v in env.items() if k.startswith(('JUSTRUST_', 'CARGO_'))},
        host=dict(platform=platform.platform(), cpu_count=os.cpu_count(), load=os.getloadavg()),
        planned_samples=args.samples)

    def save():
        for name, value in [('metadata.json', metadata), ('records.json', rows), ('receipts.json', receipts)]:
            (args.output / name).write_text(json.dumps(value, indent=2) + '\n')

    def invoke(label, argv, call_env):
        prefix = args.output / label
        row = runtime.run_process(argv, project, call_env, prefix, args.timeout)
        raw, text, error = matrix.captured(prefix)
        row.update(raw=raw, raw_error=error, label=label)
        receipts.append(row)
        save()
        if len(argv) > 1 and argv[1] == 'test':
            # Agent console output omits passing names. Recover only this invocation's
            # full recording after timing, never combine it with duplicate console text.
            run_id = recorded_run_id(text)
            log_prefix = args.output / (label + '-recorded')
            log_row = runtime.run_process([str(args.justrust), 'log', run_id], project, env,
                                          log_prefix, args.timeout)
            log_raw, text, log_error = matrix.captured(log_prefix)
            log_row.update(raw=log_raw, raw_error=log_error, label=label + '-recorded')
            receipts.append(log_row)
            row['recorded_run'] = dict(run_id=run_id, raw=log_raw)
            save()
            if not runtime.base.succeeded(log_row) or log_error:
                raise ValueError('could not retrieve exact full baseline recording')
        return row, text

    try:
        row, text = invoke('version', [str(args.justrust), '--version'], env)
        metadata['version'] = text.strip()
        if not runtime.base.succeeded(row):
            raise ValueError('baseline version failed')
        nonce = uuid.uuid4().hex
        matrix.durable(source, tier.contents(original, 'edit', 100000, nonce))
        row, text = invoke('prepare', command(args.justrust), dict(env, JR_MATRIX_NONCE=nonce))
        row['parsed'] = runtime.test_results(text)
        errors = tier.correctness(row, text, 'edit', 100000, nonce, None)
        if errors:
            raise ValueError('baseline preparation failed: ' + repr(errors))
        expected = row['parsed']
        for rep in range(args.samples):
            for phase in tier.PHASES:
                nonce = uuid.uuid4().hex
                data = tier.contents(original, phase, rep, nonce)
                matrix.durable(source, data)
                inputs = shared.source_hashes(project)
                row, text = invoke(f'{rep}-{phase}', command(args.justrust), dict(env, JR_MATRIX_NONCE=nonce))
                row.update(phase=phase, replicate=rep, nonce=nonce,
                           source_sha256=hashlib.sha256(data).hexdigest(), parsed=runtime.test_results(text))
                errors = tier.correctness(row, text, phase, rep, nonce, expected)
                if inputs != shared.source_hashes(project):
                    errors.append('project inputs changed during command')
                row.update(accepted=not errors, failures=errors)
                rows.append(row)
                save()
                print(json.dumps(dict(phase=phase, replicate=rep, seconds=row['elapsed_seconds'], accepted=row['accepted'])), flush=True)
        identity_ok = before == shared.source_hashes(args.source) and tool_hash == runtime.base.digest(args.justrust)
        metadata.update(source_unchanged=before == shared.source_hashes(args.source),
                        tool_unchanged=tool_hash == runtime.base.digest(args.justrust))
        summary = summarize(rows, args.samples, identity_ok)
        (args.output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
        metadata['status'] = 'complete' if all(r['accepted'] for r in summary.values()) else 'failed'
        print(json.dumps(summary, indent=2), flush=True)
        return 0 if metadata['status'] == 'complete' else 1
    finally:
        matrix.durable(source, original)
        save()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--justrust', type=shared.absolute, required=True)
    parser.add_argument('--source', type=shared.absolute, required=True)
    parser.add_argument('--output', type=shared.absolute, required=True)
    parser.add_argument('--samples', type=int, default=5)
    parser.add_argument('--timeout', type=float, default=180)
    raise SystemExit(execute(parser.parse_args()))


if __name__ == '__main__':
    main()
