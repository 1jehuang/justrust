#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
import bounded_failure_reduce as reducer

CHECKER = '''import json, sys, time, os
from pathlib import Path
p = Path(sys.argv[-1])
s = p.read_text().strip()
v = {"stock":"valid", "native":"failure", "failure_class":"valid-source-rejected"}
if s == 'timeout': time.sleep(5)
if s == 'child':
    pid = os.fork()
    if pid == 0:
        time.sleep(30)
        os._exit(0)
    Path('child.pid').write_text(str(pid))
    time.sleep(30)
if s == 'exit': sys.exit(7)
if s == 'signal': os.kill(os.getpid(), 9)
if s == 'malformed': print('no'); sys.exit(0)
if s == 'duplicate': print('{"stock":"valid","stock":"invalid"}'); sys.exit(0)
if s == 'syntax': v['stock'] = 'invalid'
if s == 'changed': v['failure_class'] = 'wrong-code'
if s == 'infra': v['failure_class'] = 'infrastructure'
if s == 'success': v['native'] = 'success'
if s == 'mutate': p.write_text('changed')
if s == 'retained': Path('source.rs').write_text('changed')
if s == 'original': Path('../original.rs').write_text('changed')
if s == 'symlink':
    p.unlink()
    p.symlink_to('source.rs')
if s == 'shape': v['failure_class'] = []
print(json.dumps(v))
'''

class ReductionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=os.environ.get('JCODE_SCRATCH_DIR'))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.checker = self.root / 'checker.py'
        self.checker.write_text(CHECKER)
        self.original = self.root / 'original.rs'
        self.original.write_text('baseline padding padding padding')
        self.command = [sys.executable, str(self.checker)]

    def run_case(self, texts=(), limit=30, original=None, command=None, output='out'):
        if original is not None:
            self.original.write_text(original)
        paths = []
        for i, text in enumerate(texts):
            path = self.root / f'candidate-{i}.rs'
            path.write_text(text)
            paths.append(path)
        return reducer.run(self.original, paths, self.root / output, self.root,
                           self.command if command is None else command, limit, .15)

    def test_all_adversarial_classifications_rejected(self):
        values = ['timeout', 'exit', 'signal', 'malformed', 'duplicate', 'syntax',
                  'changed', 'infra', 'success', 'mutate', 'shape', 'retained', 'original', 'symlink']
        report = self.run_case(values)
        self.assertEqual(report['best_observed_reproducer']['path'], 'original.rs')
        self.assertEqual(report['attempts'], len(values) + 1)
        for path in sorted((self.root / 'out').glob('attempt-*/result.json'))[1:]:
            result = json.loads(path.read_text())
            self.assertFalse(result['preserved'], result)
            self.assertEqual(reducer.sha((path.parent / 'source.rs').read_bytes()), result['sha256'])

    def test_bounds_and_deterministic_smallest_observed(self):
        report = self.run_case(['zz', 'a', 'bb'], limit=2)
        self.assertEqual(report['attempts'], 2)
        self.assertEqual(report['unattempted_candidates'], 2)
        self.assertEqual(report['best_observed_reproducer']['bytes'], 1)
        self.assertEqual((self.root / 'out/attempt-0001/source.rs').read_text(), 'a')
        self.assertEqual((self.root / 'out/original.rs').read_bytes(), self.original.read_bytes())

    def test_zero_evidence_stops_without_claim(self):
        for i, value in enumerate(['syntax', 'timeout', 'exit', 'malformed']):
            report = self.run_case(['a'], original=value, output=f'out{i}')
            self.assertIsNone(report['best_observed_reproducer'])
            self.assertEqual(report['attempts'], 1)

    def test_spawn_failure_no_claim(self):
        report = self.run_case(command=['/nonexistent/checker'])
        self.assertIsNone(report['best_observed_reproducer'])

    def test_exclusive_and_scratch_confinement(self):
        self.run_case()
        with self.assertRaises(FileExistsError): self.run_case()
        with self.assertRaises(ValueError):
            reducer.run(self.original, [], self.root.parent / 'escape', self.root, self.command, 1, 1)
        (self.root / 'link').symlink_to(self.root.parent, target_is_directory=True)
        with self.assertRaises(ValueError):
            reducer.run(self.original, [], self.root / 'link/escape', self.root, self.command, 1, 1)

    def test_invalid_bounds(self):
        for limit, timeout in [(0, 1), (True, 1), (1, True), (1, 0), (1, float('nan')), (1, float('inf'))]:
            with self.assertRaises(ValueError):
                reducer.run(self.original, [], self.root / 'out', self.root, self.command, limit, timeout)

    def test_no_shell_interpolation(self):
        report = self.run_case(command=[sys.executable, str(self.checker), '$(touch pwned); *'])
        self.assertTrue(report['baseline_preserved'])
        self.assertFalse((self.root / 'out/attempt-0000/pwned').exists())

    def test_timeout_kills_descendant_group(self):
        report = self.run_case(original='child')
        self.assertIsNone(report['best_observed_reproducer'])
        attempt = self.root / 'out/attempt-0000'
        pid = int((attempt / 'child.pid').read_text())
        state = Path(f'/proc/{pid}/stat')
        # A killed child can remain an init-owned zombie until reaped.
        if state.exists():
            self.assertEqual(state.read_text().split()[2], 'Z')
        self.assertEqual(json.loads((attempt / 'result.json').read_text())['status'], 'timeout')

    def test_tie_order_by_sha_not_input_order(self):
        self.run_case(['zz', 'aa'])
        expected = min([b'zz', b'aa'], key=reducer.sha)
        self.assertEqual((self.root / 'out/attempt-0001/source.rs').read_bytes(), expected)

if __name__ == '__main__': unittest.main()
