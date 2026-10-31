#!/usr/bin/env python3
"""Source-only freeze checks. No builds, probes, native launches, or workloads."""
import ast
import hashlib
from pathlib import Path
import sys

ROOT = Path('<EXPERIMENT>/watched-manifest-suite-v76')
REPO = Path('<PRIVATE_HOME>/justrust')
sys.dont_write_bytecode = True
sys.path.insert(0, str(REPO / 'bench'))
import resident_suite as suite

original = ROOT / 'originals/resident_suite.py'
assert hashlib.sha256(original.read_bytes()).hexdigest() == '1508a424fde23d7093ae004bc476abcf95d7681e36ed61d9d5c923de9f9cb2df'

def node(path, name):
    return next(n for n in ast.parse(path.read_text()).body if getattr(n, 'name', None) == name)

def same_node(name, filename):
    assert ast.dump(node(ROOT / 'originals' / filename, name)) == ast.dump(node(REPO / 'bench' / filename, name)), name

same_node('ManifestRetouchTests', 'test_resident_manifest_retouch.py')
for name in ('replay', 'retouch_fixture', 'retouch_snapshot', 'retouch_inputs', 'proc_identity',
             'worker_key', 'check_retouch_process', 'check_retouch_output_length'):
    same_node(name, 'resident_suite.py')
assert len([n for n in node(ROOT / 'originals/test_resident_manifest_retouch.py', 'ManifestRetouchTests').body
            if getattr(n, 'name', '').startswith('test_')]) == 13
assert suite.RETOUCH_POLICY.schema == 'native_resident_manifest_retouch/v1'
assert suite.WATCHED_POLICY.schema == 'native_resident_watched_manifest_retouch/v1'
for arm, watcher in (('off', '0'), ('on', '1')):
    assert suite.WATCHED_POLICY.flags(arm) == {suite.RETOUCH_FLAG: '1', suite.WATCHED_FLAG: watcher}
    assert suite.WATCHED_POLICY.flags(arm, reference=True) == {suite.RETOUCH_FLAG: '0', suite.WATCHED_FLAG: '0'}
    assert suite.WATCHED_POLICY.reuse_worker(arm)
    assert suite.RETOUCH_POLICY.flags(arm) == {suite.RETOUCH_FLAG: watcher}
    assert suite.RETOUCH_POLICY.reuse_worker(arm) == (arm == 'on')
assert suite.RETOUCH_ORDER == ('off', 'on', 'on', 'off')
assert suite.RETOUCH_FILES == ('Cargo.toml', 'Cargo.lock')
print('PASS: original v74 harness SHA, unchanged original 13-test class and shared controls, distinct immutable schemas/policies, exact flags and PID policies')
print('SOURCE ONLY: no native semantic/performance acceptance and no speed claim')
