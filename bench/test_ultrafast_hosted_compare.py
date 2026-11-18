"""Synthetic Python-only orchestration and oracle tests. Never launches Rust tools."""
import argparse
import json
import os
from pathlib import Path
import re
import tempfile
import unittest
from unittest.mock import patch

import ultrafast_hosted_compare as compare


def output(marker, failing=False, full=False):
    tests = [(name, 'FAILED' if failing and name == compare.shared.EDIT_TEST else 'ok')
             for name in compare.shared.NAMES]
    if full:
        tests += [(f'other::test_{n:03d}', 'ok') for n in range(143)] + [('other::ignored', 'ignored')]
    lines = [f'test {name} ... {status}' for name, status in tests]
    lines += [marker]
    if failing:
        lines += ['JR_RUNTIME_FAILURE']
    lines += [f'test result: {"FAILED" if failing else "ok"}. '
              f'{147 if full else 3 if failing else 4} passed; {int(failing)} failed; '
              f'{int(full)} ignored; 0 measured; {0 if full else 144} filtered out']
    return '\n'.join(lines) + '\n'


class HostedTests(unittest.TestCase):
    def setUp(self):
        # Short private output roots also exercise the socket-path guard.
        self.tmp = tempfile.TemporaryDirectory(prefix='uhc-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        source = self.root / 's'
        (source / 'src').mkdir(parents=True)
        self.original = b'fn shares_overlapping_time() {\n}\n'
        (source / 'src/summary.rs').write_bytes(self.original)
        engine = self.root / 'jr-engine'
        engine.write_bytes(b'\x7fELFmock')
        engine.chmod(0o700)
        self.args = argparse.Namespace(source=source, engine=engine,
            toolchain=self.root / compare.shared.TOOLCHAIN_NAME,
            output=self.root / 'o', samples=5, timeout=1,
            host_label='mock-dedicated-host', execute_host_workloads=True,
            skip_full_library=False)
        self.calls = []
        self.fault = None
        self.identity_side_effect = None

    def fake_run(self, cmd, cwd, env, prefix, timeout):
        rep, stack, phase = prefix.name.split('-', 2)
        self.calls.append(dict(rep=int(rep), stack=stack, phase=phase, cmd=cmd,
                               cwd=cwd, env=dict(env), contents=(cwd / 'src/summary.rs').read_bytes()))
        status = 0
        if phase in ('version', 'version-after'):
            text = {'upstream': compare.UPSTREAM_VERSION + '\n',
                    'stable': 'justrust 0.1.0-alpha.37\n',
                    'engine': 'jr-engine experimental 0.1 (Blaze-derived origin abc; build resident-v1-' +
                              'a' * 64 + ')\n',
                    'stock': 'cargo 1.93.0-nightly\n'}[stack]
        elif phase == 'rustc-version':
            text = 'rustc 1.93.0-nightly (mock 2026-10-26)\nrelease: 1.93.0-nightly\n'
        elif phase.startswith('status-'):
            text = 'daemon: pid 123, running\n  worker: pid 124, project, log /mock\n'
        elif phase in ('stop', 'cleanup'):
            text = 'stopped\n'
        elif phase == 'type-error':
            text, status = 'error[E0308]: JR_RUNTIME_TYPE_ERROR\n', 101
        else:
            marker = re.search(rb'JR_RUNTIME_EXECUTED_\d+', self.calls[-1]['contents']).group().decode()
            text = output(marker, failing=phase == 'assert-failure', full=phase == 'full-library')
            status = 101 if phase == 'assert-failure' else 0
        if self.fault:
            altered = self.fault(stack, phase, int(rep), text, status)
            if isinstance(altered, BaseException):
                raise altered
            if altered is not None:
                text, status = altered
        prefix.with_suffix('.stdout').write_text(text)
        prefix.with_suffix('.stderr').write_text('mock raw stderr\n')
        return dict(status=status, error=None, timed_out=False,
                    elapsed_seconds=0.01 if stack == 'engine' else 0.1, command=cmd)

    def run_harness(self):
        with patch.object(compare.shared, 'pinned_source'), \
                patch.object(compare, 'identities', return_value={'mock': 'stable'},
                             side_effect=self.identity_side_effect), \
                patch.object(compare.runtime, 'run_process', side_effect=self.fake_run), \
                patch.object(compare.tier, 'memory_snapshot', return_value=[{'pid': 123, 'pss_kib': 99}]), \
                patch('subprocess.Popen', side_effect=AssertionError('no native invocation allowed')):
            return compare.execute(self.args)

    def records(self):
        return json.loads((self.args.output / 'records.json').read_text())

    def enable_upstream(self):
        self.args.upstream = self.root / 'official'
        self.args.upstream.write_bytes(b'\x7fELFofficial')
        self.args.upstream.chmod(0o700)

    def enable_stable(self):
        self.args.stable_client = self.root / 'published'
        self.args.stable_client.write_bytes(b'\x7fELFpublished')
        self.args.stable_client.chmod(0o700)
        self.args.stable_version = 'justrust 0.1.0-alpha.37'

    def test_stable_four_arms_private_normal_client(self):
        self.enable_stable()
        self.enable_upstream()
        with patch.dict(os.environ, {'JUSTRUST_API_KEY': 'mock-secret',
                                    'AWS_SECRET_ACCESS_KEY': 'mock-secret',
                                    'SSH_AUTH_SOCK': '/mock-agent'}):
            self.assertEqual(self.run_harness(), 0)
        self.assertEqual(self.metadata()['stacks'], ['stock', 'engine', 'upstream', 'stable'])
        self.assertTrue(self.metadata()['full_library_parity'])
        self.assertEqual(len(self.metadata()['restored_copies']), 20)
        self.assertIn('NOT network-inclusive', self.metadata()['stable_environment_policy'])
        for rep in range(5):
            calls = [c for c in self.calls if c['rep'] == rep and c['phase'] == 'cold']
            self.assertEqual(len({c['cwd'] for c in calls}), 4)
            self.assertEqual(len({c['contents'] for c in calls}), 1)
            for key in ('CARGO_HOME', 'CARGO_TARGET_DIR', 'RUSTUP_HOME',
                        'XDG_CONFIG_HOME', 'XDG_CACHE_HOME'):
                self.assertEqual(len({c['env'][key] for c in calls}), 4)
        stable = [c for c in self.calls if c['stack'] == 'stable']
        self.assertEqual(sum(c['phase'] == 'version-after' for c in stable), 5)
        for call in stable:
            env = call['env']
            self.assertEqual(env['JUSTRUST_REMOTE'], 'off')
            self.assertEqual(env['JUSTRUST_HOME'], str(Path(env['HOME']) / '.justrust'))
            self.assertEqual(env['JUSTRUST_NO_UPDATE'], '1')
            self.assertEqual(env['JUSTRUST_AUTO_UPDATE'], '0')
            self.assertEqual(env['JUSTRUST_NO_TELEMETRY'], '1')
            self.assertEqual(env['JUSTRUST_LOCAL_REASON'], compare.STABLE_LOCAL_REASON)
            self.assertTrue(Path(env['HOME']).is_relative_to(call['cwd'].parent))
            self.assertEqual(env['RUSTC'], str(self.args.toolchain / 'bin/rustc'))
            self.assertEqual(env['RUSTUP_TOOLCHAIN'], str(self.args.toolchain))
            self.assertEqual(env['XDG_STATE_HOME'], str(Path(env['HOME']) / '.local/state'))
            for key in ('JUSTRUST_API_KEY', 'AWS_SECRET_ACCESS_KEY', 'SSH_AUTH_SOCK'):
                self.assertNotIn(key, env)
            self.assertNotIn(call['phase'], ('stop', 'cleanup'))
            self.assertNotIn('native', call['cmd'])
            if call['phase'] in compare.PHASES + ('full-library',):
                self.assertEqual(call['cmd'][:3], [str(self.args.stable_client), 'test',
                    '--local=controlled same-machine hosted comparison'])

    def test_stable_identity_hash_and_version(self):
        self.enable_stable()
        with patch.object(compare.shared, 'toolchain_identity', return_value={}):
            identity = compare.identities(self.args)['stable']
        self.assertEqual(identity['sha256'], compare.runtime.base.digest(self.args.stable_client))
        self.assertEqual(identity['required_version'], self.args.stable_version)
        self.identity_side_effect = [{'stable': identity}, {'stable': dict(identity, sha256='changed')}]
        self.assertEqual(self.run_harness(), 1)
        self.assertFalse(self.metadata()['unchanged'])

    def test_stable_version_requires_explicit_exact_pin(self):
        self.enable_stable()
        for version in (None, '', 'justrust\nother', 'justrust\rother', ' justrust'):
            self.args.stable_version = version
            with self.assertRaisesRegex(ValueError, 'stable version'):
                self.run_harness()
        self.assertFalse(self.calls)

    def test_stable_non_elf_rejected(self):
        self.enable_stable()
        self.args.stable_client.write_bytes(b'#!/bin/sh\n')
        with self.assertRaisesRegex(ValueError, 'stable_client.*ELF'):
            self.run_harness()
        self.assertFalse(self.calls)

    def test_stable_cli_parses_explicit_pin(self):
        self.enable_stable()
        argv = ['compare', '--engine', str(self.args.engine), '--source', str(self.args.source),
                '--output', str(self.args.output), '--toolchain', str(self.args.toolchain),
                '--host-label', 'mock', '--stable-client', str(self.args.stable_client),
                '--stable-version', self.args.stable_version]
        with patch('sys.argv', argv), patch.object(compare, 'execute', return_value=0) as execute:
            with self.assertRaises(SystemExit) as exit_result:
                compare.main()
        self.assertEqual(exit_result.exception.code, 0)
        parsed = execute.call_args.args[0]
        self.assertEqual(parsed.stable_client, self.args.stable_client)
        self.assertEqual(parsed.stable_version, self.args.stable_version)

    def test_stable_exception_restores_without_daemon_cleanup(self):
        self.enable_stable()
        self.fault = lambda s, p, r, t, x: RuntimeError('mock interrupted stable') \
            if s == 'stable' and p == 'edit' else None
        with self.assertRaisesRegex(RuntimeError, 'mock interrupted stable'):
            self.run_harness()
        self.assertTrue(all(self.metadata()['restored_copies'].values()))
        self.assertFalse(any(c['stack'] == 'stable' and c['phase'] in ('stop', 'cleanup')
                             for c in self.calls))

    def test_stable_failures_never_summarized_and_restore_without_stop(self):
        self.enable_stable()
        faults = [('version', lambda t: 'unexpected banner\n' + t),
                  ('version', lambda t: t.replace('alpha.37', 'alpha.38')),
                  ('version', lambda t: t.replace('alpha.37', 'alpha.37 extra')),
                  ('version-after', lambda t: t.replace('alpha.37', 'alpha.38')),
                  ('full-library', lambda t: t.replace('other::test_000', 'other::wrong')),
                  ('edit', lambda t: t.replace('4 passed', '0 passed'))]
        for index, (phase, alter) in enumerate(faults):
            with self.subTest(phase=phase, index=index):
                self.args.output = self.root / f'o{index}'
                self.calls = []
                self.fault = lambda s, p, r, t, x: (alter(t), x) if s == 'stable' and p == phase else None
                self.assertEqual(self.run_harness(), 1)
                self.assertTrue(all(self.metadata()['restored_copies'].values()))
                self.assertFalse(any(c['stack'] == 'stable' and c['phase'] in ('stop', 'cleanup')
                                     for c in self.calls))
                summary = json.loads((self.args.output / 'summary.json').read_text())
                self.assertEqual(summary['medians_seconds'], {})

    def test_optional_upstream_three_matched_isolated_arms(self):
        self.enable_upstream()
        self.assertEqual(self.run_harness(), 0)
        self.assertEqual(self.metadata()['stacks'], ['stock', 'engine', 'upstream'])
        self.assertIn('not an untouched no-op', self.metadata()['warm_unchanged_policy'])
        self.assertEqual(len(self.metadata()['restored_copies']), 15)
        for rep in range(5):
            for phase in compare.PHASES + (('full-library',) if rep == 0 else ()):
                calls = [c for c in self.calls if c['rep'] == rep and c['phase'] == phase]
                self.assertEqual([c['stack'] for c in calls],
                                 compare.runtime.order(['stock', 'engine', 'upstream'], rep))
                self.assertEqual(len({c['contents'] for c in calls}), 1)
                for key in ('BLAZE_HOME', 'BLAZE_CACHE', 'RA_JIT_HOME', 'RA_JIT_CACHE',
                            'CARGO_HOME', 'CARGO_TARGET_DIR', 'RUSTUP_HOME'):
                    self.assertEqual(len({c['env'][key] for c in calls}), 3)
                upstream = next(c for c in calls if c['stack'] == 'upstream')
                self.assertEqual(upstream['cmd'][:3], [str(self.args.upstream), 'cargo', 'test'])
        upstream_calls = [c for c in self.calls if c['stack'] == 'upstream']
        for phase in ('version', 'version-after', 'stop', 'cleanup'):
            self.assertEqual(sum(c['phase'] == phase for c in upstream_calls), 5)
        self.assertEqual(sum(r['phase'] in compare.PHASES for r in self.records()), 105)
        summary = json.loads((self.args.output / 'summary.json').read_text())
        self.assertEqual(set(summary['medians_seconds']), {'stock', 'engine', 'upstream'})

    def test_upstream_wrong_version_fails_closed(self):
        self.enable_upstream()
        self.fault = lambda s, p, r, t, x: ('blaze 0.20261027.0 (wrong)\n', x) \
            if s == 'upstream' and p == 'version' else None
        self.assertEqual(self.run_harness(), 1)

    def test_upstream_changed_version_fails_closed(self):
        self.enable_upstream()
        self.fault = lambda s, p, r, t, x: ('blaze changed\n', x) \
            if s == 'upstream' and p == 'version-after' else None
        self.assertEqual(self.run_harness(), 1)

    def test_explicit_canary_direct_cli_and_toolchains(self):
        self.enable_upstream()
        self.args.upstream_version = 'blaze 0.20261105.4 (e930296 2026-11-05)'
        self.args.upstream_cli = 'direct'
        version_output = self.args.upstream_version + '\n' + str(self.args.upstream) + '\n'
        self.fault = lambda s, p, r, t, x: (version_output, x) \
            if s == 'upstream' and p in ('version', 'version-after') else None
        self.assertEqual(self.run_harness(), 0)
        for call in self.calls:
            if call['stack'] == 'upstream':
                self.assertEqual(call['env']['BLAZE_TOOLCHAINS'], str(self.args.toolchain.parent))
                for name in ('BLAZE_BACKEND', 'BLAZE_OPT', 'RA_JIT_BACKEND', 'RA_JIT_OPT'):
                    self.assertNotIn(name, call['env'])
                if call['phase'] in compare.PHASES + ('full-library',):
                    self.assertEqual(call['cmd'][:2], [str(self.args.upstream), 'test'])
                    self.assertNotIn('cargo', call['cmd'])
            else:
                self.assertNotIn('BLAZE_TOOLCHAINS', call['env'])
        with patch.object(compare.shared, 'toolchain_identity', return_value={}):
            identity = compare.identities(self.args)['upstream']
        self.assertEqual(identity['required_version'], self.args.upstream_version)
        self.assertEqual(identity['cli'], 'direct')
        self.assertIn('native backend/optimization defaults',
                      self.metadata()['upstream_environment_policy'])

    def test_canary_not_accepted_by_stable_default(self):
        self.enable_upstream()
        self.fault = lambda s, p, r, t, x: ('blaze 0.20261105.4 (e930296 2026-11-05)\n/path\n', x) \
            if s == 'upstream' and p in ('version', 'version-after') else None
        self.assertEqual(self.run_harness(), 1)

    def test_upstream_version_must_be_exact_first_line(self):
        self.enable_upstream()
        self.fault = lambda s, p, r, t, x: ('unexpected banner\n' + t, x) \
            if s == 'upstream' and p in ('version', 'version-after') else None
        self.assertEqual(self.run_harness(), 1)

    def test_upstream_version_line_cannot_be_prefix(self):
        self.enable_upstream()
        self.fault = lambda s, p, r, t, x: (t.replace(compare.UPSTREAM_VERSION,
            compare.UPSTREAM_VERSION + ' unexpected'), x) \
            if s == 'upstream' and p in ('version', 'version-after') else None
        self.assertEqual(self.run_harness(), 1)

    def test_upstream_version_and_cli_option_guards(self):
        for version in ('', 'blaze\nother', 'blaze\rother'):
            self.args.upstream_version = version
            with self.assertRaisesRegex(ValueError, 'one exact nonempty line'):
                self.run_harness()
        self.args.upstream_version = compare.UPSTREAM_VERSION
        self.args.upstream_cli = 'unknown'
        with self.assertRaisesRegex(ValueError, 'cargo or direct'):
            self.run_harness()
        self.assertFalse(self.calls)

    def test_upstream_full_parity_mismatch_fails_closed(self):
        self.enable_upstream()
        self.fault = lambda s, p, r, t, x: (t.replace('other::test_000', 'other::wrong'), x) \
            if s == 'upstream' and p == 'full-library' else None
        self.assertEqual(self.run_harness(), 1)
        self.assertFalse(self.metadata()['full_library_parity'])

    def test_upstream_cleanup_failure_fails_closed(self):
        self.enable_upstream()
        self.fault = lambda s, p, r, t, x: (t, 1) \
            if s == 'upstream' and p == 'cleanup' else None
        self.assertEqual(self.run_harness(), 1)
        self.assertTrue(self.metadata()['cleanup_errors'])

    def test_upstream_wrapper_is_rejected_before_invocation(self):
        self.enable_upstream()
        self.args.upstream.write_bytes(b'#!/bin/sh\n')
        with self.assertRaisesRegex(ValueError, 'upstream.*ELF'):
            self.run_harness()
        self.assertFalse(self.calls)

    def test_upstream_identity_contains_exact_sha_and_expected_version(self):
        self.enable_upstream()
        with patch.object(compare.shared, 'toolchain_identity', return_value={}):
            identity = compare.identities(self.args)['upstream']
        self.assertEqual(identity['sha256'], compare.runtime.base.digest(self.args.upstream))
        self.assertEqual(identity['required_version'], 'blaze 0.20261027.0 (49ca152)')

    def test_upstream_sha_mutation_excludes_all_medians(self):
        self.enable_upstream()
        with patch.object(compare.shared, 'toolchain_identity', return_value={}):
            before = compare.identities(self.args)
            self.args.upstream.write_bytes(b'\x7fELFchanged')
            after = compare.identities(self.args)
        self.identity_side_effect = [before, after]
        self.assertEqual(self.run_harness(), 1)
        self.assertFalse(self.metadata()['unchanged'])
        self.assertEqual(json.loads((self.args.output / 'summary.json').read_text())['medians_seconds'], {})

    def test_upstream_interruption_restores_and_cleans_both_namespaces(self):
        self.enable_upstream()
        self.fault = lambda s, p, r, t, x: KeyboardInterrupt() \
            if s == 'upstream' and p == 'edit' else None
        with self.assertRaises(KeyboardInterrupt):
            self.run_harness()
        self.assertTrue(self.metadata()['unchanged'])
        self.assertEqual({c['stack'] for c in self.calls if c['phase'] == 'cleanup'},
                         {'engine', 'upstream'})

    def metadata(self):
        return json.loads((self.args.output / 'metadata.json').read_text())

    def test_complete_counterbalanced_contract_and_restoration(self):
        self.assertEqual(self.run_harness(), 0)
        metadata = self.metadata()
        self.assertEqual(metadata['status'], 'passed')
        self.assertTrue(metadata['full_library_parity'])
        self.assertTrue(metadata['unchanged'])
        self.assertEqual(len(metadata['restored_copies']), 10)
        self.assertEqual((self.args.source / 'src/summary.rs').read_bytes(), self.original)
        for rep in range(5):
            for phase in compare.PHASES:
                calls = [c for c in self.calls if c['rep'] == rep and c['phase'] == phase]
                self.assertEqual([c['stack'] for c in calls], compare.runtime.order(compare.STACKS, rep))
                self.assertEqual(calls[0]['contents'], calls[1]['contents'])
            warm = [c for c in self.calls if c['rep'] == rep and c['phase'] in ('cold', 'warm-unchanged')]
            self.assertTrue(all(c['contents'] == warm[0]['contents'] for c in warm))
        for call in self.calls:
            env = call['env']
            self.assertEqual(env['RA_JIT_REMOTE_PUSH_EVERY'], 'off')
            self.assertEqual(env['BLAZE_REMOTE'], 'off')
            self.assertEqual(env['RA_JIT_REMOTE'], 'off')
            self.assertEqual(env['RUSTC'], str(self.args.toolchain / 'bin/rustc'))
            if call['stack'] == 'stock' and call['phase'] in compare.PHASES:
                self.assertEqual(call['cmd'][0], str(self.args.toolchain / 'bin/cargo'))
                self.assertEqual(call['cmd'][1], 'test')
        rows = self.records()
        self.assertEqual(sum(r['phase'] in compare.PHASES for r in rows), 70)
        self.assertTrue(all(r['accepted'] for r in rows if not r['phase'].startswith('status-')))
        self.assertTrue(all('memory' in r for r in rows if r['phase'].startswith('status-')))
        summary = json.loads((self.args.output / 'summary.json').read_text())
        self.assertEqual(summary['medians_seconds']['stock']['edit'], 0.1)
        for row in rows:
            for raw in row['raw'].values():
                self.assertTrue((self.args.output / raw['file']).is_file())

    def test_stale_runtime_nonce_is_excluded(self):
        self.fault = lambda s, p, r, t, x: (re.sub(r'JR_RUNTIME_EXECUTED_\d+',
            'JR_RUNTIME_EXECUTED_0', t), x) if s == 'engine' and p == 'edit' else None
        self.assertEqual(self.run_harness(), 1)
        self.assertEqual(json.loads((self.args.output / 'summary.json').read_text())['medians_seconds'], {})
        self.assertTrue(self.metadata()['unchanged'])

    def test_exit_success_without_tests_is_not_accepted(self):
        self.fault = lambda s, p, r, t, x: ('all good\n', 0) if p == 'cold' else None
        self.assertEqual(self.run_harness(), 1)

    def test_error_status_without_diagnostic_is_not_accepted(self):
        self.fault = lambda s, p, r, t, x: ('failure\n', 101) if p == 'type-error' else None
        self.assertEqual(self.run_harness(), 1)

    def test_full_gate_mismatch_rejected(self):
        self.fault = lambda s, p, r, t, x: (t.replace('other::test_000', 'other::wrong'), x) \
            if p == 'full-library' and s == 'engine' else None
        self.assertEqual(self.run_harness(), 1)
        self.assertFalse(self.metadata()['full_library_parity'])

    def test_interruption_restores_bytes_and_stops_private_engine(self):
        self.fault = lambda s, p, r, t, x: KeyboardInterrupt() if p == 'edit' else None
        with self.assertRaises(KeyboardInterrupt):
            self.run_harness()
        metadata = self.metadata()
        self.assertTrue(metadata['unchanged'])
        self.assertEqual(metadata['status'], 'failed/incomplete')
        self.assertTrue(any(c['phase'] == 'cleanup' for c in self.calls))

    def test_full_gate_skipped_is_explicitly_incomplete(self):
        self.args.skip_full_library = True
        self.assertEqual(self.run_harness(), 1)
        self.assertFalse(any(c['phase'] == 'full-library' for c in self.calls))
        self.assertIn('explicitly skipped', self.metadata()['full_library_policy'])

    def test_acknowledgement_and_sample_count_guards(self):
        self.args.execute_host_workloads = False
        with self.assertRaisesRegex(ValueError, 'acknowledgement'):
            self.run_harness()
        self.assertFalse(self.calls)
        self.args.execute_host_workloads = True
        self.args.samples = 4
        with self.assertRaisesRegex(ValueError, 'at least five'):
            self.run_harness()
        self.assertFalse(self.calls)

    def test_duplicate_marker_and_wrong_test_names_rejected(self):
        row = dict(status=0, error=None, timed_out=False)
        marker = 'JR_RUNTIME_EXECUTED_7'
        text = output(marker)
        self.assertTrue(compare.accepted('edit', row, text, marker))
        self.assertFalse(compare.accepted('edit', row, text + marker, marker))
        self.assertFalse(compare.accepted('edit', row, text.replace('summary::tests::', 'unrelated::'), marker))
        self.assertFalse(compare.accepted('edit', row, text.replace('144 filtered', '143 filtered'), marker))

    def test_markers_are_valid_u64_and_recovery_fresh(self):
        nonce = (1 << 63)
        variants = compare.inputs(self.original, nonce)
        seen = []
        for phase in compare.PHASES + ('full-library',):
            if phase != 'type-error':
                marker = re.search(rb'JR_RUNTIME_EXECUTED_(\d+)', variants[phase]).group(1)
                self.assertLess(int(marker), 1 << 64)
                self.assertEqual(int(marker), compare.phase_nonce(nonce, phase))
                if phase != 'warm-unchanged':
                    seen.append(marker)
        self.assertEqual(len(seen), len(set(seen)))
        failing = variants['assert-failure']
        self.assertLess(failing.index(b'println!'), failing.index(b'JR_RUNTIME_FAILURE'))

    def test_unidentified_engine_version_is_not_certified(self):
        self.fault = lambda s, p, r, t, x: ('jr-engine experimental unidentified\n', x) \
            if p == 'version' and s == 'engine' else None
        self.assertEqual(self.run_harness(), 1)

    def test_both_missing_full_outputs_do_not_establish_parity(self):
        self.fault = lambda s, p, r, t, x: ('no tests\n', 0) if p == 'full-library' else None
        self.assertEqual(self.run_harness(), 1)
        self.assertFalse(self.metadata()['full_library_parity'])

    def test_identity_change_excludes_all_medians(self):
        self.identity_side_effect = [{'mock': 'before'}, {'mock': 'after'}]
        self.assertEqual(self.run_harness(), 1)
        self.assertFalse(self.metadata()['unchanged'])
        self.assertEqual(json.loads((self.args.output / 'summary.json').read_text())['medians_seconds'], {})

    def test_missing_identity_after_run_still_saves_failure_receipts(self):
        self.identity_side_effect = [{'mock': 'before'}, OSError('tool disappeared')]
        self.assertEqual(self.run_harness(), 1)
        self.assertIn('tool disappeared', self.metadata()['identity_error'])
        self.assertGreater(len(self.records()), 70)

    def test_nonzero_status_and_timeout_never_accepted_as_success(self):
        marker = 'JR_RUNTIME_EXECUTED_7'
        text = output(marker)
        for row in (dict(status=0, error=None, timed_out=True),
                    dict(status=-9, error=None, timed_out=False),
                    dict(status=0, error='failed spawn', timed_out=False)):
            self.assertFalse(compare.accepted('edit', row, text, marker))

    def test_environment_removes_remote_credentials_and_wrappers(self):
        with patch.dict(os.environ, {'RA_JIT_REMOTE_TOKEN': 'secret', 'BLAZE_REMOTE': 'https://invalid',
                                     'RUSTC_WRAPPER': '/bad-wrapper', 'CARGO_ENCODED_RUSTFLAGS': 'bad'}):
            env = compare.environment(self.root / 'env', 'engine', self.args)
        self.assertNotIn('RA_JIT_REMOTE_TOKEN', env)
        self.assertNotIn('RUSTC_WRAPPER', env)
        self.assertNotIn('CARGO_ENCODED_RUSTFLAGS', env)
        self.assertEqual(env['BLAZE_REMOTE'], 'off')
        self.assertNotEqual(env['JR_ENGINE_HOME'], env['HOME'])
        self.assertEqual(Path(env['JR_ENGINE_HOME']).stat().st_mode & 0o777, 0o700)


if __name__ == '__main__':
    unittest.main()
