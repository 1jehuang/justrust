#!/usr/bin/env python3
"""Offline mutation and relocated-bundle acceptance checks, no native workloads."""
import copy,json,os,shutil,subprocess,sys,tempfile,unittest
from pathlib import Path
from validate import validate,ENGINES
from raw_checks import Evidence,InvalidEvidence,session
import warm_checks
ROOT=Path(__file__).resolve().parent
WARM='observations/interleaved-v20/'

class ReplayTests(unittest.TestCase):
    def changed(self,path,change):
        value=json.loads((ROOT/path).read_text());change(value)
        return {path:(json.dumps(value)+'\n').encode()}

    def reject(self,overrides):
        with self.assertRaises((InvalidEvidence,KeyError,ValueError)):
            validate(ROOT,overrides,check_inventory=False)

    def test_positive_complete_replay(self):
        audit=validate(ROOT)
        self.assertEqual(audit['verdict'],'REJECT')
        self.assertEqual(audit['native']['full-native']['passed'],348)
        self.assertEqual(audit['interleaved']['rows'],141)

    def test_marker_even_with_rehashed_stream(self):
        path=WARM+'edit-0-new.stdout'
        original=(ROOT/path).read_bytes()
        # Marker may be on either stream. Select actual marker-bearing one.
        if b'JR_RUNTIME_EXECUTED_801' not in original:
            path=WARM+'edit-0-new.stderr';original=(ROOT/path).read_bytes()
        changed=original.replace(b'JR_RUNTIME_EXECUTED_801',b'JR_RUNTIME_EXECUTED_999')
        self.assertNotEqual(changed,original)
        from raw_checks import sha
        stream=path.rsplit('.',1)[1]
        overrides=self.changed(WARM+'state.json',lambda s:next(r for r in s['rows'] if r['phase']=='edit' and r['replicate']==0 and r['stack']=='new').update({stream+'_sha256':sha(changed)}))
        overrides[path]=changed;self.reject(overrides)

    def test_runtime_status(self):
        self.reject(self.changed(WARM+'state.json',lambda s:s['rows'][0].update(status=101)))

    def test_testcount_349_is_rejected(self):
        path='native/candidate-unit-v20/justrust-home/runs/20261030-113913476-2300871/summary.json'
        self.reject(self.changed(path,lambda s:s['tests'].update(passed=349)))

    def test_native_raw_name_substitution(self):
        path='native/candidate-unit-v20/justrust-home/runs/20261030-113913476-2300871/output.jsonl'
        raw=(ROOT/path).read_bytes()
        changed=raw.replace(b'resident_diagnostic_buffer_matches_owned_syntax_text',b'resident_diagnostic_buffer_forged_native_test_name')
        self.assertNotEqual(changed,raw);self.reject({path:changed})

    def test_hash_corruption(self):
        path=WARM+'prepare-0-old.stdout'
        with self.assertRaises(InvalidEvidence):validate(ROOT,{path:(ROOT/path).read_bytes()+b'corrupt'})

    def test_medians_not_trusted(self):
        self.reject(self.changed(WARM+'state.json',lambda s:s['medians_seconds'].update(new=0.00001)))

    def test_fixed_medians_not_trusted(self):
        self.reject(self.changed('sessions/rsa17/summary.json',lambda s:s['medians_seconds'].update(engine=0.00001)))

    def test_source63_not_trusted(self):
        self.reject(self.changed(WARM+'state.json',lambda s:s['source_before'].update({'src/summary.rs':'0'*64})))

    def test_candidate_source_corruption(self):
        path='source-ready/toolchain/resident/crates/ra-jit/src/main.rs'
        self.reject({path:(ROOT/path).read_bytes()+b'\n'})

    def test_tool_identity(self):
        self.reject(self.changed(WARM+'state.json',lambda s:s['configs']['new'].update(sha256='0'*64)))

    def test_profile(self):
        self.reject(self.changed(WARM+'state.json',lambda s:s['rows'][0]['environment'].update(CARGO_PROFILE_DEV_OPT_LEVEL='3')))

    def test_toolchain_identity(self):
        self.reject(self.changed(WARM+'state.json',lambda s:s['toolchain_after'].clear()))

    def test_paired_order(self):
        def change(s):s['rows'][15],s['rows'][16]=s['rows'][16],s['rows'][15]
        self.reject(self.changed(WARM+'state.json',change))

    def test_edit_bytes(self):
        self.reject(self.changed(WARM+'state.json',lambda s:s['rows'][15].update(edit_sha256='0'*64)))

    def test_recovery_required(self):
        self.reject(self.changed(WARM+'state.json',lambda s:s['rows'].pop(2)))

    def test_failure_required(self):
        self.reject(self.changed(WARM+'state.json',lambda s:s['rows'][1].update(status=0)))

    def test_source_restoration(self):
        self.reject(self.changed('provenance/root-restoration.json',lambda s:s.update(working_sha256='0'*64)))

    def test_moved_directory_offline(self):
        scratch=Path(os.environ.get('JCODE_SCRATCH_DIR','/home/jeremy/.jcode/scratch'))
        with tempfile.TemporaryDirectory(prefix='v20-offline-moved-',dir=scratch) as temp:
            moved=Path(temp)/'independent-bundle'
            shutil.copytree(ROOT,moved,ignore=shutil.ignore_patterns('__pycache__'))
            result=subprocess.run([sys.executable,'-B',str(moved/'validate.py'),str(moved)],cwd=temp,capture_output=True,text=True,check=True)
            audit=json.loads(result.stdout)
            self.assertEqual(audit['verdict'],'REJECT')
            self.assertEqual(audit['native']['full-native']['passed'],348)

if __name__=='__main__':unittest.main(verbosity=2)
