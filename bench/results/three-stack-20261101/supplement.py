#!/usr/bin/env python3
"""Read-only private input review, emitting only additive public evidence JSON."""
import argparse
import hashlib
import json
from pathlib import Path


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def review(source):
    raw = (source / 'journal.jsonl').read_bytes()
    entries = [json.loads(line) for line in raw.splitlines()]
    h, f = entries[0]['data'], entries[-1]['data']
    if (h['source_before'] != f['source_after'] or h['identities'] != f['identities_after']
            or f['status'] != 'complete' or not f['unchanged']
            or len(f['restoration']) != 63 or not all(v is True for v in f['restoration'].values())):
        raise ValueError('original private before/after restoration identities differ')
    mapping = {h['source']: '/SOURCE', h['original_output']: '/RUN',
               h['identities']['toolchain']['path']: '/TOOLCHAIN', str(source.parent): '/HOME'}
    for name, tool in h['identities']['tools'].items():
        if name in ('blaze', 'justrust'):
            mapping[tool['path']] = '/TOOLS/' + name
    def project(value):
        if isinstance(value, str):
            for old, new in sorted(mapping.items(), key=lambda pair: -len(pair[0])):
                value = value.replace(old, new)
        elif isinstance(value, list):
            return [project(v) for v in value]
        elif isinstance(value, dict):
            return {k: project(v) for k, v in value.items()}
        return value
    rows = []
    for sample in range(6):
        home = source / f'r{sample}/justrust/jr'
        for path in sorted((home / 'runs').iterdir()):
            meta_raw = (path / 'meta.json').read_bytes()
            meta = json.loads(meta_raw)
            selected = {k: meta[k] for k in ('id', 'args', 'subcommand', 'place', 'route', 'real_cargo', 'real_rustc') if k in meta}
            output = path / 'output.jsonl'
            rows.append({'sample': sample, 'kind': 'background-prebuild' if path.name.endswith('-pre') else 'measured',
                'metadata': project(selected), 'original_meta_sha256': sha(meta_raw), 'original_meta_bytes': len(meta_raw),
                'original_output_sha256': sha(output.read_bytes()) if output.is_file() else None,
                'original_output_bytes': output.stat().st_size if output.is_file() else None,
                'transformation': 'allowlist metadata fields, no env or private path values, background output retained privately with hash/size only'})
    result = {'schema': h['schema'], 'original_journal_sha256': sha(raw),
        'original_final_chain_sha256': entries[-1]['sha256'],
        'original_source_before_sha256': h['source_before'], 'original_source_after_sha256': f['source_after'],
        'original_identity_before_sha256': sha(canonical(h['identities'])),
        'original_identity_after_sha256': sha(canonical(f['identities_after'])),
        'source_restoration_all63': f['restoration'], 'original_complete_unchanged': True,
        'measured_recordings': sum(r['kind'] == 'measured' for r in rows),
        'background_prebuild_recordings': sum(r['kind'] == 'background-prebuild' for r in rows),
        'total_recordings': len(rows), 'recordings': rows,
        'scope': 'post-run read-only inventory and original before/after identity check, not independent host attestation; no native workload invoked'}
    if sha((source / 'journal.jsonl').read_bytes()) != sha(raw):
        raise ValueError('private original journal changed during review')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(review(args.source), indent=2))
