#!/usr/bin/env python3
"""Mock-only own-source loop tests. Coordinator runs these on the hosted guest."""
import base64
import copy
import json
import os
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

    def capture(self, root, commands=('check', 'build'), *, diagnostic_on_only=False,
                repetitions=2, macro_shm=False, macro_spins=0, profile_phases=False, **runner_options):
        source, options, attestation = self.prepare(root)
        options.update(diagnostic_on_only=diagnostic_on_only, repetitions=repetitions,
                       macro_shm=macro_shm, macro_spins=macro_spins, profile_phases=profile_phases)
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

    def test_diagnostic_on_only_is_correctness_not_performance(self):
        for repetitions in (1, 2, 3):
            with self.subTest(repetitions=repetitions), tempfile.TemporaryDirectory() as directory:
                report, calls, options = self.capture(
                    Path(directory), commands=('check',), diagnostic_on_only=True,
                    repetitions=repetitions)
                self.assertEqual(report['schema'], suite.DIAGNOSTIC_SCHEMA)
                self.assertEqual(report['claim'], suite.DIAGNOSTIC_CLAIM)
                self.assertEqual(report['verdict'], 'diagnostic-passed')
                self.assertNotIn('summary', report)
                self.assertEqual(len(calls), 5 * repetitions)
                self.assertTrue(all(pair['order'] == ['on'] and set(pair['arms']) == {'on'}
                                    for pair in report['repetitions']))
                self.assertTrue(all(env['JUSTRUST_CARGO_RESIDENT'] == 'serial'
                                    for _, _, _, env in calls))
                self.assertTrue(report['restoration']['restored'])
                self.assertEqual(json.loads((options['output'] / 'receipt.json').read_text()), report)
                with self.assertRaises(ValueError):
                    suite.validate(report)
                with self.assertRaises(ValueError):
                    suite.compare(report, report)
                # Even a stray summary must not leak as diagnostic performance evidence.
                report['summary'] = {'unsafe': 'performance'}
                self.assertEqual(suite.console_summary(report, options['output'])['summary'], {})

    def test_one_repetition_requires_explicit_diagnostic_opt_in(self):
        with tempfile.TemporaryDirectory() as directory:
            _, options, _ = self.prepare(Path(directory))
            options['repetitions'] = 1
            runner, calls = self.runner()
            with self.assertRaisesRegex(ValueError, 'Even repetition'):
                suite.capture(**options, runner=runner)
            self.assertEqual(calls, [])
            for value in (0, -1, True, 1.5):
                options.update(repetitions=value, diagnostic_on_only=True)
                with self.subTest(repetitions=value), self.assertRaisesRegex(ValueError, 'Positive diagnostic'):
                    suite.capture(**options, runner=runner)
            self.assertEqual(calls, [])

    def test_profile_phases_requires_diagnostic_before_execution(self):
        with tempfile.TemporaryDirectory() as directory:
            _, options, _ = self.prepare(Path(directory))
            runner, calls = self.runner()
            with self.assertRaisesRegex(ValueError, 'requires diagnostic'):
                suite.capture(**options, profile_phases=True, runner=runner)
            for value in (None, 0, 1, 'true'):
                with self.subTest(value=value), self.assertRaisesRegex(ValueError, 'must be boolean'):
                    suite.capture(**options, diagnostic_on_only=True, profile_phases=value, runner=runner)
            self.assertEqual(calls, [])
            self.assertFalse(options['output'].exists())

    def test_profile_phases_actual_commands_environment_and_raw_stderr(self):
        for enabled in (False, True):
            with self.subTest(enabled=enabled), tempfile.TemporaryDirectory() as directory:
                with patch.dict(os.environ, RUSTFLAGS='inherited flags must not leak'):
                    report, calls, options = self.capture(
                        Path(directory), diagnostic_on_only=True, repetitions=1,
                        profile_phases=enabled)
                self.assertEqual(len(calls), 10)
                self.assertIs(report['provenance']['profile_phases'], enabled)
                self.assertEqual(report['verdict'], 'diagnostic-passed')
                expected = suite.shared.FLAGS + (' -Ztime-passes' if enabled else '')
                for command, _, _, env in calls:
                    self.assertEqual(command[:2], [options['justrust'], 'cargo'])
                    self.assertIn(command[2], ('check', 'build'))
                    self.assertEqual(command[3:], ['--lib', '--message-format=json'])
                    self.assertEqual(env['RUSTFLAGS'], expected)
                suite._validate(report, diagnostic=True)
                self.assertNotIn('summary', report)
                for row in report['repetitions'][0]['arms']['on']['observations']:
                    self.assertEqual(base64.b64decode(row['cargo']['stderr_b64']), b'raw stderr\xff\x00\n')
                self.assertEqual(json.loads((options['output'] / 'receipt.json').read_text()), report)
                with self.assertRaises(ValueError):
                    suite.validate(report)
                with self.assertRaises(ValueError):
                    suite.compare(report, report)

    def test_profile_phases_provenance_and_environment_tampering_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            report, _, _ = self.capture(Path(directory), diagnostic_on_only=True,
                                        repetitions=1, profile_phases=True)
            for value in (None, 0, 1, 'true', False):
                bad = copy.deepcopy(report)
                if value is None:
                    bad['provenance'].pop('profile_phases')
                else:
                    bad['provenance']['profile_phases'] = value
                with self.subTest(value=value), self.assertRaises(ValueError):
                    suite._validate(bad, diagnostic=True)
            for flags in (suite.shared.FLAGS, suite.shared.FLAGS + ' -Ztime-passes -Ztime-passes',
                          suite.shared.FLAGS + ' -Ztime-passes=on'):
                bad = copy.deepcopy(report)
                bad['repetitions'][0]['arms']['on']['observations'][0]['cargo']['invocation']['environment']['RUSTFLAGS'] = flags
                with self.subTest(flags=flags), self.assertRaisesRegex(ValueError, 'environment mismatch'):
                    suite._validate(bad, diagnostic=True)
            forged = copy.deepcopy(report)
            forged.update(schema=suite.SCHEMA, verdict='passed')
            with self.assertRaisesRegex(ValueError, 'requires diagnostic'):
                suite.validate(forged)
            with self.assertRaises(ValueError):
                suite.compare(forged, forged)

    def test_profile_phases_cannot_be_enabled_in_performance_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            report, _, _ = self.capture(Path(directory))
            self.assertIs(report['provenance']['profile_phases'], False)
            for pair in report['repetitions']:
                for arm in pair['arms'].values():
                    for row in arm['observations']:
                        self.assertEqual(row['cargo']['invocation']['environment']['RUSTFLAGS'], suite.shared.FLAGS)
            bad = copy.deepcopy(report)
            bad['provenance']['profile_phases'] = True
            with self.assertRaisesRegex(ValueError, 'requires diagnostic'):
                suite.validate(bad)
            bad = copy.deepcopy(report)
            bad['repetitions'][0]['arms']['off']['observations'][0]['cargo']['invocation']['environment']['RUSTFLAGS'] += ' -Ztime-passes'
            with self.assertRaisesRegex(ValueError, 'environment mismatch'):
                suite.validate(bad)

    def test_historical_missing_profile_flag_validates_and_compares_without_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            current, _, _ = self.capture(Path(directory))
            historical = copy.deepcopy(current)
            historical['provenance'].pop('profile_phases')
            original = copy.deepcopy(historical)
            current_original = copy.deepcopy(current)
            self.assertEqual(suite.validate(historical), suite.validate(current))
            expected = suite.compare(current, current)
            self.assertEqual(suite.compare(historical, historical), expected)
            self.assertEqual(suite.compare(historical, current), expected)
            self.assertEqual(suite.compare(current, historical), expected)
            self.assertEqual(historical, original)
            self.assertEqual(current, current_original)
            for value in (None, 0, 1, 'false'):
                bad = copy.deepcopy(historical)
                bad['provenance']['profile_phases'] = value
                with self.subTest(value=value), self.assertRaisesRegex(ValueError, 'profiling provenance'):
                    suite.validate(bad)
            for mode in ('off', 'on'):
                bad = copy.deepcopy(historical)
                bad['repetitions'][0]['arms'][mode]['observations'][0]['cargo']['invocation']['environment']['RUSTFLAGS'] += ' -Ztime-passes'
                with self.subTest(mode=mode), self.assertRaisesRegex(ValueError, 'environment mismatch'):
                    suite.validate(bad)
                with self.assertRaises(ValueError):
                    suite.compare(historical, bad)

    def test_historical_missing_profile_flag_does_not_relax_diagnostic_claims(self):
        with tempfile.TemporaryDirectory() as directory:
            report, _, _ = self.capture(Path(directory), diagnostic_on_only=True, repetitions=1)
            report['provenance'].pop('profile_phases')
            original = copy.deepcopy(report)
            suite._validate(report, diagnostic=True)
            with self.assertRaises(ValueError):
                suite.validate(report)
            with self.assertRaises(ValueError):
                suite.compare(report, report)
            self.assertEqual(report, original)
            profiled = copy.deepcopy(report)
            profiled['repetitions'][0]['arms']['on']['observations'][0]['cargo']['invocation']['environment']['RUSTFLAGS'] += ' -Ztime-passes'
            with self.assertRaisesRegex(ValueError, 'environment mismatch'):
                suite._validate(profiled, diagnostic=True)

    def test_diagnostic_failure_restores_and_preserves_raw_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            source, options, attestation = self.prepare(Path(directory))
            options.update(repetitions=1, diagnostic_on_only=True)
            stat = source.stat()
            runner, calls = self.runner(fail_at=4)
            with patch.object(suite.safety, 'attest', return_value=attestation):
                with self.assertRaisesRegex(RuntimeError, 'injected'):
                    suite.capture(**options, runner=runner)
            self.assertEqual(len(calls), 4)
            self.assertEqual(source.read_bytes(), ORIGINAL)
            self.assertEqual(source.stat().st_mtime_ns, stat.st_mtime_ns)
            receipt = json.loads((options['output'] / 'receipt.json').read_text())
            self.assertEqual(receipt['schema'], suite.DIAGNOSTIC_SCHEMA)
            self.assertEqual(receipt['claim'], suite.DIAGNOSTIC_CLAIM)
            self.assertEqual(receipt['verdict'], 'failed')
            self.assertTrue(receipt['restoration']['restored'])
            row = receipt['repetitions'][0]['arms']['on']['observations'][0]
            self.assertEqual(base64.b64decode(row['cargo']['stderr_b64']), b'raw stderr\xff\x00\n')
            self.assertNotIn('summary', receipt)

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

    def retirement_rows(self):
        rows = []
        for index, name in enumerate(suite.STAGES):
            initial = index == 0
            result = dict(pid=152845 if initial else 153293, id=index + 1, status=0,
                          external_inputs_changed=initial, retire=initial)
            event = dict(event='complete', crate_name='justrust', key='root',
                         elapsed_ns=1234, result=result)
            rows.append(dict(name=name, semantics=dict(fresh=index == 1),
                             trace=[] if index == 1 else [event],
                             cargo=dict(resident_worker_rss=[dict(pid=result['pid'])])))
        return rows

    def test_only_initial_explicit_safety_retirement_permits_new_generation(self):
        rows = self.retirement_rows()
        raw = copy.deepcopy(rows)
        evidence = suite.validate_residency(rows, 'on')
        self.assertEqual(rows, raw)
        self.assertEqual(evidence, dict(root_worker_key='root', root_worker_pid=153293,
                                       initial_safety_retirement=dict(root_worker_key='root',
                                           root_worker_pid=152845, status=0,
                                           external_inputs_changed=True, retire=True)))
        # Key changes are also permitted only between the evicted cold worker
        # and the single stable measured generation.
        rows[0]['trace'][0]['key'] = 'cold-root'
        self.assertEqual(suite.validate_residency(rows, 'on')['root_worker_key'], 'root')

    def test_initial_retirement_requires_both_true_flags_and_successful_rpc(self):
        for field, value in (('external_inputs_changed', None), ('retire', None),
                             ('external_inputs_changed', False), ('retire', False),
                             ('external_inputs_changed', 1), ('retire', 1),
                             ('status', None), ('status', 1), ('status', False)):
            with self.subTest(field=field, value=value):
                rows = self.retirement_rows()
                result = rows[0]['trace'][0]['result']
                if value is None:
                    result.pop(field)
                else:
                    result[field] = value
                with self.assertRaises(AssertionError):
                    suite.validate_residency(rows, 'on')
        rows = self.retirement_rows()
        rows[0]['trace'][0]['result'].update(external_inputs_changed=False, retire=False)
        with self.assertRaisesRegex(AssertionError, 'worker changed'):
            suite.validate_residency(rows, 'on')

    def test_retirement_never_allows_fallback_or_extra_root_rpc(self):
        for index in (0, 2, 3, 4):
            for extra in ('fallback', 'duplicate', 'result-fallback', 'retirement-trace'):
                if index == 0 and extra == 'retirement-trace':
                    continue
                with self.subTest(index=index, extra=extra):
                    rows = self.retirement_rows()
                    if extra == 'fallback':
                        rows[index]['trace'].append(dict(event='fallback', crate_name='justrust'))
                    elif extra == 'duplicate':
                        rows[index]['trace'].append(copy.deepcopy(rows[index]['trace'][0]))
                    elif extra == 'retirement-trace':
                        rows[index]['trace'].append(dict(event='retire', crate_name='justrust'))
                    else:
                        rows[index]['trace'][0]['result']['fallback'] = True
                    with self.assertRaises(AssertionError):
                        suite.validate_residency(rows, 'on')

    def test_measured_generations_require_stable_key_pid_and_no_retirement(self):
        for index in (2, 3, 4):
            for field, value in (('pid', 999999), ('key', 'replacement'),
                                 ('retire', True), ('external_inputs_changed', True),
                                 ('both', True)):
                # The first measured worker establishes identity, so change it
                # and leave subsequent workers unchanged to test that boundary.
                with self.subTest(index=index, field=field):
                    rows = self.retirement_rows()
                    event = rows[index]['trace'][0]
                    if field == 'key':
                        event[field] = value
                    elif field == 'both':
                        event['result'].update(retire=True, external_inputs_changed=True)
                    else:
                        event['result'][field] = value
                    with self.assertRaises(AssertionError):
                        suite.validate_residency(rows, 'on')

    def test_initial_safety_retirement_preserves_strict_default_capture(self):
        def mutate(event, index):
            if index == 0:
                event['result'].update(pid=152845, external_inputs_changed=True, retire=True)
        with tempfile.TemporaryDirectory() as directory:
            report, calls, _ = self.capture(Path(directory), mutate=mutate)
            self.assertEqual(len(calls), 40)
            self.assertEqual(set(suite.validate(report)), {'check', 'build'})
            self.assertEqual(report['schema'], suite.SCHEMA)
            for pair in report['repetitions']:
                for command in ('check', 'build'):
                    evidence = pair['arms']['on']['residency'][command]
                    self.assertEqual(evidence['root_worker_pid'], 9876543)
                    self.assertEqual(evidence['initial_safety_retirement']['root_worker_pid'], 152845)

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

    def test_macro_shm_mode_is_explicit_and_overrides_inheritance_in_both_arms(self):
        for mode in (False, True):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                with patch.dict(os.environ, JUSTRUST_CARGO_RESIDENT_MACRO_SHM='0' if mode else '1'):
                    report, calls, _ = self.capture(Path(directory), macro_shm=mode)
                self.assertIs(report['provenance']['macro_shm'], mode)
                self.assertEqual(len(calls), 40)
                self.assertEqual({env['JUSTRUST_CARGO_RESIDENT_MACRO_SHM']
                                  for _, _, _, env in calls}, {'1' if mode else '0'})
                self.assertEqual(set(suite.validate(report)), {'check', 'build'})
        for value in (None, 0, 1, '1'):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, 'Macro SHM'):
                suite.capture(None, None, None, None, macro_shm=value)

    def test_macro_spins_canonical_flags_and_shm_requirement(self):
        for value in ('0', '1', '256'):
            self.assertEqual(suite.parse_macro_spins(value), int(value))
        for value in ('-1', '257', '+1', '01', ' 1', '1 ', '1.0', '', '١'):
            with self.subTest(value=value), self.assertRaises(suite.argparse.ArgumentTypeError):
                suite.parse_macro_spins(value)
        for value in (None, True, False, -1, 257, 1.0, '1'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                suite.capture(None, None, None, None, macro_shm=True, macro_spins=value)
        with self.assertRaisesRegex(ValueError, 'require macro SHM'):
            suite.capture(None, None, None, None, macro_spins=1)

    def test_macro_spins_capture_and_provenance_are_strict(self):
        for count in (0, 256):
            with self.subTest(count=count), tempfile.TemporaryDirectory() as directory:
                with patch.dict(os.environ, JUSTRUST_CARGO_RESIDENT_MACRO_SPINS='17'):
                    report, calls, _ = self.capture(Path(directory), macro_shm=True, macro_spins=count)
                self.assertEqual(report['provenance']['macro_spins'], count)
                self.assertEqual({env['JUSTRUST_CARGO_RESIDENT_MACRO_SPINS'] for _, _, _, env in calls}, {str(count)})
                self.assertEqual(len(calls), 40)
                if count:
                    bad = copy.deepcopy(report)
                    bad['provenance']['macro_shm'] = False
                    with self.assertRaisesRegex(ValueError, 'require macro SHM'):
                        suite.validate(bad)
                for value in (None, True, -1, 257, '0', 1 if count == 0 else 0):
                    bad = copy.deepcopy(report)
                    if value is None:
                        bad['provenance'].pop('macro_spins')
                    else:
                        bad['provenance']['macro_spins'] = value
                    with self.subTest(value=value), self.assertRaises(ValueError):
                        suite.validate(bad)
                for mode in ('off', 'on'):
                    for value in (None, '00', '+0', '257', 0):
                        bad = copy.deepcopy(report)
                        env = bad['repetitions'][0]['arms'][mode]['observations'][0]['cargo']['invocation']['environment']
                        if value is None:
                            env.pop('JUSTRUST_CARGO_RESIDENT_MACRO_SPINS')
                        else:
                            env['JUSTRUST_CARGO_RESIDENT_MACRO_SPINS'] = value
                        with self.subTest(mode=mode, value=value), self.assertRaises(ValueError):
                            suite.validate(bad)

    def test_macro_spins_compare_requires_independent_exact_authorization(self):
        with tempfile.TemporaryDirectory() as directory:
            before, _, _ = self.capture(Path(directory), macro_shm=True)
            after = copy.deepcopy(before)
            after['provenance']['macro_spins'] = 32
            for pair in after['repetitions']:
                for arm in pair['arms'].values():
                    for row in arm['observations']:
                        row['cargo']['invocation']['environment']['JUSTRUST_CARGO_RESIDENT_MACRO_SPINS'] = '32'
            with self.assertRaisesRegex(ValueError, 'Provenance mismatch'):
                suite.compare(before, after)
            with self.assertRaisesRegex(ValueError, 'Provenance mismatch'):
                suite.compare(before, after, expected_before_macro_shm=True, expected_after_macro_shm=True)
            approval = dict(expected_before_macro_spins=0, expected_after_macro_spins=32)
            self.assertEqual(set(suite.compare(before, after, **approval)), {'check', 'build'})
            for overrides in (dict(expected_before_macro_spins=None), dict(expected_after_macro_spins=None),
                              dict(expected_before_macro_spins=False), dict(expected_after_macro_spins='32'),
                              dict(expected_before_macro_spins=1), dict(expected_after_macro_spins=33)):
                with self.subTest(overrides=overrides), self.assertRaises(ValueError):
                    suite.compare(before, after, **(approval | overrides))
            bad = copy.deepcopy(after)
            bad['provenance']['attestation']['compiler']['sha256'] = 'c' * 64
            bad['provenance']['expected']['compiler_sha256'] = 'c' * 64
            with self.assertRaisesRegex(ValueError, 'Provenance mismatch'):
                suite.compare(before, bad, **approval)

    def test_macro_shm_provenance_and_environment_tampering_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            report, _, _ = self.capture(Path(directory))
            for value in (None, 0, 1, '0', True):
                bad = copy.deepcopy(report)
                if value is None:
                    bad['provenance'].pop('macro_shm')
                else:
                    bad['provenance']['macro_shm'] = value
                with self.subTest(value=value), self.assertRaises(ValueError):
                    suite.validate(bad)
            for mode in ('off', 'on'):
                for value in (None, '1', True):
                    bad = copy.deepcopy(report)
                    env = bad['repetitions'][0]['arms'][mode]['observations'][0]['cargo']['invocation']['environment']
                    if value is None:
                        env.pop('JUSTRUST_CARGO_RESIDENT_MACRO_SHM')
                    else:
                        env['JUSTRUST_CARGO_RESIDENT_MACRO_SHM'] = value
                    with self.subTest(mode=mode, value=value), self.assertRaisesRegex(ValueError, 'environment mismatch'):
                        suite.validate(bad)

    def test_macro_shm_compare_requires_exact_explicit_modes_only(self):
        with tempfile.TemporaryDirectory() as directory:
            before, _, _ = self.capture(Path(directory))
            after = copy.deepcopy(before)
            after['provenance']['macro_shm'] = True
            for pair in after['repetitions']:
                for arm in pair['arms'].values():
                    for row in arm['observations']:
                        row['cargo']['invocation']['environment']['JUSTRUST_CARGO_RESIDENT_MACRO_SHM'] = '1'
            with self.assertRaisesRegex(ValueError, 'Provenance mismatch'):
                suite.compare(before, after)
            approval = dict(expected_before_macro_shm=False, expected_after_macro_shm=True)
            self.assertEqual(set(suite.compare(before, after, **approval)), {'check', 'build'})
            for overrides in (dict(expected_after_macro_shm=None), dict(expected_before_macro_shm=None),
                              dict(expected_before_macro_shm=True), dict(expected_after_macro_shm=False),
                              dict(expected_before_macro_shm=0), dict(expected_after_macro_shm=1)):
                with self.subTest(overrides=overrides), self.assertRaisesRegex(ValueError, 'macro SHM mode mismatch'):
                    suite.compare(before, after, **(approval | overrides))
            for name in ('compiler', 'driver', 'cargo', 'client'):
                bad = copy.deepcopy(after)
                bad['provenance']['attestation'][name]['sha256'] = 'c' * 64
                bad['provenance']['expected'][name + '_sha256'] = 'c' * 64
                with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'Provenance mismatch'):
                    suite.compare(before, bad, **approval)
            bad = copy.deepcopy(after)
            for pair in bad['repetitions']:
                for arm in pair['arms'].values():
                    for row in arm['observations']:
                        row['cargo']['invocation']['environment']['RUSTC'] = '/unauthorized/compiler'
            with self.assertRaisesRegex(ValueError, 'environment mismatch'):
                suite.compare(before, bad, **approval)
            changed_compiler = copy.deepcopy(after)
            hashes = {}
            for name, digest in (('compiler', 'a' * 64), ('driver', 'b' * 64)):
                changed_compiler['provenance']['attestation'][name].update(sha256=digest, path='/new/' + name)
                changed_compiler['provenance']['expected'][name + '_sha256'] = digest
                hashes['expected_before_' + name + '_sha256'] = before['provenance']['expected'][name + '_sha256']
                hashes['expected_after_' + name + '_sha256'] = digest
            for pair in changed_compiler['repetitions']:
                for arm in pair['arms'].values():
                    for row in arm['observations']:
                        row['cargo']['invocation']['environment']['RUSTC'] = '/new/compiler'
            self.assertEqual(set(suite.compare(before, changed_compiler, **approval, **hashes)), {'check', 'build'})
            partial = dict(hashes)
            partial.pop('expected_after_driver_sha256')
            with self.assertRaisesRegex(ValueError, 'hash mismatch'):
                suite.compare(before, changed_compiler, **approval, **partial)
            with self.assertRaisesRegex(ValueError, 'Provenance mismatch'):
                suite.compare(before, changed_compiler, **hashes)

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
