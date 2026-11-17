#!/usr/bin/env python3
"""Mock-only own-source loop tests. Coordinator runs these on the hosted guest."""
import base64
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import stable_resident_own_loop as suite


def encoded(value):
    return base64.b64encode(value).decode()


ORIGINAL = (b'fn cargo_shim_routes(agent: bool, dev_wrapper: bool, nested: bool, setting: Option<&str>) -> bool {\n'
            b'    match setting {\n' + suite.BODY_ANCHOR + b'    }\n}\n')


class OwnLoopTests(unittest.TestCase):
    def prepare(self, root):
        checkout = root / 'checkout'
        (checkout / 'src').mkdir(parents=True)
        (checkout / 'Cargo.toml').write_text('[package]\nname="justrust"\nversion="0.0.0"\n')
        source = checkout / 'src/lib.rs'
        source.write_bytes(ORIGINAL)
        paths = {}
        for name in ('compiler', 'cargo', 'client', 'driver'):
            paths[name] = root / name
            paths[name].write_bytes(name.encode())
        attestation = {name: dict(path=str(path), sha256=suite.shared.native.digest(path))
                       for name, path in paths.items()}
        options = dict(compiler=paths['compiler'], cargo=paths['cargo'], justrust=paths['client'],
                       output=root / 'o', checkout=checkout, repetitions=2,
                       root_native_frozen=True, idle_authorized=True,
                       **{'expected_' + name + '_sha256': item['sha256']
                          for name, item in attestation.items()})
        return source, options, attestation

    def runner(self, fail_at=None, mutate=None, initial_fresh=False):
        counters = {}
        calls = []

        def run(command, timeout, cwd, env):
            calls.append((command, timeout, cwd, env))
            if fail_at == len(calls):
                raise RuntimeError('injected runner failure')
            trace = Path(env['JUSTRUST_CARGO_RESIDENT_TRACE'])
            key = (str(trace), command[2])
            index = counters.get(key, 0)
            counters[key] = index + 1
            fresh = index == 1 or (initial_fresh and index == 0)
            if env['JUSTRUST_CARGO_RESIDENT'] == 'serial' and not fresh:
                event = dict(event='complete', crate_name='justrust', key='root-' + command[2],
                             elapsed_ns=1234, result=dict(pid=9876543, id=index + 1, status=0))
                if mutate:
                    mutate(event, index)
                with trace.open('a') as stream:
                    stream.write(json.dumps(event) + '\n')
            stdout = '\n'.join(json.dumps(message) for message in (
                dict(reason='compiler-artifact', target=dict(name='justrust', kind=['lib']), fresh=fresh),
                dict(reason='build-finished', success=True))) + '\n'
            return dict(status=0, timed_out=False, stdout_b64=encoded(stdout.encode()),
                        stderr_b64=encoded(b'raw stderr\xff\x00\n'))
        return run, calls

    def capture(self, root, commands=('check', 'build'), **runner_options):
        source, options, attestation = self.prepare(root)
        original = source.read_bytes()
        stat = source.stat()
        runner, calls = self.runner(**runner_options)
        with patch.object(suite.safety, 'attest', return_value=attestation):
            report = suite.capture(**options, commands=commands, runner=runner)
        self.assertEqual(source.read_bytes(), original)
        self.assertEqual(source.stat().st_mtime_ns, stat.st_mtime_ns)
        return report, calls, options

    def test_counterbalanced_actual_client_commands_and_raw_streams(self):
        with tempfile.TemporaryDirectory() as directory:
            report, calls, options = self.capture(Path(directory))
            self.assertEqual(report['verdict'], 'passed')
            self.assertTrue(report['restoration']['restored'])
            self.assertEqual([p['order'] for p in report['repetitions']], [['off', 'on'], ['on', 'off']])
            self.assertEqual(len(calls), 40)
            self.assertEqual(set(suite.validate(report)), {'check', 'build'})
            for command, _, cwd, _ in calls:
                self.assertEqual(command[:2], [options['justrust'], 'cargo'])
                self.assertIn(command[2], ('check', 'build'))
                self.assertEqual(command[3:], ['--lib', '--message-format=json'])
                self.assertEqual(cwd, options['checkout'])
            for _, _, _, env in calls:
                for key, value in suite.STRICT_ENV.items():
                    self.assertEqual(env[key], value)
            for pair in report['repetitions']:
                for arm in pair['arms'].values():
                    for row in arm['observations']:
                        self.assertEqual(base64.b64decode(row['cargo']['stderr_b64']), b'raw stderr\xff\x00\n')
                        self.assertIn('resident_worker_rss', row['cargo'])
            self.assertEqual(json.loads((options['output'] / 'receipt.json').read_text()), report)

    def test_failure_restores_backup_and_retains_failure_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            source, options, attestation = self.prepare(Path(directory))
            original = source.read_bytes()
            runner, _ = self.runner(fail_at=4)
            with patch.object(suite.safety, 'attest', return_value=attestation):
                with self.assertRaisesRegex(RuntimeError, 'injected'):
                    suite.capture(**options, runner=runner)
            self.assertEqual(source.read_bytes(), original)
            self.assertEqual((options['output'] / 'original-lib.rs').read_bytes(), original)
            receipt = json.loads((options['output'] / 'receipt.json').read_text())
            self.assertEqual(receipt['verdict'], 'failed')
            self.assertTrue(receipt['restoration']['restored'])

    def test_dependency_completion_cannot_substitute_for_root(self):
        with tempfile.TemporaryDirectory() as directory:
            source, options, attestation = self.prepare(Path(directory))
            runner, _ = self.runner(mutate=lambda event, _: event.update(crate_name='dependency'))
            with patch.object(suite.safety, 'attest', return_value=attestation):
                with self.assertRaisesRegex(AssertionError, 'root crate_name'):
                    suite.capture(**options, runner=runner)
            self.assertEqual(source.read_bytes(), ORIGINAL)

    def test_worker_change_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            _, options, attestation = self.prepare(Path(directory))
            def mutate(event, index):
                if index == 3:
                    event['result']['pid'] += 1
            runner, _ = self.runner(mutate=mutate)
            with patch.object(suite.safety, 'attest', return_value=attestation):
                with self.assertRaisesRegex(AssertionError, 'worker changed'):
                    suite.capture(**options, runner=runner)

    def test_fresh_and_environment_tampering_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            report, _, _ = self.capture(Path(directory))
            bad = copy.deepcopy(report)
            rows = bad['repetitions'][0]['arms']['on']['observations']
            rows[1]['trace'] = rows[0]['trace']
            with self.assertRaisesRegex(AssertionError, 'fresh'):
                suite.validate(bad)
            bad = copy.deepcopy(report)
            bad['repetitions'][0]['arms']['off']['observations'][0]['cargo']['invocation']['command'][2] = 'test'
            with self.assertRaisesRegex(ValueError, 'command mismatch'):
                suite.validate(bad)

    def test_initial_fresh_excluded_but_repeated_edits_require_same_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            report, _, _ = self.capture(Path(directory), initial_fresh=True)
            for pair in report['repetitions']:
                for arm in pair['arms'].values():
                    self.assertTrue(arm['observations'][0]['semantics']['fresh'])
                    self.assertEqual(arm['observations'][0]['trace'], [])
            suite.validate(report)

    def test_actual_body_and_optional_comment_modes(self):
        body = suite.stages(ORIGINAL, 'nonce')
        self.assertIn(b'_ => agent && dev_wrapper,', body[2][1])
        self.assertIn(b'_ => !agent || dev_wrapper,', body[3][1])
        self.assertEqual(body[-1][1], ORIGINAL)
        comments = suite.stages(ORIGINAL, 'nonce', 'comment')
        self.assertTrue(comments[2][1].startswith(ORIGINAL))
        self.assertIn(b'// jr-own-loop nonce edit 1', comments[2][1])
        with self.assertRaises(ValueError):
            suite.stages(b'no anchor', 'nonce')

    def test_selected_commands_and_rejection(self):
        for selected in (('check',), ('build',), ('build', 'check')):
            with self.subTest(commands=selected), tempfile.TemporaryDirectory() as directory:
                report, calls, _ = self.capture(Path(directory), commands=selected)
                self.assertEqual(report['provenance']['commands'], list(selected))
                self.assertEqual(len(calls), len(selected) * 20)
                self.assertEqual(set(suite.validate(report)), set(selected))
                self.assertEqual(set(suite.compare(report, report)), set(selected))
                self.assertEqual({command[2] for command, _, _, _ in calls}, set(selected))
        for selected in ((), ('test',), ('check', 'check'), 'check'):
            with self.assertRaisesRegex(ValueError, 'Commands'):
                suite.capture(None, None, None, None, commands=selected)

    def test_compare_authorizes_only_explicit_compiler_and_driver_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            before, _, _ = self.capture(Path(directory), commands=('check',))
            after = copy.deepcopy(before)
            for name, digest in (('compiler', 'a' * 64), ('driver', 'b' * 64)):
                after['provenance']['expected'][name + '_sha256'] = digest
                after['provenance']['attestation'][name].update(sha256=digest, path='/new/' + name)
            for pair in after['repetitions']:
                for arm in pair['arms'].values():
                    for row in arm['observations']:
                        row['cargo']['invocation']['environment']['RUSTC'] = '/new/compiler'
            approval = dict(expected_before_compiler_sha256=before['provenance']['expected']['compiler_sha256'],
                            expected_before_driver_sha256=before['provenance']['expected']['driver_sha256'],
                            expected_after_compiler_sha256='a' * 64, expected_after_driver_sha256='b' * 64)
            self.assertEqual(set(suite.compare(before, after, **approval)), {'check'})
            with self.assertRaisesRegex(ValueError, 'Provenance mismatch'):
                suite.compare(before, after)
            partial = dict(approval)
            partial.pop('expected_after_driver_sha256')
            with self.assertRaisesRegex(ValueError, 'hash mismatch'):
                suite.compare(before, after, **partial)
            wrong = dict(approval, expected_after_compiler_sha256='c' * 64)
            with self.assertRaisesRegex(ValueError, 'hash mismatch'):
                suite.compare(before, after, **wrong)
            for change in ('host', 'client', 'cargo', 'workload'):
                bad = copy.deepcopy(after)
                if change == 'host':
                    bad['provenance']['host']['boot_id'] = 'different-boot'
                elif change == 'workload':
                    bad['provenance']['manifest_b64'] = encoded(b'different manifest')
                else:
                    bad['provenance']['expected'][change + '_sha256'] = 'c' * 64
                    bad['provenance']['attestation'][change]['sha256'] = 'c' * 64
                with self.subTest(change=change), self.assertRaisesRegex(ValueError, 'Provenance mismatch'):
                    suite.compare(before, bad, **approval)

    def test_console_summary_does_not_emit_raw_receipt_or_mutate_it(self):
        report = dict(schema=suite.SCHEMA, verdict='passed', claim=suite.CLAIM,
                      summary={'check': {}}, restoration={'restored': True},
                      provenance={'original_b64': 'secret-source'},
                      repetitions=[{'stdout_b64': 'raw-command-stream'}])
        original = copy.deepcopy(report)
        summary = suite.console_summary(report, Path('/private/output'))
        self.assertEqual(set(summary), {'schema', 'verdict', 'claim', 'receipt', 'summary', 'restoration'})
        self.assertEqual(summary['receipt'], '/private/output/receipt.json')
        self.assertNotIn('secret-source', json.dumps(summary))
        self.assertNotIn('raw-command-stream', json.dumps(summary))
        self.assertEqual(report, original)

    def test_authorization_and_hash_required_before_runner(self):
        with tempfile.TemporaryDirectory() as directory:
            _, options, _ = self.prepare(Path(directory))
            runner, calls = self.runner()
            options['idle_authorized'] = False
            with self.assertRaisesRegex(ValueError, 'authorization'):
                suite.capture(**options, runner=runner)
            self.assertEqual(calls, [])
            options['idle_authorized'] = True
            options['expected_compiler_sha256'] = '0' * 64
            # Real attestation fails before running any candidate or touching source.
            with self.assertRaises(ValueError):
                suite.capture(**options, runner=runner)
            self.assertEqual(calls, [])


if __name__ == '__main__':
    unittest.main()
