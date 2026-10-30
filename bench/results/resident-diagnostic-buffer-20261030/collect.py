#!/usr/bin/env python3
"""One-shot allowlisted evidence collection. No builds or runtime execution."""
import hashlib,json,shutil
from pathlib import Path
ROOT=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030')
OUT=Path(__file__).resolve().parent

def copy(src,dest):
    dst=OUT/dest
    dst.parent.mkdir(parents=True,exist_ok=True)
    shutil.copyfile(src,dst)

def files(src,dest):
    for p in src.rglob('*'):
        if p.is_file(): copy(p,Path(dest)/p.relative_to(src))

files(ROOT/'diagnostic-buffer-v20','source-ready')
for kind in ['build','unit']:
    src=ROOT/f'candidate-{kind}-v20'
    for p in src.iterdir():
        if p.is_file(): copy(p,f'native/candidate-{kind}-v20/{p.name}')
    if kind=='build': files(src/'frozen-source','native/candidate-build-v20/frozen-source')
    runs=sorted((src/'justrust-home/runs').iterdir())
    assert len(runs)==(2 if kind=='build' else 5)
    for run in runs:
        for name in ['output.jsonl','meta.json','summary.json']:
            copy(run/name,f'native/candidate-{kind}-v20/justrust-home/runs/{run.name}/{name}')
for name in ['build-test-candidate-v20.sh','full-native-v20.sh','verify-native-unit-v20.py','run-suite-v20.sh','queue-suite-v20.sh','finish-v20.sh','interleaved-v20.py','interleaved-v20.log','native-unit-v20-admission.log','candidate-build-v20-launch.log','suite-v20-baseline.log','suite-v20-candidate.log','suite-v20-baseline.exit','suite-v20-candidate.exit','suite-v20-compare.log','suite-v20.exit','final-v20.exit','final-v20-launch.log']:
    copy(ROOT/name,'provenance/'+name)
for case in ['rsb11','rsa17']:
    for p in (Path('/home/jeremy')/case).iterdir():
        if p.is_file(): copy(p,f'sessions/{case}/{p.name}')
files(Path('/home/jeremy/.jcode/scratch/resident-interleaved-v20-20261030'),'observations/interleaved-v20')
old=OUT.parent/'resident-root-snapshot-20261030'
files(old/'native/candidate-build-v4','native/candidate-build-v4')
for name in ['raw_checks.py','warm_checks.py']:
    copy(old/name,name)
for name in ['suite.py','resident_engine_compare.py','blaze_runtime_compare.py','blaze_edit_compare.py','resident_suite.py']:
    copy(Path('bench')/name,'harness-captured/'+name)
# Preserve exact seven run identifiers as a collection receipt, independently pinned by validator.
rows=[]
for p in sorted((OUT/'native').glob('candidate-*-v20/justrust-home/runs/*/summary.json')):
    d=json.loads(p.read_text()); rows.append({'id':d['id'],'args':d['args'],'summary':str(p.relative_to(OUT))})
(OUT/'native-runs.json').write_text(json.dumps(rows,indent=2)+'\n')
print(json.dumps({'native_runs':len(rows),'files':sum(p.is_file() for p in OUT.rglob('*'))}))
