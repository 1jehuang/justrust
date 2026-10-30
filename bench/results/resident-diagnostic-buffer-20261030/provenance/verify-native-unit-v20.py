import json
from pathlib import Path
root = Path('/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/candidate-unit-v20')
expected = {'resident_diagnostic_buffer': 2, 'resident_graph_reuse': 1, 'the_roots_of_a_listed_crate': 1, 'telemetry': 3}
rows = []
for name, count in expected.items():
    matches = []
    for path in (root / 'justrust-home/runs').glob('*/summary.json'):
        data = json.loads(path.read_text())
        if name in data.get('args', []):
            matches.append((path, data))
    if len(matches) != 1:
        raise SystemExit(f'{name}: expected one exact native call, got {len(matches)}')
    path, data = matches[0]
    tests = data.get('tests', {})
    if data.get('exit') != 0 or tests.get('passed') != count or tests.get('failed') != 0:
        raise SystemExit(f'{name}: missing exact positive test coverage: {tests}')
    journal = path.with_name('output.jsonl')
    if not journal.is_file() or not journal.stat().st_size:
        raise SystemExit(f'{name}: missing raw native journal')
    rows.append({'filter': name, 'passed': tests['passed'], 'run_id': data['id']})
report = {'verdict': 'passed', 'native_test_rows': rows}
(root / 'positive-test-admission.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
