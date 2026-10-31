#!/usr/bin/env python3
"""Static source-only preservation and syntax inspection. No native invocations."""
import ast
import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parent
original = ast.parse((root / 'resident_suite.original.py').read_text())
candidate_path = Path('<PRIVATE_HOME>/justrust/bench/resident_suite.py')
tests_path = Path('<PRIVATE_HOME>/justrust/bench/test_resident_manifest_retouch.py')
candidate = ast.parse(candidate_path.read_text())
tests = ast.parse(tests_path.read_text())
old = {n.name: n for n in original.body if isinstance(n, ast.FunctionDef)}
new = {n.name: n for n in candidate.body if isinstance(n, ast.FunctionDef)}
checks = {}
for name, node in old.items():
    other = new[name]
    if name in ('validate', 'cmd_resident', 'compare'):
        assert isinstance(other.body[0], ast.If)
        other.body = other.body[1:]
    if name == 'add_parser':
        other.body = [statement for statement in other.body if not (
            isinstance(statement, ast.Expr) and isinstance(statement.value, ast.Call)
            and statement.value.args and isinstance(statement.value.args[0], ast.Constant)
            and statement.value.args[0].value == '--mode')]
    checks['legacy_' + name + '_unchanged_after_explicit_dispatch'] = ast.dump(node) == ast.dump(other)
checks['legacy_schema_unchanged'] = "SCHEMA = 'native_resident/v1'" in candidate_path.read_text()
checks['new_distinct_schema'] = "RETOUCH_SCHEMA = 'native_resident_manifest_retouch/v1'" in candidate_path.read_text()
checks['fixed_abba'] = "RETOUCH_ORDER = ('off', 'on', 'on', 'off')" in candidate_path.read_text()
checks['source_test_methods'] = sum(isinstance(n, ast.FunctionDef) and n.name.startswith('test_') for n in ast.walk(tests)) == 13
checks['native_launch_guard_in_tests'] = "native process forbidden in Python test" in tests_path.read_text()
report = {'kind': 'AST/static only, not native runtime acceptance', 'checks': checks,
          'candidate_sha256': hashlib.sha256(candidate_path.read_bytes()).hexdigest(),
          'tests_sha256': hashlib.sha256(tests_path.read_bytes()).hexdigest()}
(root / 'sourcechecks.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
if not all(checks.values()):
    raise SystemExit(1)
