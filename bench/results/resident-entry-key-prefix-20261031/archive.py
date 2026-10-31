#!/usr/bin/env python3
"""Archive existing journals only. Does not run native jobs or change source."""
import hashlib
import json
from pathlib import Path
import shutil

ROOT = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
OUT = Path(__file__).resolve().parent

def copy(src, dst):
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dst)

for version in ('v44', 'v45'):
    frozen = ROOT / ('entry-key-prefix-' + version)
    shutil.copytree(frozen, OUT / 'frozen' / version, dirs_exist_ok=True)
    for kind in ('build', 'unit'):
        src = ROOT / ('candidate-' + kind + '-' + version)
        for file in src.iterdir():
            if file.is_file():
                copy(file, OUT / 'native' / version / kind / file.name)
        for run in (src / 'justrust-home' / 'runs').glob('*'):
            for name in ('meta.json', 'output.jsonl', 'summary.json', 'units.jsonl'):
                if (run / name).is_file():
                    copy(run / name, OUT / 'native' / version / kind / 'runs' / run.name / name)
    copy(ROOT / ('build-test-candidate-' + version + '.sh'), OUT / 'native' / version / 'launcher.sh')
    copy(ROOT / ('entry-key-prefix-' + version + '-pairs.exit'), OUT / 'native' / version / 'pairs.exit')
    for mode in ('off', 'on', 'on-repeat', 'off-repeat'):
        src = ROOT / ('entry-key-prefix-' + version + '-' + mode)
        for file in src.iterdir():
            if file.is_file():
                copy(file, OUT / 'raw' / version / mode / file.name)
copy(ROOT / 'probe-entry-key-prefix-v44.py', OUT / 'provenance' / 'probe-entry-key-prefix-v44.py')
for name in ('validate-prefix-v45.py', 'prefix-v45-validation.json'):
    copy(ROOT / name, OUT / 'provenance' / name)
for name in ('resident_state_matrix', 'resident_engine_compare', 'resident_workload_compare', 'blaze_runtime_compare', 'blaze_edit_compare'):
    copy(Path('/home/jeremy/justrust/bench') / (name + '.py'), OUT / 'oracle' / (name + '.py'))
copy(ROOT / 'cleanup-completed-v43-v44.py', OUT / 'provenance' / 'cleanup-completed-v43-v44.py')
for file in (ROOT / 'completed-v43-v44-stop').iterdir():
    if file.is_file():
        copy(file, OUT / 'provenance' / 'completed-v43-v44-stop' / file.name)
copy(ROOT / 'cleanup-completed-v45.py', OUT / 'provenance' / 'cleanup-completed-v45.py')
for file in (ROOT / 'completed-v45-stop').iterdir():
    if file.is_file():
        copy(file, OUT / 'provenance' / 'completed-v45-stop' / file.name)
rows = []
for state in sorted((OUT / 'raw').glob('*/*/state.json')):
    data = json.loads(state.read_text())
    for row in data['rows']:
        rows.append(dict(version=state.parent.parent.name, mode=state.parent.name, **row))
(OUT / 'rows56.json').write_text(json.dumps(rows, indent=2) + '\n')
print('Archived', len(rows), 'actual rows without native execution')
