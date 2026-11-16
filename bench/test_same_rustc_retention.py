"""Regression tests for the isolated frozen-compiler experiment."""
import base64
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import same_rustc_retention as bench


def record(status=0, stdout=b'', stderr=b''):
    return dict(status=status, stdout_b64=base64.b64encode(stdout).decode(),
                stderr_b64=base64.b64encode(stderr).decode(), elapsed_seconds=0.1,
                peak_rss_kib=100)


class RetentionTests(unittest.TestCase):
    def test_only_retention_flags_and_cache_differ(self):
        root = Path('/private')
        off = list(map(str, bench.command('/rustc', root, root / 'off', 'off')))
        on = list(map(str, bench.command('/rustc', root, root / 'on', 'on')))
        differences = [(a, b) for a, b in zip(off, on) if a != b]
        self.assertEqual(len(differences), 3)
        self.assertIn('-Zcodegen-backend=llvm', off)
        self.assertIn('-Zincremental-verify-ich', off)
        for flag in bench.FLAGS:
            self.assertIn((flag + '=no', flag + '=yes'), differences)

    def test_revision_matrix(self):
        cases = bench.cases(32)
        self.assertEqual([c[0] for c in cases], ['warm', 'unchanged-forced-recompile',
                         'body-edit', 'revert', 'type-error', 'recovery'])
        self.assertEqual(cases[0][1], cases[1][1])
        self.assertEqual(cases[0][1], cases[-1][1])
        self.assertEqual(cases[0][2], '1095\n')
        self.assertIn('"wrong"', cases[4][1])
        self.assertIn('impl Dispatch', cases[0][1])
        self.assertEqual(cases[0][1].replace('body() -> u64 { 7 }', 'body() -> u64 { 8 }'), cases[2][1])
        self.assertEqual(cases[0][1].count('constant() -> u64 { 7 }'), 1)
        self.assertEqual(cases[2][1].count('constant() -> u64 { 7 }'), 1)

    def test_exact_diagnostic_equivalence(self):
        pair = {m: dict(compile=record(1, stderr=b'error\n')) for m in ('off', 'on')}
        bench.validate_pair(pair, None)
        pair['on']['compile']['stderr_b64'] = base64.b64encode(b'error \n').decode()
        with self.assertRaisesRegex(AssertionError, 'Exact'):
            bench.validate_pair(pair, None)

    def test_reject_ice_and_bad_runtime(self):
        pair = {m: dict(compile=record(101)) for m in ('off', 'on')}
        with self.assertRaisesRegex(AssertionError, 'ordinary'):
            bench.validate_pair(pair, None)
        pair = {m: dict(compile=record(), runtime=record(stdout=b'wrong')) for m in ('off', 'on')}
        with self.assertRaisesRegex(AssertionError, 'runtime'):
            bench.validate_pair(pair, 'correct\n')

    def test_wait4_status_output_and_rss(self):
        result = bench.run([sys.executable, '-c', 'import sys; print("hello"); sys.stderr.write("bad\\n"); sys.exit(1)'], Path('unused'))
        self.assertEqual(result['status'], 1)
        self.assertEqual(base64.b64decode(result['stdout_b64']), b'hello\n')
        self.assertEqual(base64.b64decode(result['stderr_b64']), b'bad\n')
        self.assertGreater(result['peak_rss_kib'], 0)

    def test_cache_sizes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'one').write_bytes(b'123')
            size = bench.cache_size(root)
            self.assertEqual(size['logical_bytes'], 3)
            self.assertEqual(size['files'], 1)
            self.assertGreaterEqual(size['allocated_bytes'], 3)

    def test_capture_rotates_and_preserves_failure_identity(self):
        def fake_run(command, rss_file=None):
            if len(command) == 1:
                return record(stdout=b'10\n')
            root = Path(command[1]).parent
            source = (root / 'fixture.rs').read_text()
            if '"wrong"' in source:
                return record(1, stderr=b'type error\n')
            (root / 'fixture').write_bytes(b'executable')
            return record()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(bench, 'identity', side_effect=[{'hash': 'a'}, {'hash': 'b'}]), \
                    patch.object(bench, 'run', side_effect=fake_run), \
                    patch.object(bench, 'cases', return_value=[('warm', 'fn main() {}', '10\n')]):
                with self.assertRaisesRegex(AssertionError, 'identity changed'):
                    bench.capture(Path('/rustc'), root, repetitions=2, items=1)
            receipt = json.loads((root / (root.name + '.json')).read_text())
            self.assertEqual(receipt['verdict'], 'failed')
            self.assertEqual([r['order'] for r in receipt['observations']], [['off', 'on'], ['on', 'off']])
            self.assertEqual(len(receipt['observations']), 2)

    def make_toolchain(self, root):
        sysroot = root / 'toolchain'
        target = sysroot / 'lib/rustlib/x86_64-test-linux-gnu/lib'
        target.mkdir(parents=True)
        compiler = sysroot / 'bin/rustc'
        compiler.parent.mkdir()
        compiler.write_bytes(b'compiler')
        for name in ('libstd-test.rlib', 'libcore-test.rlib'):
            (target / name).write_bytes(name.encode())
        (sysroot / 'lib/libLLVM-test.so').write_bytes(b'llvm')
        (sysroot / 'lib/librustc_driver-test.so').write_bytes(b'driver')
        return compiler, target

    def identity_query(self, compiler, target, command):
        if command[-1] == '-vV':
            return record(stdout=b'rustc test\nLLVM version: 23.1.3\n')
        if command[-1] == 'sysroot':
            return record(stdout=(str(compiler.parent.parent) + '\n').encode())
        if command[-1] == 'target-libdir':
            return record(stdout=(str(target) + '\n').encode())
        raise AssertionError('Unexpected identity command: ' + repr(command))

    def test_all_artifact_byte_mutations_reject_capture(self):
        for kind in ('compiler', 'driver', 'llvm', 'std', 'core'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as tmp:
                base = Path(tmp)
                compiler, target = self.make_toolchain(base)
                paths = dict(compiler=compiler, driver=compiler.parent.parent / 'lib/librustc_driver-test.so',
                             llvm=compiler.parent.parent / 'lib/libLLVM-test.so',
                             std=target / 'libstd-test.rlib', core=target / 'libcore-test.rlib')
                output = base / 'acceptance'
                output.mkdir()
                actual_identity = bench.identity
                identities = []

                def capture_identity(path):
                    if identities:
                        paths[kind].write_bytes(paths[kind].read_bytes() + b'-mutated')
                    result = actual_identity(path)
                    identities.append(result)
                    return result

                def fake_run(command, rss_file=None):
                    if len(command) > 1 and (command[-1] == '-vV' or command[1] == '--print'):
                        return self.identity_query(compiler, target, command)
                    if len(command) == 1:
                        return record(stdout=b'10\n')
                    (output / 'fixture').write_bytes(b'fixture')
                    return record()

                with patch.object(bench, 'identity', side_effect=capture_identity), \
                        patch.object(bench, 'run', side_effect=fake_run), \
                        patch.object(bench, 'cases', return_value=[('warm', 'fn main() {}', '10\n')]):
                    with self.assertRaisesRegex(AssertionError, 'identity changed'):
                        bench.capture(compiler, output, repetitions=2, items=1)
                receipt = json.loads((output / 'acceptance.json').read_text())
                self.assertEqual(receipt['schema'], 2)
                self.assertEqual(receipt['verdict'], 'failed')
                self.assertNotEqual(receipt['identity_before'], receipt['identity_after'])
                self.assertIn('sysroot_artifacts', receipt['identity_before'])

    def test_sysroot_inventory_add_remove_and_symlink_mutations(self):
        with tempfile.TemporaryDirectory() as tmp:
            compiler, target = self.make_toolchain(Path(tmp))
            sysroot = compiler.parent.parent
            initial = bench.sysroot_identity(sysroot, target)
            self.assertEqual(initial, bench.sysroot_identity(sysroot, target))
            added = target / 'new.rmeta'
            added.write_bytes(b'new')
            self.assertNotEqual(initial, bench.sysroot_identity(sysroot, target))
            added.unlink()
            self.assertEqual(initial, bench.sysroot_identity(sysroot, target))
            alias = sysroot / 'lib/libLLVM-alias.so'
            alias.symlink_to('libLLVM-test.so')
            linked = bench.sysroot_identity(sysroot, target)
            entry = next(e for e in linked['files'] if e['relative_path'].endswith('libLLVM-alias.so'))
            self.assertEqual(entry['symlink_target'], 'libLLVM-test.so')
            alias.unlink()
            alias.symlink_to(str(sysroot / 'lib/libLLVM-test.so'))
            self.assertNotEqual(linked, bench.sysroot_identity(sysroot, target))
            (target / 'libcore-test.rlib').unlink()
            with self.assertRaisesRegex(AssertionError, 'core rlib'):
                bench.sysroot_identity(sysroot, target)

    def test_sysroot_rejects_missing_llvm_and_unhashed_artifact_directories(self):
        with tempfile.TemporaryDirectory() as tmp:
            compiler, target = self.make_toolchain(Path(tmp))
            sysroot = compiler.parent.parent
            llvm = sysroot / 'lib/libLLVM-test.so'
            llvm.unlink()
            with self.assertRaisesRegex(AssertionError, 'LLVM library'):
                bench.sysroot_identity(sysroot, target)
            llvm.write_bytes(b'llvm')
            outside = Path(tmp) / 'outside'
            outside.mkdir()
            with self.assertRaisesRegex(AssertionError, 'outside'):
                bench.sysroot_identity(sysroot, outside)
            (sysroot / 'lib/other').symlink_to(outside, target_is_directory=True)
            with self.assertRaisesRegex(AssertionError, 'directory symlink'):
                bench.sysroot_identity(sysroot, target)

    def test_source_symlink_boundary_is_explicit_not_traversed(self):
        with tempfile.TemporaryDirectory() as tmp:
            compiler, target = self.make_toolchain(Path(tmp))
            sysroot = compiler.parent.parent
            source = Path(tmp) / 'source'
            source.mkdir()
            (source / 'code.rs').write_bytes(b'not a native input')
            link = sysroot / 'lib/rustlib/src/rust'
            link.parent.mkdir()
            link.symlink_to(source, target_is_directory=True)
            before = bench.sysroot_identity(sysroot, target)
            (source / 'code.rs').write_bytes(b'changed source, identical installed artifacts')
            self.assertEqual(before, bench.sysroot_identity(sysroot, target))
            self.assertEqual(before['directory_symlinks_not_traversed'],
                             [dict(relative_path='lib/rustlib/src/rust', target=str(source))])

    def test_sysroot_walk_errors_are_not_silently_skipped(self):
        with tempfile.TemporaryDirectory() as tmp:
            compiler, target = self.make_toolchain(Path(tmp))

            def denied_walk(path, followlinks, onerror):
                onerror(PermissionError('unreadable artifact directory'))

            with patch.object(bench.os, 'walk', side_effect=denied_walk):
                with self.assertRaisesRegex(PermissionError, 'unreadable'):
                    bench.sysroot_identity(compiler.parent.parent, target)

    def test_hash_rejects_file_mutation_during_read(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'artifact'
            path.write_bytes(b'before')
            actual_digest = bench.digest

            def mutate(p):
                path.write_bytes(b'after-and-longer')
                return actual_digest(p)

            with patch.object(bench, 'digest', side_effect=mutate):
                with self.assertRaisesRegex(AssertionError, 'changed while hashing'):
                    bench.artifact_identity(path)

    def test_compile_failure_receipt_survives(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(bench, 'identity', return_value={'hash': 'a'}), \
                    patch.object(bench, 'run', return_value=record(101, stderr=b'ICE')):
                with self.assertRaises(AssertionError):
                    bench.capture(Path('/rustc'), root, repetitions=2, items=1)
            receipt = json.loads((root / (root.name + '.json')).read_text())
            self.assertEqual(receipt['verdict'], 'failed')
            self.assertIn('Unexpected compile failure', receipt['error'])
            self.assertEqual(receipt['identity_before'], receipt['identity_after'])


if __name__ == '__main__':
    unittest.main()
