#!/usr/bin/env python3
"""Offline archival copy only. Never executes any recorded workload/runner."""
import gzip
import hashlib
import json
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parent
PHYSICAL = Path('/home/jeremy/.jcode/scratch/dp47')
SOURCE = Path('/home/jeremy/.jcode/scratch/ba30/source')
RUNNER = Path('/home/jeremy/.jcode/scratch/ba30')
REPO = ROOT.parents[2]


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def copy(source, relative):
    destination = ROOT / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    assert not destination.exists(), relative
    shutil.copyfile(source, destination)
    assert destination.read_bytes() == source.read_bytes()


def main():
    state = json.loads((PHYSICAL / 'state.json').read_text())
    assert state['status'] == 'completed'
    assert state['source_before'] == state['source_after']
    assert state['identities_before'] == state['identities_after']
    copy(PHYSICAL / 'state.json', 'state.json')
    copy(PHYSICAL / 'summary.rs.backup', 'source.original')
    for name, sha in state['source_before'].items():
        assert not Path(name).is_absolute() and '..' not in Path(name).parts
        assert digest((SOURCE / name).read_bytes()) == sha, name
        copy(SOURCE / name, 'source/' + name)
    original = (ROOT / 'source.original').read_bytes()
    anchor = b'fn shares_overlapping_time() {'
    assert original.count(anchor) == 1
    edited = original.replace(anchor, anchor + b'\nlet observed = std::hint::black_box(47u64); assert_eq!(observed, 47u64); println!("JR_RUNTIME_EXECUTED_47");', 1)
    (ROOT / 'source.edited').write_bytes(edited)
    for name in ('profile-cgu47.py', 'launch-profile47.sh', 'profile47.stdout', 'profile47.stderr', 'profile47.exit', 'profile47.launch.log'):
        copy(RUNNER / name, 'runner/' + name)
    copy(REPO / 'bench/query_profile.py', 'decoder/query_profile.py')
    profiles = []
    for row in state['rows']:
        key = row['stack'] + '-' + row['phase']
        assert len(row['profiles']) == 1
        profile = row['profiles'][0]
        raw = (PHYSICAL / profile['path']).read_bytes()
        assert digest(raw) == profile['sha256']
        path = 'profiles/' + key + '.mm_profdata.gz'
        compressed = gzip.compress(raw, compresslevel=1, mtime=0)
        (ROOT / 'profiles').mkdir(exist_ok=True)
        (ROOT / path).write_bytes(compressed)
        assert gzip.decompress(compressed) == raw
        profiles.append(dict(key=key, path=path, raw_sha256=digest(raw), raw_bytes=len(raw), gzip_sha256=digest(compressed), gzip_bytes=len(compressed)))
        for suffix in ('stdout', 'stderr', 'raw-output.jsonl'):
            copy(PHYSICAL / (key + '.' + suffix), 'logs/' + key + '.' + suffix)
        run = Path(row['environment']['JUSTRUST_HOME']) / 'runs' / row['raw_output']['run_id']
        for artifact in sorted(run.iterdir()):
            assert artifact.is_file() and not artifact.is_symlink()
            copy(artifact, 'journals/' + key + '/' + artifact.name)
    (ROOT / 'profiles.json').write_text(json.dumps(profiles, indent=2) + '\n')
    print(json.dumps(dict(rows=len(profiles), source_files=len(state['source_before']), raw_profile_bytes=sum(p['raw_bytes'] for p in profiles), gzip_profile_bytes=sum(p['gzip_bytes'] for p in profiles)), indent=2))


if __name__ == '__main__':
    main()
