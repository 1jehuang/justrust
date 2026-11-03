#!/usr/bin/env python3
"""Bounded authored-candidate observation, not Rust parsing or global minimization.

Protocol: checker argv follows --, receives absolute source path as its last
argument, runs in a private attempt directory, exits 0 and emits exactly JSON:
{"stock":"valid", "native":"failure", "failure_class":"valid-source-rejected"}.
Other allowed classes: wrong-code, native-crash. Only the baseline's exact class
is preserved. Checker is trusted to perform stock validation and distinguish
compiler diagnostics from infrastructure errors. Nonzero exit, timeout, malformed
protocol, invalid stock or other classes are never bug-preserving evidence.
No retries. max-attempts includes baseline. Candidates sorted by (byte length,
SHA256, authored index). Each attempted input, stdout, stderr and result is kept.
Output must be a NEW directory beneath an explicitly supplied scratch root.
Checker is trusted executable code, not sandboxed. Use trusted commands only.
This standalone protocol is not authenticated tracker evidence. Bounds cover
attempt count and wall time, not disk usage or source/candidate-list size.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys

CLASSES = {'valid-source-rejected', 'wrong-code', 'native-crash'}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def save(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, sort_keys=True, indent=2)
        stream.write('\n')


def intact(path, source):
    return not path.is_symlink() and path.is_file() and path.read_bytes() == source


def restore(path, source):
    if path.exists() or path.is_symlink():
        path.unlink()
    with path.open('xb') as stream:
        stream.write(source)


def attempt(directory, source, command, timeout):
    directory.mkdir()
    retained = directory / 'source.rs'
    retained.write_bytes(source)
    path = directory / 'checker-input.rs'
    path.write_bytes(source)
    result = {'sha256': sha(source), 'bytes': len(source), 'status': 'infrastructure',
              'preserved': False}
    with (directory / 'stdout').open('xb') as out, (directory / 'stderr').open('xb') as err:
        try:
            process = subprocess.Popen(command + [str(path)], cwd=directory,
                                       stdout=out, stderr=err, start_new_session=True)
        except OSError as error:
            result['error'] = str(error)
        else:
            try:
                result['exit_code'] = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                result['status'] = 'timeout'
            finally:
                # Kill descendants even when the leader has already exited.
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
            if result['status'] != 'timeout' and result.get('exit_code') == 0:
                try:
                    raw = (directory / 'stdout').read_bytes()
                    if len(raw) > 65536:
                        raise ValueError('protocol too large')
                    def unique(pairs):
                        value = {}
                        for key, item in pairs:
                            if key in value:
                                raise ValueError('duplicate protocol key')
                            value[key] = item
                        return value
                    evidence = json.loads(raw, object_pairs_hook=unique)
                    if not isinstance(evidence, dict) or set(evidence) != {'stock', 'native', 'failure_class'}:
                        raise ValueError('unexpected protocol keys')
                    result['evidence'] = evidence
                    if (evidence['stock'] == 'valid' and evidence['native'] == 'failure'
                            and evidence['failure_class'] in CLASSES):
                        result['status'] = 'classified-failure'
                    else:
                        result['status'] = 'not-preserved'
                except (ValueError, TypeError, UnicodeError) as error:
                    result['error'] = str(error)
    if not intact(path, source) or not intact(retained, source):
        result['status'] = 'infrastructure'
        result['error'] = 'checker modified input or retained source'
        restore(retained, source)
    return result


def run(original, candidates, output, scratch_root, command, max_attempts, timeout):
    if (type(max_attempts) is not int or max_attempts < 1
            or type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0):
        raise ValueError('positive finite bounds required')
    if (not isinstance(command, list) or not command
            or any(not isinstance(arg, str) or '\x00' in arg for arg in command)):
        raise ValueError('checker argv must be a nonempty string list')
    root = Path(scratch_root).resolve(strict=True)
    output = Path(output).absolute()
    parent = output.parent.resolve(strict=True)
    if parent != root and root not in parent.parents:
        raise ValueError('output must be beneath scratch root')
    output = parent / output.name
    # Snapshot all explicit inputs before executing untrusted-by-content sources.
    baseline = Path(original).read_bytes()
    authored = [(Path(path).read_bytes(), str(Path(path).absolute()), i)
                for i, path in enumerate(candidates)]
    authored.sort(key=lambda row: (len(row[0]), sha(row[0]), row[2]))
    output.mkdir()  # Exclusive, never overwrite or reuse an existing output.
    (output / 'original.rs').write_bytes(baseline)
    save(output / 'inputs.json', {'original': str(Path(original).absolute()),
         'original_sha256': sha(baseline), 'checker_argv': command,
         'max_attempts': max_attempts, 'timeout_seconds': timeout,
         'candidates': [{'path': p, 'sha256': sha(b), 'bytes': len(b), 'authored_index': i}
                        for b, p, i in authored]})
    first = attempt(output / 'attempt-0000', baseline, command, timeout)
    if not intact(output / 'original.rs', baseline):
        first['status'] = 'infrastructure'
        first['error'] = 'checker modified original snapshot'
        restore(output / 'original.rs', baseline)
    valid_baseline = first['status'] == 'classified-failure'
    first['preserved'] = valid_baseline
    save(output / 'attempt-0000' / 'result.json', first)
    best = {'path': 'original.rs', 'sha256': sha(baseline), 'bytes': len(baseline)} if valid_baseline else None
    count = 1
    if valid_baseline:
        for source, path, index in authored[:max_attempts - 1]:
            name = f'attempt-{count:04d}'
            result = attempt(output / name, source, command, timeout)
            if not intact(output / 'original.rs', baseline):
                result['status'] = 'infrastructure'
                result['error'] = 'checker modified original snapshot'
                restore(output / 'original.rs', baseline)
            result.update({'authored_index': index, 'input_path': path})
            result['preserved'] = (result['status'] == 'classified-failure'
                                   and result['evidence'] == first['evidence'])
            save(output / name / 'result.json', result)
            count += 1
            if result['preserved'] and len(source) < best['bytes']:
                best = {'path': name + '/source.rs', 'sha256': sha(source), 'bytes': len(source)}
    report = {'schema_version': 1, 'attempts': count, 'baseline_preserved': valid_baseline,
              'original_sha256': sha(baseline), 'best_observed_reproducer': best,
              'claim': 'finite authored candidates only, not globally minimal',
              'unattempted_candidates': len(authored) - (count - 1)}
    save(output / 'report.json', report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--original', required=True)
    parser.add_argument('--candidate', action='append', default=[])
    parser.add_argument('--scratch-root', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--max-attempts', type=int, required=True)
    parser.add_argument('--timeout', type=float, required=True)
    parser.add_argument('checker', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.checker[1:] if args.checker[:1] == ['--'] else args.checker
    try:
        report = run(args.original, args.candidate, args.output, args.scratch_root,
                     command, args.max_attempts, args.timeout)
    except (OSError, ValueError) as error:
        parser.exit(2, f'{error}\n')
    print(json.dumps(report, sort_keys=True))
    return 0 if report['baseline_preserved'] else 1


if __name__ == '__main__':
    sys.exit(main())
