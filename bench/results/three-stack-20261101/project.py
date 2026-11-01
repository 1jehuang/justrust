#!/usr/bin/env python3
"""Build a privacy-projected evidence derivative from explicit private inputs.

This never starts subprocesses or network requests. Run only on the owner's
private machine. Original evidence is read-only. Public source provenance is
retained as hashes even when its fixture path strings are privacy-redacted.
"""
import argparse
import copy
import gzip
import hashlib
import json
from pathlib import Path


def encoded(v):
    return json.dumps(v, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()


def sha(b):
    return hashlib.sha256(b).hexdigest()


def build(args):
    src, out = args.source, args.destination
    if (out / 'COMMITMENT.json').exists() or (out / 'journal.jsonl.gz').exists():
        raise ValueError('evidence destination already frozen; choose a distinct fresh directory')
    entries = [json.loads(line) for line in (src / 'journal.jsonl').read_bytes().splitlines()]
    original_journal = (src / 'journal.jsonl').read_bytes()
    header = entries[0]['data']
    previous = None
    for entry in entries:
        claim = entry['sha256']
        unsigned = {k: v for k, v in entry.items() if k != 'sha256'}
        if entry['previous'] != previous or sha(encoded(unsigned)) != claim:
            raise ValueError('original journal hash chain invalid')
        previous = claim
    if entries[-1]['data']['status'] != 'complete':
        raise ValueError('original run is incomplete')
    home = str(src.parent)
    mapping = {header['original_output']: '/RUN', header['source']: '/SOURCE',
               header['identities']['toolchain']['path']: '/TOOLCHAIN'}
    for name, identity in header['identities']['tools'].items():
        if not identity['path'].startswith(header['identities']['toolchain']['path'] + '/'):
            mapping[identity['path']] = '/TOOLS/' + name
    mapping[str(args.prep)] = '/PREPARATION'
    mapping[home] = '/HOME'
    def text_project(text):
        for old, new in sorted(mapping.items(), key=lambda item: -len(item[0])):
            text = text.replace(old, new)
        return text
    def project(v):
        if isinstance(v, str):
            return text_project(v)
        if isinstance(v, list):
            return [project(x) for x in v]
        if isinstance(v, dict):
            return {text_project(k): project(x) for k, x in v.items()}
        return v
    inventory = {}
    def put(relative, raw, transformed=None, transformation='unchanged'):
        data = raw if transformed is None else transformed
        dest = out / relative
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(data)
        inventory[relative] = {'original_sha256': sha(raw), 'original_bytes': len(raw),
                               'projected_sha256': sha(data), 'projected_bytes': len(data),
                               'transformation': transformation}
        return {'file': relative, 'sha256': sha(data)}
    refs = {}
    # Explicit allowlist only, never recursively copy build/cache/daemon stores.
    for entry in entries:
        data = entry['data']
        candidates = []
        if entry['kind'] in ('process', 'outcome', 'cleanup'):
            candidates += list(data.get('raw', {}).values())
            candidates += list(data.get('recorded', {}).values())
        if entry['kind'] == 'backups':
            candidates += [r['artifact'] for r in data.values()]
        for ref in candidates:
            name = ref['file']
            if name in refs:
                continue
            rel = Path(name)
            if rel.is_absolute() or '..' in rel.parts:
                raise ValueError('unsafe original artifact reference')
            raw = (src / rel).read_bytes()
            if sha(raw) != ref['sha256']:
                raise ValueError('original artifact hash changed: ' + name)
            transformed = text_project(raw.decode('utf-8')).encode()
            note = 'literal private-path prefixes replaced by stable absolute placeholders; all output lines retained'
            if name.endswith('.meta.json'):
                meta = json.loads(raw)
                keys = ('id', 'args', 'subcommand', 'cwd', 'start', 'real_cargo', 'real_rustc',
                        'rustc_wrapper', 'tty', 'ncpu', 'mem_total_mb', 'env', 'slot', 'route', 'place', 'toolchain')
                selected = {k: meta[k] for k in keys if k in meta}
                selected['env'] = {k: v for k, v in selected.get('env', {}).items()
                                   if k in ('CARGO_PROFILE_DEV_DEBUG', 'CARGO_TARGET_DIR')}
                transformed = encoded(project(selected)) + b'\n'
                note = 'allowlist metadata fields; omit PID/agent_session; env only CARGO_PROFILE_DEV_DEBUG/CARGO_TARGET_DIR; literal private-path projection'
            refs[name] = put(name, raw, transformed, note if transformed != raw else 'unchanged UTF-8 bytes')
    projected = project(copy.deepcopy(entries))
    source = {}
    backups = next(e['data'] for e in projected if e['kind'] == 'backups')
    for name, item in backups.items():
        item['artifact'] = refs[item['artifact']['file']]
        source[name] = item['artifact']['sha256']
    h = projected[0]['data']
    original_source = dict(header['source_before'])
    h['source_before'] = source
    ident_hash = sha(encoded(h['identities']))
    seed_hash = sha(encoded(h['seed']))
    envs = {}
    for entry in projected:
        d = entry['data']
        if entry['kind'] == 'state':
            envs[d['sample'], d['stack']] = d['environment']
        if entry['kind'] in ('intent', 'process', 'outcome'):
            changed_summary = d['source']['src/summary.rs']
            d['source'] = dict(source, **{'src/summary.rs': changed_summary})
            d['identities_sha256'] = ident_hash
            d['seed_sha256'] = seed_hash
            d['environment_sha256'] = sha(encoded(envs[d['sample'], d['stack']]))
        if entry['kind'] in ('process', 'outcome', 'cleanup'):
            if 'raw' in d:
                d['raw'] = {k: refs[v['file']] for k, v in d['raw'].items()}
            if 'recorded' in d:
                d['recorded'] = {k: refs[v['file']] for k, v in d['recorded'].items()}
        if entry['kind'] == 'footer':
            d['source_after'] = source
    previous = None
    journal = b''
    for entry in projected:
        entry['previous'] = previous
        entry.pop('sha256', None)
        entry['sha256'] = sha(encoded(entry))
        previous = entry['sha256']
        journal += encoded(entry) + b'\n'
    compressed = gzip.compress(journal, mtime=0)
    put('journal.jsonl.gz', original_journal, compressed,
        'lossless gzip of derivative journal; projected paths/artifact hashes/source-projection hashes and dependent chain digests recomputed; original unchanged privately')
    put('summary.json', (src / 'summary.json').read_bytes())
    put('machine-and-provenance.json', (args.prep / 'machine-and-provenance.json').read_bytes(),
        encoded(project(json.loads((args.prep / 'machine-and-provenance.json').read_text()))) + b'\n', 'path-projected JSON')
    launch = args.prep / args.launch_name
    put('launch.sh.txt', launch.read_bytes(),
        text_project(launch.read_text()).encode(), 'private paths replaced; text only, never executed')
    for name, expected in header['identities']['harness'].items():
        origin = args.frozen / name if (args.frozen / name).is_file() else args.helpers / name
        raw = origin.read_bytes()
        if sha(raw) != expected:
            raise ValueError('measured frozen closure changed: ' + name)
        put('frozen/' + name, raw)
    log = args.frozen / 'tests.log'
    if log.is_file():
        raw = log.read_bytes()
        put('validation/frozen-mocked-tests.log', raw, text_project(raw.decode()).encode(), 'private-path projection only')
    recording_proof = []
    for sample in range(6):
        home = src / f'r{sample}/justrust/jr'
        recordings = []
        for folder in sorted((home / 'runs').iterdir()):
            meta = json.loads((folder / 'meta.json').read_text())
            recordings.append({'id': folder.name, 'args': meta.get('args'),
                'place': meta.get('place'), 'route': meta.get('route'),
                'meta_original_sha256': sha((folder / 'meta.json').read_bytes()),
                'background_prebuild': folder.name.endswith('-pre')})
        recording_proof.append({'sample': sample, 'recordings': recordings,
            'count': len(recordings), 'buildd_entries': sorted(p.name for p in home.glob('buildd*'))})
    put('recording-inventory.json', encoded(recording_proof) + b'\n',
        encoded(project(recording_proof)) + b'\n',
        'read-only post-run private recording inventory, args/place/route/meta original hashes only, no recursive HOME copy')
    for name in args.context:
        raw = (args.prep / name).read_bytes()
        put('context/' + name, raw, text_project(raw.decode()).encode(),
            'coordinator-supplied context, path projection only; attributed not contemporaneous worker observation')
    original_identities = project(header['identities'])
    provenance = {'schema': 'three-stack-portable-evidence-1',
        'original_journal_sha256': sha(original_journal), 'original_journal_bytes': len(original_journal),
        'original_final_chain_sha256': entries[-1]['sha256'],
        'original_source63_sha256': original_source, 'original_identities': original_identities,
        'original_seed_manifest': header['seed'],
        'derivative_journal_uncompressed_sha256': sha(journal),
        'transformation_policy': 'explicit artifact-reference allowlist; no cache/target/home trees, credentials, tokens or inherited environment copied; stable path placeholders /RUN /SOURCE /TOOLCHAIN /TOOLS /PREPARATION /HOME; source fixtures projected when necessary but original hashes retained',
        'trust': 'Consistency and fixed version-controlled derivative commitment, not cryptographic host attestation. Original private journal remains immutable and is not published.',
        'preflight_history': {'149183jbgm': 'raw log not supplied; do not infer outcome',
            '16908574mg': 'root reported seed CACHEDIR.TAG rejection before native execution; raw log not supplied',
            '230717aa6k': 'root reported legal public module path rejection before native execution; raw log not supplied'},
        'pilot': 'workflow pilot 339382hn3n excluded from comparative statistics; raw pilot evidence not supplied to this archive',
        'validation_history': 'v90 mocked log retained; coordinator reports 800 full benchmark Python tests PASS, raw full-suite log not supplied',
        'inventory': inventory}
    (out / 'PROJECTION.json').write_bytes(encoded(provenance) + b'\n')
    committed = {str(p.relative_to(out)): sha(p.read_bytes()) for p in sorted(out.rglob('*'))
                 if p.is_file() and p.name not in ('project.py', 'validate.py', 'test_archive.py', 'COMMITMENT.json')}
    commitment = encoded({'schema': 'three-stack-fixed-derivative-1', 'original_journal_sha256': sha(original_journal),
                          'files': committed}) + b'\n'
    (out / 'COMMITMENT.json').write_bytes(commitment)
    print('COMMITMENT_SHA256=' + sha(commitment))
    print('ORIGINAL_JOURNAL_SHA256=' + sha(original_journal))
    print('FILES=' + str(len(committed)))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'destination', 'prep', 'frozen', 'helpers'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--launch-name', default='run-three-stack.sh')
    parser.add_argument('--context', action='append', default=[])
    build(parser.parse_args())
