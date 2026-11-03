#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys
import tempfile
import time
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
        # SIGKILL delivery to the descendant is asynchronous even after its
        # parent was reaped. Require disappearance or a zombie, but allow a
        # bounded scheduling window rather than asserting on the first tick.
        deadline = time.monotonic() + 2
        while True:
            try:
                status = state.read_text().split()[2]
            except FileNotFoundError:
                break
            if status == 'Z':
                break
            self.assertLess(time.monotonic(), deadline, f'descendant {pid} remains {status}')
            time.sleep(0.01)
        self.assertEqual(json.loads((attempt / 'result.json').read_text())['status'], 'timeout')

    def test_tie_order_by_sha_not_input_order(self):
        self.run_case(['zz', 'aa'])
        expected = min([b'zz', b'aa'], key=reducer.sha)
        self.assertEqual((self.root / 'out/attempt-0001/source.rs').read_bytes(), expected)


# Narrow utility acceptance, not a permanent compiler-bug assertion.
ACTUAL_CHECKER = r'''import json, os, subprocess, sys
from pathlib import Path
stock, native, source = sys.argv[1:]
env = dict(os.environ)
env.pop('RUSTC_BOOTSTRAP', None)
def invoke(label, args):
    r = subprocess.run(args, env=env, capture_output=True, timeout=30)
    Path(label + '.stdout').write_bytes(r.stdout)
    Path(label + '.stderr').write_bytes(r.stderr)
    return r
r = invoke('stock-version', [stock, '-vV'])
required = {'release: 1.98.1', 'commit-hash: 48a229ceaefd4985c50990b14116b6d856af0985',
            'host: x86_64-unknown-linux-gnu', 'LLVM version: 22.1.8'}
if r.returncode or not required.issubset(set(r.stdout.decode().splitlines())):
    raise SystemExit('pinned stock identity mismatch')
r = invoke('stock', [stock, '--crate-name', 'reducer_probe', '--edition=2024',
                     '--emit=metadata', '-o', 'stock.rmeta', source])
outcomes = {'stock_exit': r.returncode, 'native': {}}
for workers in (1, 8):
    r = invoke('native-' + str(workers), [native, source, '--check', '--workers', str(workers)])
    expected = ('jr-rust: ' + source + ': in callable crate::main: contract: nested index place projection\n'
                'note: source preparation and syntax diagnostics use original file locations; semantic diagnostic source spans remain incomplete\n').encode()
    status = ('success' if r.returncode == 0 else 'valid-source-rejected'
              if r.returncode == 1 and r.stderr == expected and not r.stdout
              else 'unexpected-failure')
    outcomes['native'][str(workers)] = {'exit': r.returncode, 'status': status}
Path('outcomes.json').write_text(json.dumps(outcomes, sort_keys=True))
statuses = [v['status'] for v in outcomes['native'].values()]
if outcomes['stock_exit'] != 0 or len(set(statuses)) != 1 or statuses[0] == 'unexpected-failure':
    raise SystemExit('stock rejection, unexpected native failure or worker mismatch')
success = statuses[0] == 'success'
print(json.dumps({'stock': 'valid', 'native': 'success' if success else 'failure',
                  'failure_class': 'none' if success else 'valid-source-rejected'}))
'''


class ActualReductionTests(unittest.TestCase):
    def test_pinned_stock_current_native_authored_reduction(self):
        import subprocess
        missing = [n for n in ('JR_REDUCER_STOCK', 'JR_REDUCER_NATIVE') if not os.environ.get(n)]
        if missing:
            self.skipTest('optional actual acceptance requires ' + ', '.join(missing))
        env = dict(os.environ)
        env.pop('RUSTC_BOOTSTRAP', None)
        stock = str(Path(env['JR_REDUCER_STOCK']).resolve(strict=True))
        native = str(Path(env['JR_REDUCER_NATIVE']).resolve(strict=True))
        version = subprocess.run([stock, '-vV'], env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(version.returncode, 0, version.stderr)
        required = {'release: 1.98.1', 'commit-hash: 48a229ceaefd4985c50990b14116b6d856af0985',
                    'host: x86_64-unknown-linux-gnu', 'LLVM version: 22.1.8'}
        self.assertTrue(required.issubset(set(version.stdout.splitlines())), version.stdout)
        # Exact bytes retained in failure-reducer.ii5ZZK, including no final newline.
        original_bytes = b'struct Guard{n:i32}impl Drop for Guard{fn drop(&mut self){println!("drop {}",self.n);}}fn main(){let pair=(3i32,4i32);let r=&pair.0;let guard=Guard{n:7};let f=move||{let value=*r+guard.n;drop(guard);value};println!("{}",f());println!("{}",pair.1);}'
        smaller_bytes = b'struct Guard{n:i32}impl Drop for Guard{fn drop(&mut self){}}fn main(){let pair=(3i32,4i32);let r=&pair.0;let guard=Guard{n:7};let f=move||{let value=*r+guard.n;drop(guard);value};println!("{}",f());}'
        self.assertEqual(reducer.sha(original_bytes), '1fa0c58c743faf3e4af0072eab02c0adf45a557f24f98d438ebcb99cf0ecc600')
        self.assertEqual(reducer.sha(smaller_bytes), '4df8f6f8326929184e9d614db92266bce42fa23b1a001ebf5d4f0b277214d5d7')
        root = Path(tempfile.mkdtemp(prefix='reducer-actual-', dir=os.environ.get('JCODE_SCRATCH_DIR')))
        print('pinned-reducer-acceptance: evidence=' + str(root), flush=True)
        original, smaller = root / 'original.rs', root / 'authored-smaller.rs'
        original.write_bytes(original_bytes)
        smaller.write_bytes(smaller_bytes)
        checker = root / 'checker.py'
        checker.write_text(ACTUAL_CHECKER)
        identities = {name: {'path': path, 'sha256': reducer.sha(Path(path).read_bytes())}
                      for name, path in [('stock', stock), ('native', native)]}
        reducer.save(root / 'identities.json', {'binaries': identities, 'stock_version': version.stdout,
                     'sources': {'original': reducer.sha(original_bytes), 'smaller': reducer.sha(smaller_bytes)},
                     'checker_sha256': reducer.sha(checker.read_bytes())})
        command = [sys.executable, str(checker), stock, native]
        report = reducer.run(original, [smaller], root / 'reduction', root, command, 2, 150)
        def validate(directory, result):
            self.assertEqual(result.get('exit_code'), 0, (directory / 'stderr').read_text())
            outcomes = json.loads((directory / 'outcomes.json').read_text())
            self.assertEqual(outcomes['stock_exit'], 0)
            self.assertEqual(set(outcomes['native']), {'1', '8'})
            statuses = [outcomes['native'][str(w)]['status'] for w in (1, 8)]
            self.assertEqual(statuses[0], statuses[1], outcomes)
            self.assertIn(statuses[0], ('success', 'valid-source-rejected'))
            success = statuses[0] == 'success'
            self.assertEqual(result['status'], 'not-preserved' if success else 'classified-failure')
            self.assertEqual(result['evidence'], {'stock': 'valid', 'native': 'success' if success else 'failure',
                             'failure_class': 'none' if success else 'valid-source-rejected'})
            return statuses[0]
        first_dir = root / 'reduction/attempt-0000'
        first = json.loads((first_dir / 'result.json').read_text())
        baseline = validate(first_dir, first)
        preserved = baseline == 'valid-source-rejected'
        self.assertEqual(report['baseline_preserved'], preserved)
        self.assertEqual(first['preserved'], preserved)
        self.assertEqual(report['attempts'], 2 if preserved else 1)
        if preserved:
            second_dir = root / 'reduction/attempt-0001'
            second = json.loads((second_dir / 'result.json').read_text())
        else:
            self.assertIsNone(report['best_observed_reproducer'])
            # Reducer stops at a successful baseline, still check the other source.
            second_dir = root / 'smaller-observation'
            second = reducer.attempt(second_dir, smaller_bytes, command, 150)
            reducer.save(second_dir / 'result.json', second)
        candidate = validate(second_dir, second)
        if preserved:
            smaller_preserved = candidate == 'valid-source-rejected'
            self.assertEqual(second['preserved'], smaller_preserved)
            expected = {'path': 'attempt-0001/source.rs' if smaller_preserved else 'original.rs',
                        'sha256': reducer.sha(smaller_bytes if smaller_preserved else original_bytes),
                        'bytes': len(smaller_bytes if smaller_preserved else original_bytes)}
            self.assertEqual(report['best_observed_reproducer'], expected)
        self.assertEqual(report['unattempted_candidates'], 0 if preserved else 1)
        self.assertEqual((root / 'reduction/original.rs').read_bytes(), original_bytes)
        for identity in identities.values():
            self.assertEqual(reducer.sha(Path(identity['path']).read_bytes()), identity['sha256'])
        reducer.save(root / 'acceptance.json', {'original': baseline, 'smaller': candidate, 'report': report})
        print('pinned-reducer-acceptance: original=' + baseline + ', smaller=' + candidate +
              ', baseline_preserved=' + str(preserved) + ', no feature/rule/compatibility credit', flush=True)

if __name__ == '__main__': unittest.main()
