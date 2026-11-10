#!/usr/bin/env python3
"""Real Linux subprocess tests. Run only on an isolated hosted fixture."""
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

HELPER = Path(__file__).with_name('remote_session.py')
spec = importlib.util.spec_from_file_location('remote_session', HELPER)
remote = importlib.util.module_from_spec(spec)
spec.loader.exec_module(remote)


class Sessions(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='remote-session-test-')
        self.root = Path(self.tmp.name)
        self.record = self.root / 'session.json'
        self.pidfile = self.root / 'session.pid'
        self.processes = []

    def tearDown(self):
        for process in self.processes:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
        self.tmp.cleanup()

    def launch(self, code, timeout=30, grace=.2):
        process = subprocess.Popen(
            [sys.executable, str(HELPER), 'run', '--record', str(self.record),
             '--pidfile', str(self.pidfile), '--timeout', str(timeout),
             '--grace', str(grace), '--', sys.executable, '-c', code],
            start_new_session=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True)
        self.processes.append(process)
        self.wait_for(lambda: self.record.exists())
        return process

    def wait_for(self, predicate):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(.02)
        self.fail('fixture readiness timed out')

    def control(self, action='stop'):
        return subprocess.run([sys.executable, str(HELPER), action,
                               '--record', str(self.record)], capture_output=True,
                              text=True, timeout=15)

    def nested(self, ignore=False):
        ready = self.root / 'ready'
        # Both the initial payload and its child can belong to different PGIDs.
        code = ("import os,signal,time,pathlib; "
                "child=os.fork(); "
                "os.setpgid(0,0); "
                + ("signal.signal(signal.SIGTERM,signal.SIG_IGN); " if ignore else '')
                + f"pathlib.Path({str(ready)!r}+str(os.getpid())).write_text(str(os.getpgrp())); "
                "time.sleep(60)")
        process = self.launch(code)
        self.wait_for(lambda: len(list(self.root.glob('ready*'))) == 2)
        return process

    def assert_clean(self, process, expected=None):
        output, _ = process.communicate(timeout=12)
        if expected is not None:
            self.assertEqual(process.returncode, expected, output)
        record = remote.read_record(self.record)
        self.assertEqual(record['status'], 'finished', output)
        self.assertFalse(remote.members(record['sid']))
        # Independent known-descendant check, not only the helper's scanner.
        for marker in self.root.glob('ready*'):
            suffix = marker.name.removeprefix('ready')
            if suffix.isdecimal():
                proc = Path('/proc') / suffix / 'stat'
                try:
                    self.assertIn(proc.read_text().split()[2], ('Z', 'X'))
                except FileNotFoundError:
                    pass
        self.assertFalse(self.pidfile.exists())
        return output

    def test_nested_groups_stop_and_unrelated_safety(self):
        unrelated = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'],
                                     start_new_session=True)
        self.processes.append(unrelated)
        process = self.nested()
        self.assertEqual(self.control().returncode, 0)
        self.assert_clean(process, 138)
        self.assertIsNone(unrelated.poll())

    def test_ignored_term_escalates(self):
        process = self.nested(ignore=True)
        begin = time.monotonic()
        self.assertEqual(self.control().returncode, 0)
        self.assertGreaterEqual(time.monotonic() - begin, .2)
        self.assert_clean(process, 138)

    def test_timeout_cleans_nested_groups(self):
        ready = self.root / 'ready'
        code = ("import os,signal,time,pathlib; os.fork(); os.setpgid(0,0); "
                "signal.signal(signal.SIGTERM,signal.SIG_IGN); "
                f"pathlib.Path({str(ready)!r}+str(os.getpid())).touch(); time.sleep(60)")
        process = self.launch(code, timeout=.5)
        self.wait_for(lambda: len(list(self.root.glob('ready*'))) == 2)
        self.assert_clean(process, 124)

    def test_normal_failure_output_and_durable_record_before_payload(self):
        code = (f"import json,pathlib,sys; r=json.loads(pathlib.Path({str(self.record)!r}).read_text()); "
                "assert r['status']=='running' and r['boot'] and r['start']; "
                "print('payload stdout'); print('payload stderr',file=sys.stderr); sys.exit(7)")
        output = self.assert_clean(self.launch(code), 7)
        self.assertIn('payload stdout', output)
        self.assertIn('payload stderr', output)
        self.assertEqual(self.record.stat().st_mode & 0o777, 0o600)

    def test_parent_exit_cleans_background_group(self):
        code = ("import os,time; pid=os.fork(); "
                "os._exit(9) if pid else None; os.setpgid(0,0); time.sleep(60)")
        self.assert_clean(self.launch(code), 9)

    def test_signal_to_guardian_cleans_groups(self):
        process = self.nested(ignore=True)
        process.send_signal(signal.SIGTERM)
        self.assert_clean(process, 143)

    def test_malformed_record_no_signal(self):
        process = self.launch('import time; time.sleep(60)')
        original = self.record.read_bytes()
        self.record.write_text('{not json')
        self.assertNotEqual(self.control().returncode, 0)
        self.assertIsNone(process.poll())
        self.record.write_bytes(original)
        self.assertEqual(self.control().returncode, 0)
        self.assert_clean(process)

    def test_reboot_and_pid_reuse_fail_closed(self):
        process = self.launch('import time; time.sleep(60)')
        original = remote.read_record(self.record)
        for field, value in [('boot', 'different-boot'), ('start', original['start'] + 1),
                             ('sid', original['sid'] + 1), ('pid', 1)]:
            altered = dict(original)
            altered[field] = value
            remote.atomic(self.record, altered)
            self.assertNotEqual(self.control().returncode, 0, field)
            self.assertIsNone(process.poll(), field)
        remote.atomic(self.record, original)
        self.assertEqual(self.control().returncode, 0)
        self.assert_clean(process)

    def test_public_record_rejected(self):
        process = self.launch('import time; time.sleep(60)')
        self.record.chmod(0o644)
        self.assertNotEqual(self.control().returncode, 0)
        self.assertIsNone(process.poll())
        self.record.chmod(0o600)
        self.assertEqual(self.control().returncode, 0)
        self.assert_clean(process)

    def test_changed_record_before_signal(self):
        process = self.launch('import time; time.sleep(60)')
        original = remote.read_record(self.record)
        changed = dict(original, start=original['start'] + 1)
        args = type('Args', (), {'record': str(self.record), 'action': 'stop'})()
        with mock.patch.object(remote, 'read_record', side_effect=[original, changed]), \
                mock.patch.object(signal, 'pidfd_send_signal') as send:
            with self.assertRaisesRegex(ValueError, 'record changed'):
                remote.control(args)
            send.assert_not_called()
        self.assertEqual(self.control().returncode, 0)
        self.assert_clean(process)

    def test_cleanup_failure_is_not_success(self):
        with mock.patch.object(remote, 'members', return_value=[{'pid': 1}]), \
                mock.patch.object(remote, 'sweep'), \
                mock.patch.object(remote.time, 'sleep'), \
                mock.patch.object(remote.time, 'monotonic', side_effect=range(100)):
            self.assertFalse(remote.cleanup(123, .1))

    def test_pidfd_revalidation_prevents_reused_member_signal(self):
        me = remote.identity(os.getpid())
        changed = dict(me, start=me['start'] + 1)
        with mock.patch.object(remote, 'members', return_value=[me]), \
                mock.patch.object(remote, 'identity', return_value=changed), \
                mock.patch.object(signal, 'pidfd_send_signal') as send:
            remote.sweep(me['sid'], signal.SIGTERM)
            send.assert_not_called()

    def test_cleanup_failure_exit_and_record(self):
        code = (f"import sys; sys.path.insert(0,{str(HELPER.parent)!r}); "
                "import remote_session as r; from unittest import mock; "
                "from types import SimpleNamespace; "
                f"a=SimpleNamespace(record={str(self.record)!r},pidfile={str(self.pidfile)!r},"
                "timeout=2,grace=.2,command=[sys.executable,'-c','pass']); "
                "patch=mock.patch.object(r,'cleanup',return_value=False); patch.start(); "
                "sys.exit(r.run(a))")
        result = subprocess.run([sys.executable, '-c', code], start_new_session=True,
                                capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 125, result.stderr)
        self.assertEqual(remote.read_record(self.record)['status'], 'failed')
        self.assertNotEqual(self.control().returncode, 0)

    def test_symlink_record_is_rejected(self):
        target = self.root / 'target'
        target.write_text('{}')
        target.chmod(0o600)
        self.record.symlink_to(target)
        self.assertNotEqual(self.control().returncode, 0)
        self.assertEqual(target.read_text(), '{}')

    def test_duplicate_launch_cannot_replace_active_owner(self):
        process = self.launch('import time; time.sleep(60)')
        original = self.record.read_bytes()
        other = subprocess.run(
            [sys.executable, str(HELPER), 'run', '--record', str(self.record),
             '--pidfile', str(self.pidfile), '--timeout', '1', '--',
             sys.executable, '-c', 'raise Exception("must not run")'],
            start_new_session=True, capture_output=True, text=True, timeout=5)
        self.assertEqual(other.returncode, 125)
        self.assertEqual(self.record.read_bytes(), original)
        self.assertEqual(self.control().returncode, 0)
        self.assert_clean(process)

    def test_term_handler_can_exit_without_kill(self):
        marker = self.root / 'termed'
        code = ("import signal,time,pathlib,sys; "
                f"signal.signal(signal.SIGTERM,lambda *args: (pathlib.Path({str(marker)!r}).touch(),sys.exit(0))); "
                f"pathlib.Path({str(self.root / 'ready')!r}).touch(); time.sleep(60)")
        process = self.launch(code)
        self.wait_for(lambda: (self.root / 'ready').exists())
        self.assertEqual(self.control().returncode, 0)
        self.assert_clean(process, 138)
        self.assertTrue(marker.exists())

    def test_shell_activity_loop_is_cleaned_without_pid_trap(self):
        code = ("import subprocess,sys; "
                "sys.exit(subprocess.run(['bash','-c',"
                "'(while sleep 60; do :; done) & echo shell-output; exit 7']).returncode)")
        output = self.assert_clean(self.launch(code), 7)
        self.assertIn('shell-output', output)

    def test_terminal_record_with_live_members_is_failure(self):
        process = self.nested()
        original = remote.read_record(self.record)
        remote.atomic(self.record, dict(original, status='finished', exit=0))
        self.assertNotEqual(self.control().returncode, 0)
        self.assertIsNone(process.poll())
        remote.atomic(self.record, original)
        self.assertEqual(self.control().returncode, 0)
        self.assert_clean(process)

    def test_gnu_timeout_nested_group_regression(self):
        ready = self.root / 'ready'
        nested = ("import os,signal,time,pathlib; os.fork(); os.setpgid(0,0); "
                  "signal.signal(signal.SIGTERM,signal.SIG_IGN); "
                  f"pathlib.Path({str(ready)!r}+str(os.getpid())).touch(); time.sleep(60)")
        code = ("import subprocess,sys; "
                f"subprocess.run(['timeout','30',sys.executable,'-c',{nested!r}])")
        process = self.launch(code)
        self.wait_for(lambda: len(list(self.root.glob('ready*'))) == 2)
        self.assertEqual(self.control().returncode, 0)
        self.assert_clean(process, 138)

    def test_detached_start_acknowledges_and_stops(self):
        result = subprocess.run(
            [sys.executable, str(HELPER), 'start', '--record', str(self.record),
             '--pidfile', str(self.pidfile), '--log', str(self.root / 'log'),
             '--timeout', '5', '--grace', '.2', '--', sys.executable, '-c',
             'import time; time.sleep(60)'], capture_output=True, text=True, timeout=12)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.control().returncode, 0)
        self.assertEqual(remote.read_record(self.record)['status'], 'finished')


if __name__ == '__main__':
    unittest.main(verbosity=2)
