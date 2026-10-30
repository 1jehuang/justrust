import json
from pathlib import Path
root=Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/candidate-unit-v26-snapshot')
expected=[('ide-diagnostics','resident_collector_subphases',6),('hir','resident_collector_subphases',2),('ide-diagnostics','resident_semantic_subphases',5),('ra-jit','diagnostic_hash::tests',2),('ra-jit','diagnostics_timing_tests',3),('ra-jit','resident_graph_reuse',1),('ra-jit','the_roots_of_a_listed_crate',1),('ra-jit','telemetry',3)]
rows=[]
for package,name,count in expected:
    matches=[]
    for path in (root/'justrust-home/runs').glob('*/summary.json'):
        data=json.loads(path.read_text());args=data.get('args',[])
        if name in args and package in args:matches.append((path,data))
    if len(matches)!=1:raise SystemExit(f'{package}/{name}: expected1 exact native call got{len(matches)}')
    path,data=matches[0];tests=data.get('tests',{})
    if data.get('exit')!=0 or tests.get('passed')!=count or tests.get('failed')!=0:raise SystemExit(f'{package}/{name}: positive coverage missing {tests}')
    raw=path.with_name('output.jsonl')
    if not raw.is_file() or not raw.stat().st_size:raise SystemExit('missing raw native journal')
    rows.append(dict(package=package,filter=name,passed=count,run_id=data['id']))
report=dict(verdict='passed',native_test_rows=rows)
(root/'positive-test-admission.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report,indent=2))
