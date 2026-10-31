#!/usr/bin/env python3
"""Read-only follow-up export, never runs native code or changes old receipts."""
import argparse
import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('demand_collector', HERE / 'collect.py')
x = importlib.util.module_from_spec(spec)
spec.loader.exec_module(x)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--negative-launches', action='store_true')
    parser.add_argument('--default-control', action='store_true')
    parser.add_argument('--regression', action='store_true')
    args = parser.parse_args()
    if sum((args.negative_launches, args.default_control, args.regression)) != 1:
        parser.error('select explicit follow-up scope')
    evidence = json.loads((HERE / 'evidence.json').read_text())
    if args.regression:
        path = Path('/tmp/jcode-bg-tasks/32891831af.output')
        data = path.read_bytes()
        x.c.emit(HERE / 'regression/root733.log', data)
        if path.read_bytes() != data:
            raise ValueError('original regression log changed')
        evidence['receipts']['regression/root733.log'] = dict(original_path=str(path), original_sha256=x.c.sha(data), retained_sha256=x.c.sha(data), omitted=dict(environment_maps=0,omitted_entries=0))
        evidence['native_source_publication'] = dict(commit='9f3bab61e',main_git_blob='ce0f6796092661bd2bfc51fca8c77b23e266c8da',policy='Later source publication, not a claim that an earlier executable embeds a later source/doc commit.')
        (HERE / 'evidence.json').write_bytes(x.c.encoded(evidence))
        return
    runs = [('rsm08', False)] if args.default_control else [('rsm06', False), ('rsm07', True)]
    for run, partial in runs:
        source = Path('/home/jeremy') / run
        transform = (x.partial_run if partial else x.c.export_run)(source, HERE / 'runs' / run)
        evidence['private_commitments'][run] = transform['private_original_sha256']
    originals = {}
    tasks = [('rsm08','60596051rv')] if args.default_control else [('rsm06','606691nr54'),('rsm07','9040157w9k')]
    for run, task in tasks:
        path = Path('/tmp/jcode-bg-tasks') / (task + '.output')
        originals[f'launches/{run}.log'] = (path,path.read_bytes())
    for path in ([] if args.default_control else sorted((x.ROOT / 'matrix-v33-post-timeout-stop').iterdir())):
        if path.name == 'receipt.json' or path.suffix in ('.stdout','.stderr'):
            originals['cleanup/' + path.name] = (path,path.read_bytes())
    if not args.default_control:
        path = x.ROOT / 'cleanup-v33-matrix-timeout.py'
        originals['provenance/' + path.name] = (path,path.read_bytes())
    outputs, secrets = {}, set()
    for name, (path, original) in originals.items():
        stats = dict(environment_maps=0, omitted_entries=0)
        data = x.c.encoded(x.c.sanitized(json.loads(original),stats,secrets)) if path.suffix == '.json' else original
        outputs[name] = data
        evidence['receipts'][name] = dict(original_path=str(path),original_sha256=x.c.sha(original),retained_sha256=x.c.sha(data),omitted=stats)
    for name,data in outputs.items():
        if any(secret in data for secret in secrets):
            raise ValueError('omitted sensitive value outside environment maps')
        x.c.emit(HERE / name,data)
    if any(path.read_bytes() != data for path,data in originals.values()):
        raise ValueError('private follow-up originals changed')
    if args.default_control:
        evidence['default_control_scope'] = 'rsm08 complete observations, failed cross-engine error gate. Same v33 binary, demand-std flag OFF, actual pinned reference. Separate original exclusions and intrinsic derivative. No general/default promotion.'
    else:
        evidence['followup_scope'] = 'rsm06 unsupported reference engine identity/command control, rsm07 incomplete outer-timeout/resource-pressure observation. Neither is attribution or a completed matrix. Scoped native stops are not universal exit attestation.'
    (HERE / 'evidence.json').write_bytes(x.c.encoded(evidence))


if __name__ == '__main__':
    main()
