#!/usr/bin/env python3
"""Coherently re-sign derivatives before semantic rejection. Python-only checks."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import validate as v
from seal import seal


def emit(path, obj):
    path.write_text(json.dumps(obj, indent=2, sort_keys=True)+'\n')


def resign(root):
    # Deliberately repair public derivative SHA/length and outer bundle inventory.
    # The independent historical anchor and semantic invariants are not rewritten.
    privacy = v.load(root, 'privacy.json')
    for p in privacy['files']:
        data = v.read(root, p['published'])
        p.update(published_sha256=v.sha(data), published_bytes=len(data))
    emit(root/'privacy.json', privacy)
    seal(root)


def state(root, name, change):
    path = root/'runtime'/name/'state.json'
    s = json.loads(path.read_bytes())
    change(s)
    emit(path, s)


def stream(root, name, index, change, which='stdout', row_change=None):
    def edit(s):
        row = s['rows'][index]
        receipt = row['raw'][which]
        path = root/'runtime'/name/Path(receipt['path']).name
        data = change(path.read_text()).encode()
        path.write_bytes(data)
        receipt.update(bytes=len(data), sha256=v.sha(data))
        if row_change:
            row_change(row)
    state(root, name, edit)


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='native-reload-check-', dir=os.environ.get('JCODE_SCRATCH_DIR'))
        self.root = Path(self.temp.name)/'relocated'
        shutil.copytree(HERE, self.root, ignore=shutil.ignore_patterns('__pycache__'))

    def tearDown(self):
        self.temp.cleanup()

    def reject(self, reason):
        resign(self.root)
        # Outer hashes and privacy hashes are valid. Reject only on semantic replay.
        with self.assertRaisesRegex(ValueError, reason):
            v.validate(self.root)

    def test_baseline(self):
        self.assertEqual(v.validate(self.root)['runtime_rows'], 34)

    def test_coherent_wrong_native_value(self):
        stream(self.root, 'native-link-reload-v58-candidate', 1, lambda text: text.replace('_22\n', '_99\n'), row_change=lambda row: row.update(expected=99))
        self.reject('native expected value')

    def test_coherent_nonce_replacement(self):
        name = 'native-link-reload-v58-candidate'
        row = v.load(self.root, 'runtime/'+name+'/state.json')['rows'][0]
        stream(self.root, name, 0, lambda text: text.replace(row['nonce'], '0'*32), row_change=lambda row: row.update(nonce='0'*32))
        self.reject('exact historical semantic identity')

    def test_failure_history_not_pass(self):
        state(self.root, 'native-link-reload-v56-original', lambda s: s['rows'][1].update(accepted=True))
        self.reject('failure history acceptance')

    def test_return_to_a_not_stale_b(self):
        stream(self.root, 'native-file-reload-v58-candidate', 2, lambda text: text.replace('_11\n', '_33\n'))
        self.reject('native nonce/value semantics')

    def test_native_error_not_success(self):
        state(self.root, 'native-link-reload-v58-candidate', lambda s: s['rows'][0].update(error='transport failed'))
        self.reject('runtime transport error')

    def test_coherent_tool_change(self):
        state(self.root, 'native-link-reload-v58-candidate', lambda s: s.update(tool_before='a'*64, tool_after='a'*64))
        self.reject('native engine identity')

    def test_coherent_tc_change(self):
        def change(s):
            for key in ('toolchain_before', 'toolchain_after'):
                s[key]['files']['bin/cargo']['sha256'] = 'a'*64
        state(self.root, 'native-link-reload-v58-candidate', change)
        self.reject('exact historical semantic identity')

    def test_coherent_source_bytes_change(self):
        name = 'native-link-reload-v58-candidate'
        path = self.root/'runtime'/name/'source/b.c'
        path.write_text('unsigned int native_value(void){return 77;}\n')
        def change(s):
            s['source_before']['b.c'] = s['source_after']['b.c'] = v.sha(path.read_bytes())
        state(self.root, name, change)
        self.reject('exact historical semantic identity')

    def test_manifest_missing_diagnostic(self):
        stream(self.root, 'manifest-cfg-reload-v58', 1, lambda text: text.replace('E0308', 'E9999'), which='stderr')
        self.reject('manifest diagnostic semantics')

    def test_manifest_runtime_nonce(self):
        stream(self.root, 'manifest-cfg-reload-v58', 2, lambda text: text.replace('GEN_OK_', 'GEN_BAD_'))
        self.reject('manifest runtime nonce')

    def test_type_error_nonce(self):
        stream(self.root, 'native-reload-v58-seven-state', 3, lambda text: text.replace('JR_MATRIX_TYPE_', 'JR_MATRIX_OTHER_'), which='stderr')
        self.reject('type diagnostic semantics')

    def test_coherent_assert_summary(self):
        stream(self.root, 'native-reload-v58-seven-state', 5, lambda text: text.replace('3 passed; 1 failed;', '4 passed; 0 failed;'), row_change=lambda row: row['parsed'].update(passed=4, failed=0))
        self.reject('assert failure semantics')

    def test_whole_reload_not_activated(self):
        state(self.root, 'native-reload-v58-seven-state', lambda s: s['startup_controls'].update(RA_JIT_WHOLE_RELOAD='1'))
        self.reject('default correctness only')

    def test_file_call_source_hash(self):
        state(self.root, 'native-file-reload-v58-candidate', lambda s: s['rows'][1]['source_at_call'].update({'a.c': 'a'*64}))
        self.reject('C source at call')

    def test_status_identity(self):
        path = self.root/'runtime/native-link-reload-v58-candidate/0-a-status.stdout'
        path.write_text(path.read_text().replace('daemon: pid ', 'daemon: gone '))
        self.reject('missing native status identities')

    def test_status_pid_exact(self):
        path = self.root/'runtime/native-link-reload-v58-candidate/0-a-status.stdout'
        path.write_text(path.read_text().replace('pid 3640568', 'pid 9999999'))
        self.reject('exact historical semantic identity')

    def test_private_env_injection(self):
        state(self.root, 'native-link-reload-v58-candidate', lambda s: s['rows'][0]['environment'].update(SECRET_SYNTHETIC_TEST_KEY='not-a-real-secret'))
        self.reject('unsafe environment entries')

    def test_omission_count(self):
        p = self.root/'privacy.json'
        s = json.loads(p.read_bytes())
        next(e for f in s['files'] for e in f['environment'])['omitted_entries'] += 1
        emit(p, s)
        self.reject('privacy omission arithmetic')

    def test_coherent_patch_candidate(self):
        base = self.root/'frozen/v58-final-doc'
        p = base/'candidate.patch'
        p.write_text(p.read_text().replace('## ', '## MUTATED ', 1))
        candidate = base/'candidates/docs/resident-whole-generation-reuse.md'
        # Ensure candidate and patch both change their added heading, not unrelated source.
        old = (base/'originals/docs/resident-whole-generation-reuse.md').read_text()
        added = candidate.read_text()[len(old):]
        candidate.write_text(old+added.replace('## ', '## MUTATED ', 1))
        lines = []
        for line in (base/'SHA256SUMS').read_text().splitlines():
            _, name = line.split(None, 1)
            lines.append(v.sha((base/name.strip()).read_bytes())+'  '+name.strip())
        (base/'SHA256SUMS').write_text('\n'.join(lines)+'\n')
        self.reject('exact historical semantic identity')

    def test_journal_failure_preserved(self):
        p = next((self.root/'journals/candidate-unit-v56').glob('*/summary.json'))
        for p in (self.root/'journals/candidate-unit-v56').glob('*/summary.json'):
            s = json.loads(p.read_bytes())
            if s['exit'] == 101:
                s['exit'] = 0
                s['tests'].update(passed=362, failed=0)
                emit(p, s)
                op = p.parent/'output.jsonl'
                op.write_text(op.read_text().replace('361 passed; 1 failed;', '362 passed; 0 failed;'))
        self.reject('journal gate counts')

    def test_source3757_missing(self):
        p = self.root/'gates/candidate-build-v58/all-source-before.sha256'
        p.write_text(''.join(p.read_text().splitlines(keepends=True)[1:]))
        self.reject('source3757 identity count')

    def test_rootlock_change(self):
        p = self.root/'gates/candidate-build-v58/lock-before.sha256'
        p.write_text('0'*64+'  Cargo.lock\n')
        self.reject('exact historical semantic identity')

    def test_launcher_build_version(self):
        p = self.root/'gates/candidate-build-v58/engine-version.txt'
        p.write_text(p.read_text().replace('resident-v1-', 'resident-v9-'))
        self.reject('exact historical semantic identity')

    def test_cleanup_no_store_deletion(self):
        p = self.root/'cleanup/completed-native-reload-v58-stop/receipt.json'
        s = json.loads(p.read_bytes())
        s['stores_deleted'] = True
        emit(p, s)
        self.reject('cleanup preserves stores')

    def test_733_regression_result(self):
        p = self.root/'provenance/v58-bench-regression.log'
        p.write_text(p.read_text().replace('Ran 733 tests', 'Ran 732 tests'))
        self.reject('733 regression result')

    def test_sanitizer_explicit_allowlist_and_nonmutation(self):
        from collect import sanitize
        original = {'nested': [{'env': {'SYNTHETIC_SECRET': 'fake-value-only', 'RA_JIT_WHOLE_RELOAD': '0', 'RA_JIT_NOT_ALLOWLISTED': 'must-omit'}}]}
        before = json.dumps(original, sort_keys=True)
        counts = []
        derivative = sanitize(original, counts)
        self.assertEqual(json.dumps(original, sort_keys=True), before)
        self.assertEqual(derivative['nested'][0]['env'], {'RA_JIT_WHOLE_RELOAD': '0'})
        self.assertEqual(counts[0]['omitted_entries'], 2)
        self.assertNotIn('fake-value-only', json.dumps(derivative))

    def test_empty_home_path_relocation_audited(self):
        home = Path(self.temp.name)/'empty-home'
        home.mkdir()
        proc = subprocess.run([sys.executable, '-I', '-B', str(self.root/'validate.py'), '--audit'], cwd=self.temp.name, env={'HOME': str(home), 'PATH': '', 'PYTHONDONTWRITEBYTECODE': '1'}, capture_output=True, text=True, timeout=30)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        result = json.loads(proc.stdout)
        self.assertTrue(result['audit_enabled'])
        self.assertEqual(result['evidence_sha256'], v.EVIDENCE_SHA)

    def test_audit_denies_external_checkout_native_network_and_writes(self):
        actions = ["open('/home/jeremy/justrust/AGENTS.md')", "__import__('os').listdir('/home/jeremy/justrust')", "__import__('sys').audit('subprocess.Popen', 'forbidden-native')", "__import__('sys').audit('socket.connect', 'forbidden-network')", "open(HERE/'forbidden-write', 'w')"]
        for action in actions:
            with self.subTest(action=action):
                script = f"import sys;sys.path.insert(0,{str(self.root)!r});from validate import audit,HERE;audit(HERE);{action}"
                proc = subprocess.run([sys.executable, '-I', '-B', '-c', script], cwd=self.temp.name, env={'HOME': self.temp.name, 'PATH': ''}, capture_output=True, text=True, timeout=10)
                self.assertNotEqual(proc.returncode, 0)
                self.assertIn('audit ', proc.stderr)


if __name__ == '__main__':
    unittest.main(verbosity=2)
