#!/usr/bin/env python3
"""Private v83 factor adapter derived from immutable v81, never a live harness edit.
Only factor identity, flag, activation marker and descriptive labels change.
All input recipes, original oracles, sampling, PID, nonce, raw replay and cleanup
logic remain the frozen implementation. --mock-tests runs mocked Python only.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import runpy
import sys
import types
import unittest

BENCH=Path('<repo>/bench')
ROOT=Path('<native-root>')
FROZEN=ROOT/'parallel-manifest-harness-v81/candidates/bench/resident_suite.py'
TEST=ROOT/'parallel-manifest-harness-v81/candidates/bench/test_resident_parallel_manifest.py'
REPLACEMENTS=(
 ('RA_JIT_PARALLEL_MANIFEST_PARSE','RA_JIT_NATIVE_TYPED_PLANNING'),
 ('native_resident_parallel_manifest_retouch/v1','native_resident_typed_planning_retouch/v1'),
 ('parallel-manifest-retouch','typed-planning-retouch'),
 ('ra-jit: parallel-manifest-parse-v81 batch','ra-jit: native-typed-planning-v83 entered'),
 ('ra-jit: parallel-manifest-parse-','ra-jit: native-typed-planning-'),
 ('actual_batch_stderr_marker','actual_conversion_stderr_marker'),
 # Preserve the original wrong-version negative after renaming v81 to v83.
 ("line.replace('v81', 'v80')","line.replace('v83', 'v80')"),
 ('noop/clientplanning parser factor','noop/clientplanning typed planning factor'),
 ('Parser flag explicitly','Typed planning flag explicitly'),
)

def transformed(path):
 raw=path.read_bytes()
 text=raw.decode()
 for old,new in REPLACEMENTS:
  text=text.replace(old,new)
 return raw,text

sys.path.insert(0,str(BENCH))
raw,source=transformed(FROZEN)
module=types.ModuleType('resident_suite')
module.__file__=str(FROZEN)
sys.modules['resident_suite']=module
exec(compile(source,str(FROZEN),'exec'),module.__dict__)
# Frozen pinned_hashes derives only git authority from this path.
module.__file__=str(BENCH/'resident_suite.py')

if '--mock-tests' in sys.argv:
 test_raw,test_source=transformed(TEST)
 tests=types.ModuleType('typed_planning_factor_tests')
 tests.__file__=str(TEST)
 exec(compile(test_source,str(TEST),'exec'),tests.__dict__)
 suite=unittest.defaultTestLoader.loadTestsFromModule(tests)
 result=unittest.TextTestRunner(verbosity=2).run(suite)
 print(json.dumps({'frozen_sha256':hashlib.sha256(raw).hexdigest(),
                   'executed_factor_sha256':hashlib.sha256(source.encode()).hexdigest(),
                   'frozen_test_sha256':hashlib.sha256(test_raw).hexdigest(),
                   'tests':result.testsRun,'passed':result.wasSuccessful(),
                   'native_jobs_executed':0},sort_keys=True))
 raise SystemExit(0 if result.wasSuccessful() else 1)
runpy.run_path(str(BENCH/'suite.py'),run_name='__main__')
