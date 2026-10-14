#!/usr/bin/env python3
"""Opt-in acceptance against actual rebuilt jr-cargo and installed justrust.

JR_GRAPH_CARGO and JR_GRAPH_RUSTC must name installable binaries. This test
runs local because its subject is a newly built local native producer, not the
currently deployed fleet. Normal unittest discovery skips it without those pins.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


@unittest.skipUnless(os.environ.get('JR_GRAPH_CARGO') and os.environ.get('JR_GRAPH_RUSTC'),
                     'requires actual native Cargo and rustc binaries')
class NativeGraphAcceptance(unittest.TestCase):
    def test_capture_neutrality_failure_and_unwritable_sink(self):
        scratch = os.environ.get('JCODE_SCRATCH_DIR')
        with tempfile.TemporaryDirectory(prefix='jr-graph-acceptance-', dir=scratch) as tmp:
            root = Path(tmp)
            env = os.environ.copy()
            env.update(JUSTRUST_TOOLCHAIN='system',
                       JUSTRUST_REAL_CARGO=str(Path(env['JR_GRAPH_CARGO']).resolve()),
                       RUSTC=str(Path(env['JR_GRAPH_RUSTC']).resolve()),
                       CARGO_TARGET_DIR=str(root / 'target'),
                       JUSTRUST_LOCALCACHE='0', JUSTRUST_BUILDD='0',
                       JUSTRUST_EARLY='0')
            env.pop('JUSTRUST_EXECUTION_GRAPH_DIR', None)
            exe = env.get('JR_GRAPH_JUSTRUST', shutil.which('justrust'))
            self.assertIsNotNone(exe)

            def write(path, text):
                target = root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(text)

            write('Cargo.toml', '[workspace]\nmembers=["app","leaf"]\nresolver="2"\n')
            write('app/Cargo.toml', '[package]\nname="graph-app"\nversion="0.1.0"\nedition="2024"\n'
                  '[dependencies]\ngraph-leaf={path="../leaf"}\n')
            write('leaf/Cargo.toml', '[package]\nname="graph-leaf"\nversion="0.1.0"\nedition="2024"\n')
            write('leaf/src/lib.rs', 'pub fn answer() -> u32 { 42 }\n')
            write('app/src/lib.rs', 'pub fn answer() -> u32 { graph_leaf::answer() }\n')
            source = 'fn main() { assert!(option_env!("JUSTRUST_EXECUTION_GRAPH_DIR").is_none()); assert_eq!(graph_app::answer(),42); }\n'
            write('app/src/main.rs', source)
            write('app/build.rs', 'fn main() { assert!(std::env::var_os("JUSTRUST_EXECUTION_GRAPH_DIR").is_none()); println!("cargo:rerun-if-env-changed=JUSTRUST_EXECUTION_GRAPH_DIR"); println!("cargo:rerun-if-changed=build.rs"); }\n')

            def command(args, capture=None, success=True):
                child_env = env.copy()
                if capture is not None:
                    child_env['JUSTRUST_EXECUTION_GRAPH_DIR'] = str(capture)
                p = subprocess.run([exe, *args], cwd=root, env=child_env,
                                   capture_output=True, text=True, timeout=180)
                output = p.stdout + p.stderr
                if success:
                    self.assertEqual(p.returncode, 0, output)
                else:
                    self.assertNotEqual(p.returncode, 0, output)
                return p, output

            def inspect(capture):
                p, _ = command(['graph', str(capture), '--json'])
                receipts = json.loads(p.stdout)
                self.assertTrue(receipts)
                self.assertTrue(all(r['structural_complete'] for r in receipts), p.stdout)
                self.assertTrue(all(not r['full_pipeline_complete'] for r in receipts))
                self.assertTrue(all(r['cpu_work_ns'] is None and r['execution_span_ns'] is None
                                    for r in receipts))
                raw = [json.loads(p.read_text()) for p in capture.glob('*.json')]
                scheduler = [r for r in raw if r.get('producer') == 'cargo-scheduler']
                manifests = list(capture.glob('*.manifest.json'))
                self.assertTrue(manifests, 'updated native owner manifest is required')
                for manifest_path in manifests:
                    manifest = json.loads(manifest_path.read_text())
                    envelopes = [(p, json.loads(p.read_text()))
                                 for p in capture.glob('*.envelope.json')]
                    selected = [(p, e) for p, e in envelopes
                                if e['run_id'] == manifest['run_id']]
                    self.assertTrue(selected)
                    # Reconcile one explicitly scoped owner inventory, not all
                    # unrelated native invocations sharing the capture sink.
                    with tempfile.TemporaryDirectory(prefix='owner-reconcile-', dir=root) as subset:
                        subset = Path(subset)
                        copied_manifest = subset / manifest_path.name
                        shutil.copyfile(manifest_path, copied_manifest)
                        for journal_path in capture.glob('owner-' + manifest['authority_root'] + '.*.json'):
                            shutil.copyfile(journal_path, subset / journal_path.name)
                        for envelope_path, envelope in selected:
                            shutil.copyfile(envelope_path, subset / envelope_path.name)
                            receipt_path = capture / (envelope['capture_id'] + '.json')
                            shutil.copyfile(receipt_path, subset / receipt_path.name)
                        p, _ = command(['graph', str(subset), '--manifest', str(copied_manifest), '--json'])
                        coverage = json.loads(p.stdout)
                        self.assertTrue(coverage['scheduler_entry_complete'], p.stdout)
                        self.assertFalse(coverage['full_pipeline_complete'])
                        self.assertEqual(coverage['authority_root'], manifest['authority_root'])
                        self.assertFalse(coverage['outcome_agreement_verified'])
                        declaration = next(subset.glob('*.declaration.json'))
                        saved_declaration = declaration.read_bytes()
                        declaration.unlink()
                        _, missing_journal_output = command(
                            ['graph', str(subset), '--manifest', str(copied_manifest), '--json'],
                            success=False)
                        self.assertIn('missing independent initial owner declaration', missing_journal_output)
                        declaration.write_bytes(saved_declaration)
                        saved_manifest = copied_manifest.read_bytes()
                        contradictory = json.loads(saved_manifest)
                        contradictory['owners'][0]['outcome'] = 'failed'
                        contradictory['owners'][0]['tickets'][0]['lifecycle'] = 'not_started'
                        contradictory['owners'][0]['tickets'][0]['execute_entered'] = False
                        copied_manifest.write_text(json.dumps(contradictory))
                        _, conflict_output = command(
                            ['graph', str(subset), '--manifest', str(copied_manifest), '--json'],
                            success=False)
                        self.assertIn('conflicts with final lifecycle', conflict_output)
                        copied_manifest.write_bytes(saved_manifest)
                        # A different declared authority is excluded explicitly,
                        # never silently promoted to directory/run completeness.
                        other = json.loads(saved_declaration)
                        other['run_id'] = other['authority_root'] = 'other-pending-owner'
                        other['owners'][0]['invocation_id'] = 'other-pending-owner'
                        other_path = subset / 'other.declaration.json'
                        other_path.write_text(json.dumps(other))
                        p, _ = command(['graph', str(subset), '--manifest', str(copied_manifest), '--json'])
                        scoped = json.loads(p.stdout)
                        self.assertTrue(scoped['scheduler_entry_complete'])
                        self.assertIn('other-pending-owner', scoped['excluded_authority_roots'])
                        other_path.unlink()
                        # Expected ticket remains present after deleting both
                        # its envelope and receipt. Missing work is not success.
                        envelope_path, envelope = selected[0]
                        (subset / envelope_path.name).unlink()
                        (subset / (envelope['capture_id'] + '.json')).unlink()
                        p, _ = command(['graph', str(subset), '--manifest', str(copied_manifest), '--json'])
                        missing = json.loads(p.stdout)
                        self.assertFalse(missing['scheduler_entry_complete'], p.stdout)
                        self.assertTrue(any('missing expected receipt' in issue
                                            for issue in missing['issues']))
                return scheduler

            _, output = command(['build', '--local'])
            binary = root / 'target/debug/graph-app'
            before = hashlib.sha256(binary.read_bytes()).hexdigest()
            capture = root / 'fresh-capture'
            _, output = command(['build', '--local'], capture)
            self.assertNotIn('Dirty ', output)
            self.assertIn('0 compiled', output)
            self.assertEqual(before, hashlib.sha256(binary.read_bytes()).hexdigest())
            receipts = inspect(capture)
            self.assertTrue(all(n['outcome'] == 'fresh' for r in receipts for n in r['nodes']))
            self.assertEqual(subprocess.run([binary], cwd=root, env=env).returncode, 0)
            command(['build', '--local'])

            # A fresh target exercises native starts and metadata pipelining,
            # rather than testing only no-op/fresh scheduler outcomes.
            env['CARGO_TARGET_DIR'] = str(root / 'compiled-target')
            compiled = root / 'compiled-capture'
            command(['build', '--local'], compiled)
            receipts = inspect(compiled)
            self.assertTrue(any(n['outcome'] == 'success' for r in receipts for n in r['nodes']))
            self.assertTrue(any(e['artifact'] == 'rmeta' for r in receipts for e in r['edges']))

            write('leaf/src/lib.rs', 'compile_error!("intentional graph failure");\n')
            failed = root / 'failed-capture'
            _, output = command(['build', '--local'], failed, success=False)
            self.assertIn('intentional graph failure', output)
            receipts = inspect(failed)
            self.assertTrue(any(n['outcome'] == 'failure' for r in receipts for n in r['nodes']))
            self.assertTrue(any(n['outcome'] == 'not_started' for r in receipts for n in r['nodes']))

            write('leaf/src/lib.rs', 'pub fn answer() -> u32 { 42 }\n')
            blocked = root / 'not-a-directory'
            blocked.write_text('existing private data')
            command(['build', '--local'], blocked)
            self.assertEqual(blocked.read_text(), 'existing private data')
            print('native graph acceptance: fresh reuse, binary identity, compiled metadata edges, failure closure and fail-open sink passed')


if __name__ == '__main__':
    unittest.main()
