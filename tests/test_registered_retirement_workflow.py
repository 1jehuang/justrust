"""Offline registered dispatch tests, no live calls, builds, or credentials."""
import contextlib
import io
import json
import os
from pathlib import Path
import textwrap
import unittest
from unittest import mock

import test_native_retirement_workflow as original

WORKFLOW = Path(__file__).resolve().parents[1] / '.github/workflows/hosted-linux-validation.yml'


class RegisteredRetirementTests(original.RetirementWorkflowTests):
    def setUp(self):
        self.patch = mock.patch.object(original, 'WORKFLOW', WORKFLOW)
        self.patch.start()
        self.addCleanup(self.patch.stop)

    def test_manual_trusted_source_and_secret_scope_contract(self):
        workflow = WORKFLOW.read_text()
        self.assertIn("if: inputs.mode == 'linux-build' || inputs.mode == ''", workflow)
        self.assertIn("if: inputs.mode == 'native-retirement' && github.ref == 'refs/heads/stable'", workflow)
        self.assertIn('default: linux-build', workflow)
        self.assertEqual(workflow.count('group: justrust-hosted-linux'), 1)
        self.assertNotIn('workflow_call:', workflow)
        self.assertNotIn('continue-on-error:', workflow)
        self.assertIn('git merge-base --is-ancestor "$SOURCE_REF" refs/remotes/origin/stable', workflow)
        self.assertIn('[[ "$EXCLUSIVE_CANARY_FENCE" == true ]]', workflow)
        retirement = workflow.split('  retirement:', 1)[1]
        self.assertNotIn('upload-artifact', retirement)
        self.assertNotIn('justrust build', retirement)
        self.assertEqual(retirement.count('secrets.JCODE_API_KEY'), 2)
        self.assertIn("if: always() && steps.harness.outcome != 'skipped' && steps.harness.outcome != ''", retirement)
        self.assertIn('id: harness', retirement)

    def rollback(self, response, lease='b' * 64, error=None):
        section = WORKFLOW.read_text().split('      - name: Independent exact-lease rollback', 1)[1]
        code = textwrap.dedent(section.split("python3 - <<'PY'\n", 1)[1].split('          PY\n', 1)[0])
        self.assertNotIn('/connect', code)
        opener = mock.Mock()
        opener.open.return_value = io.BytesIO(json.dumps(response).encode())
        if error:
            opener.open.side_effect = error
        output = io.StringIO()
        with mock.patch.dict(os.environ, EXPECTED_LEASE=lease, JCODE_API_KEY='offline-test-key'), \
                mock.patch('urllib.request.build_opener', return_value=opener) as build, \
                contextlib.redirect_stdout(output):
            exec(compile(code, str(WORKFLOW), 'exec'), {})
            handler = build.call_args.args[0]
            with self.assertRaises(RuntimeError):
                handler.redirect_request(None, None, None, None, None, None)
        self.assertNotIn('offline-test-key', output.getvalue())
        return opener

    def test_independent_rollback_binds_exact_lease_and_fixed_origin(self):
        opener = self.rollback({'native_only': False, 'ssh': 'restoring', 'private': 'not logged'})
        request = opener.open.call_args.args[0]
        self.assertEqual(request.full_url, 'https://build.jcode.sh/v1/build/host/native-only')
        self.assertEqual(json.loads(request.data), {'action': 'rollback', 'lease_id': 'b' * 64})
        self.assertEqual(opener.open.call_args.kwargs, {'timeout': 15})

    def test_independent_rollback_failures_are_suppressed_and_fail_closed(self):
        for response in ({}, {'native_only': True, 'ssh': 'restoring'}, {'native_only': False, 'ssh': 'ready'}):
            with self.subTest(response=response), self.assertRaises(SystemExit) as error:
                self.rollback(response)
            self.assertNotIn(str(response), str(error.exception))
        with self.assertRaises(SystemExit) as error:
            self.rollback({}, error=RuntimeError('private fixture or credentials'))
        self.assertNotIn('private fixture', str(error.exception))
        with self.assertRaises(SystemExit):
            self.rollback({}, lease='replacement')


if __name__ == '__main__':
    unittest.main()
