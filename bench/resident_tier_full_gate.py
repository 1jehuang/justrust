#!/usr/bin/env python3
"""Execute a full pinned-library correctness gate, not a timed speed claim."""
import argparse
import json

import resident_current_baseline as baseline
import resident_tier_compare as tier


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ('engine', 'justrust', 'source', 'toolchain', 'output'):
        p.add_argument('--' + name, required=True, type=tier.shared.absolute)
    a = p.parse_args()
    tier.shared.pinned_source(a.source)
    if a.output.exists() or a.output.is_relative_to(a.source) or a.source.is_relative_to(a.output):
        raise ValueError('fresh disjoint output required')
    a.output.mkdir(mode=0o700)
    before = tier.shared.source_hashes(a.source)
    env = tier.shared.environment(a.output, 'engine', a.toolchain, None)
    env.update(RA_JIT_MEMORY_MAX='off', RA_JIT_SPARE='0', RA_JIT_IDLE_SECS='3600')
    normal = baseline.environment(a.output / 'current')
    rows = []
    report = dict(status='incomplete', scope='Exact full-lib test contract on pinned source; no performance claim',
                  source_before=before, tools={k: tier.runtime.base.digest(v) for k, v in
                    [('engine', a.engine), ('justrust', a.justrust)]}, records=rows)

    def invoke(label, argv, call_env):
        prefix = a.output / label
        row = tier.runtime.run_process(argv, a.source, call_env, prefix, 180)
        raw, text, error = tier.matrix.captured(prefix)
        row.update(raw=raw, raw_error=error, label=label)
        rows.append(row)
        if error:
            raise ValueError(error)
        return row, text

    def save():
        (a.output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')

    try:
        row, console = invoke('current', [str(a.justrust), 'test', '--local=controlled full-library correctness gate',
            '--lib', '--', '--test-threads=1', '--nocapture'], normal)
        run_id = baseline.recorded_run_id(console)
        full, text = invoke('current-recorded', [str(a.justrust), 'log', run_id], normal)
        expected = tier.runtime.test_results(text)
        report.update(current_run_id=run_id, current_tests=expected)
        if row['status'] != 0 or not tier.runtime.base.succeeded(full) or not expected or expected['failed']:
            raise ValueError('current full-lib gate failed')
        native, text = invoke('resident', [str(a.engine), 'test', '--lib', '--', '--test-threads=1', '--nocapture'], env)
        actual = tier.runtime.test_results(text)
        report['resident_tests'] = actual
        if not tier.runtime.base.succeeded(native) or actual != expected:
            raise ValueError('native exact full-library test parity failed')
        report['status'] = 'passed'
    except BaseException as exc:
        report.update(status='failed', error=repr(exc))
        raise
    finally:
        try:
            stop, _ = invoke('stop', [str(a.engine), 'stop'], env)
            report['cleanup_succeeded'] = tier.runtime.base.succeeded(stop)
            if not report['cleanup_succeeded']:
                report['status'] = 'failed'
        finally:
            report['source_unchanged'] = before == tier.shared.source_hashes(a.source)
            report['tools_unchanged'] = report['tools'] == {k: tier.runtime.base.digest(v) for k, v in
                [('engine', a.engine), ('justrust', a.justrust)]}
            if not report['source_unchanged'] or not report['tools_unchanged']:
                report['status'] = 'failed'
            save()
    print(json.dumps({k: v for k, v in report.items() if k not in ('source_before', 'records')}, indent=2))
    return 0 if report['status'] == 'passed' else 1


if __name__ == '__main__':
    raise SystemExit(main())
