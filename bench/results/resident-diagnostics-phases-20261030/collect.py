#!/usr/bin/env python3
"""Copy completed diagnostics evidence only. Never reads live candidate source."""
import hashlib
import json
import shutil
import subprocess
from pathlib import Path

ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
OUT = Path(__file__).resolve().parent
COPIED = []


def copy(source, destination):
    source, destination = Path(source), OUT / destination
    if source.is_symlink():
        raise ValueError(f'refuse symlink: {source}')
    if source.is_dir():
        for item in sorted(source.rglob('*')):
            if item.is_file():
                copy(item, destination.relative_to(OUT) / item.relative_to(source))
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    COPIED.append({'source': str(source), 'artifact': str(destination.relative_to(OUT)),
                   'sha256': hashlib.sha256(destination.read_bytes()).hexdigest()})


def top_files(source, destination):
    for item in sorted(source.iterdir()):
        if item.is_file():
            copy(item, Path(destination) / item.name)


def collect():
    for version in ('v21', 'v22'):
        copy(ROOT / f'diagnostics-phases-{version}', f'frozen/{version}')
    copy(ROOT / 'diagnostic-flat-hash-v23/source-ready.txt', 'race/original-source-ready.txt')
    copy(ROOT / 'diagnostic-flat-hash-v23/diagnostic_hash.pre-refinement.rs',
         'race/diagnostic_hash.pre-refinement.rs')
    for version in ('v21', 'v21c', 'v22'):
        copy(Path(f'/home/jeremy/.jcode/scratch/resident-diagnostics-phases-{version}-20261030'),
             f'profiles/{version}')
    for version in ('v21', 'v22'):
        build = ROOT / f'candidate-build-{version}'
        copy(build / 'frozen-source', f'build/{version}/frozen-source')
        top_files(build, f'build/{version}')
        copy(build / 'justrust-home/runs', f'journals/build-{version}/runs')
        copy(build / 'justrust-home/index.jsonl', f'journals/build-{version}/index.jsonl')
        unit = ROOT / f'candidate-unit-{version}'
        top_files(unit, f'native/{version}')
        copy(unit / 'justrust-home/runs', f'journals/unit-{version}/runs')
        copy(unit / 'justrust-home/index.jsonl', f'journals/unit-{version}/index.jsonl')
    restored = ROOT / 'candidate-unit-v22-restored'
    top_files(restored, 'native/v22-restored')
    copy(restored / 'justrust-home/runs', 'journals/unit-v22-restored/runs')
    copy(restored / 'justrust-home/index.jsonl', 'journals/unit-v22-restored/index.jsonl')
    # Completed suite top-level files only. Exclude caches, stores and binaries.
    for name in ('rsb12', 'rsa18', 'rsb13', 'rsa19'):
        top_files(Path('/home/jeremy') / name, f'suites/{name}')
    for version in ('v21', 'v22'):
        for prefix in ('build-test-candidate-', 'verify-native-unit-', 'run-suite-', 'finish-',
                       'full-native-', 'profile-diagnostics-'):
            for source in sorted(ROOT.glob(f'{prefix}{version}.*')):
                if source.is_file() and source.suffix != '.pid':
                    copy(source, f'provenance/{source.name}')
        for source in sorted(ROOT.glob(f'*{version}*.log')):
            copy(source, f'provenance/{source.name}')
        for source in sorted(ROOT.glob(f'*{version}*.exit')):
            copy(source, f'provenance/{source.name}')
    for name in ('profile-diagnostics-v21c.py', 'full-native-v22-restored.sh'):
        copy(ROOT / name, f'provenance/{name}')
    # The harness code is preserved from the accepted source commit, not the live tree.
    for name in ('suite.py', 'resident_engine_compare.py', 'blaze_runtime_compare.py',
                 'blaze_edit_compare.py', 'resident_suite.py'):
        path = f'bench/{name}'
        data = subprocess.check_output(['git', 'show', f'1a0326d59:{path}'])
        dest = OUT / 'harness-captured' / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(data)
        COPIED.append({'source': f'git:1a0326d59:{path}',
                       'artifact': str(dest.relative_to(OUT)),
                       'sha256': hashlib.sha256(data).hexdigest()})
    (OUT / 'collection.json').write_text(json.dumps({
        'source_commit': '1a0326d59', 'kind': 'completed evidence copies, not new native workloads',
        'scope': 'v22 instrumentation acceptance, no speed claim', 'files': COPIED,
        'limitations': ['Original absolute paths retained verbatim in raw evidence; validation uses bundle-local artifacts.',
                        'Selected eleven-source hash checks cannot detect new unlisted files. Race receipt and restored helper-absence checks remain distinct.',
                        'v21/v21c failed profiles are controls, not zero-cost observations.']
    }, indent=2) + '\n')
    print(json.dumps({'copied': len(COPIED)}))


if __name__ == '__main__':
    collect()
