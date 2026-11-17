"""Hosted probe regression tests. Python only, no compiler required."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import hosted_probe
from test_probe import ProbeTests, gen


class TransportTests(unittest.TestCase):
    def client(self):
        with mock.patch.object(hosted_probe.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0,
                "JR_REMOTE_KIND=hosted\nJR_REMOTE_DEST=guest@host\nJR_REMOTE_SSH='ssh -p 22001 -i /key'\n", '')):
            return hosted_probe.HostedProbe()

    def test_parse_data_not_eval(self):
        values = hosted_probe.parse_env("JR_REMOTE_KIND=hosted\nJR_REMOTE_DEST=host\nJR_REMOTE_SSH='ssh $(touch /never)'\n")
        self.assertEqual(values['JR_REMOTE_SSH'], 'ssh $(touch /never)')
        for text in ('JR_REMOTE_KIND=local', 'JR_REMOTE_KIND=hosted\nexport BAD=1',
                     'JR_REMOTE_KIND=hosted\nJR_REMOTE_KIND=hosted'):
            with self.assertRaises(RuntimeError):
                hosted_probe.parse_env(text)

    def test_auth_failure_never_falls_back(self):
        with mock.patch.object(hosted_probe.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, '', 'denied')) as run:
            with self.assertRaisesRegex(RuntimeError, 'authentication'):
                hosted_probe.HostedProbe()
            self.assertEqual(run.call_count, 1)

    def test_sync_whole_snapshot_excludes_outputs_and_uses_full_shell(self):
        client = self.client()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            cwd = root / 'workspace'
            cwd.mkdir()
            (cwd / 'Cargo.toml').write_text('')
            with mock.patch.object(hosted_probe.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, '', '')) as run:
                client.sync(root, cwd)
            command = run.call_args.args[0]
            self.assertIn(client.shell, command)
            self.assertIn('target/', command)
            self.assertIn('.justrust/', command)
            self.assertEqual(command[-2:], [str(root) + '/', 'guest@host:' + str(root) + '/'])
            (root / 'escape').symlink_to('/etc')
            with self.assertRaisesRegex(RuntimeError, 'escapes'):
                client.sync(root, cwd)

    def test_sync_failure_is_fatal(self):
        client = self.client()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'Cargo.toml').write_text('')
            with mock.patch.object(client, 'run'), mock.patch.object(hosted_probe.subprocess, 'run',
                    return_value=subprocess.CompletedProcess([], 23, '', 'sync denied')):
                with self.assertRaisesRegex(RuntimeError, 'sync denied'):
                    client.sync(root, root)

    def test_identity_requires_installed_selected_compiler(self):
        client = self.client()
        with mock.patch.object(client, 'sync'), mock.patch.object(client, 'run',
                return_value=subprocess.CompletedProcess([], 0, 'justrust x\npinned  missing (not installed yet)\n', '')):
            with self.assertRaisesRegex(RuntimeError, 'installed'):
                client.identity(Path('/snapshot'), Path('/snapshot'))

    def test_raw_failure_preserved_and_transport_failure_fatal(self):
        client = self.client()
        with mock.patch.object(client, 'sync'), mock.patch.object(client, 'run', return_value=subprocess.CompletedProcess([], 101, '{"reason":"compiler-message"}\n', 'raw stderr')) as run:
            result = client.check(Path('/snapshot'), Path('/snapshot/project'), ['--message-format=json'], Path('/target'))
        self.assertEqual(result.returncode, 101)
        self.assertEqual(result.stderr, 'raw stderr')
        self.assertIn('justrust cargo check', run.call_args.args[0])
        self.assertIn('--message-format=json', run.call_args.args[0])
        with mock.patch.object(client, 'sync'), mock.patch.object(client, 'run', return_value=subprocess.CompletedProcess([], 255, '', 'lost')):
            with self.assertRaisesRegex(RuntimeError, 'lost'):
                client.check(Path('/snapshot'), Path('/snapshot/project'), [], Path('/target'))


class RemoteIntegrationTests(ProbeTests):
    def test_remote_mode_ignores_local_cache_and_records_identity(self):
        cache = gen.WORK / 'probe/snapshot/fixture.json'
        cache.parent.mkdir(parents=True)
        cache.write_text(json.dumps({'wrong.rs': {'body': [999], 'test': []}}))
        identity = {'mode': 'authenticated-hosted-v1', 'compiler': 'published compiler'}
        with mock.patch.dict(os.environ, JUSTRUST_SYNTH_PROBE_REMOTE='1'), \
             mock.patch.object(gen.hosted_probe, 'HostedProbe') as client, \
             mock.patch.object(gen, '_probe_run', return_value=({0}, set(), set())):
            client.return_value.identity.return_value = identity
            result = gen.probe(self.ws, 'fixture')
        self.assertEqual(result, {'src.rs': {'body': [0], 'test': []}})
        remote = gen.WORK / 'probe-remote' / hosted_probe.cache_key(identity) / 'snapshot/fixture.json'
        self.assertEqual(json.loads(remote.read_text())['provenance'], identity)
        self.assertIn('wrong.rs', json.loads(cache.read_text()))

    def test_compiler_change_aborts_and_restores(self):
        with mock.patch.dict(os.environ, JUSTRUST_SYNTH_PROBE_REMOTE='1'), \
             mock.patch.object(gen.hosted_probe, 'HostedProbe') as client:
            client.return_value.identity.return_value = {'compiler': 'new'}
            with self.assertRaisesRegex(RuntimeError, 'compiler changed'):
                gen._probe_run(self.ws, self.pkg, {0: (self.source, 0)}, False,
                               expected_identity={'compiler': 'old'})
            client.return_value.check.assert_not_called()
            client.return_value.sync.assert_called_once()
        self.assertEqual(self.source.read_text(), self.original)
        self.assertFalse(gen.JOURNAL.exists())

    def test_remote_source_escape_is_rejected_before_edit(self):
        with tempfile.TemporaryDirectory() as outside:
            external = Path(outside) / 'external.rs'
            external.write_text(self.original)
            self.source.unlink()
            self.source.symlink_to(external)
            with mock.patch.dict(os.environ, JUSTRUST_SYNTH_PROBE_REMOTE='1'):
                with self.assertRaisesRegex(RuntimeError, 'escapes owned snapshot'):
                    gen._probe_run(self.ws, self.pkg, {0: (self.source, 0)}, False)
            self.assertEqual(external.read_text(), self.original)

    def test_remote_probe_never_invokes_stock_cargo(self):
        diag = self.diagnostic('jrprobe_0', 'E0308', self.source)
        with mock.patch.dict(os.environ, JUSTRUST_SYNTH_PROBE_REMOTE='1'), \
             mock.patch.object(gen.hosted_probe, 'HostedProbe') as client, \
             mock.patch.object(gen.subprocess, 'run', side_effect=AssertionError('local invocation')):
            client.return_value.identity.return_value = {'compiler': 'published'}
            client.return_value.check.return_value = subprocess.CompletedProcess([], 101, json.dumps(diag), '')
            self.assertEqual(gen._probe_run(self.ws, self.pkg, {0: (self.source, 0)}, False), ({0}, set(), set()))
        self.assertEqual(self.source.read_text(), self.original)


if __name__ == '__main__':
    unittest.main()
