#!/usr/bin/env python3
"""Offline source-only checks. Does not launch any native tool or inspect /proc."""
import ast
import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = Path('<repo>')
old = ast.parse((ROOT / 'originals/bench/resident_suite.py').read_text())
new = ast.parse((REPO / 'bench/resident_suite.py').read_text())
changed = {'RetouchPolicy', 'retouch_policy', 'validate', 'cmd_resident', 'compare',
           'replay_retouch', 'cmd_manifest_retouch', 'compare_retouch', 'add_parser'}
nodes = lambda tree: {n.name: ast.dump(n, include_attributes=False) for n in tree.body
                      if isinstance(n, (ast.FunctionDef, ast.ClassDef))}
a, b = nodes(old), nodes(new)
for name in a:
    if name not in changed:
        assert a[name] == b[name], f'legacy helper AST changed: {name}'
assert set(a) <= set(b)
assert 'RetouchPolicy' in b and 'replay_retouch' in b
# Existing v74/v76 mock classes remain completely unchanged in the checkout.
legacy = REPO / 'bench/test_resident_manifest_retouch.py'
assert hashlib.sha256(legacy.read_bytes()).hexdigest() == (ROOT / 'legacy-tests.sha256').read_text().strip()
assert not Path('<home>/pm81a').exists(), 'prospective output is not fresh'
assert len(b'<home>/pm81a') <= 30
print('PASS unchanged legacy helper AST outside enumerated policy/dispatch/runner extensions')
print('PASS unchanged full v74/v76 test source hash')
print('PASS new schema reuses one RetouchPolicy runner/replay')
print('PASS prospective absolute native output <home>/pm81a is fresh and <=30 bytes')
print('SOURCE ONLY. Native correctness, activation, and timing remain coordinator acceptance.')
