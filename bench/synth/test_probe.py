"""No-build regression tests for fail-closed live-location probing."""
import contextlib
import json
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import suite
engine, _ = suite._synth()
gen = engine.gen


class ProbeTests(unittest.TestCase):
    def setUp(self):
        self.ctx = contextlib.ExitStack()
        self.addCleanup(self.ctx.close)
        self.root = Path(self.ctx.enter_context(tempfile.TemporaryDirectory()))
        self.source = self.root / "src.rs"
        self.original = "fn example() {\n}\n"
        self.source.write_text(self.original)
        self.pkg = gen.Pkg("fixture", self.root, self.root / "Cargo.toml",
                           [{"kind": ["lib"], "src_path": str(self.source)}], True)
        self.ws = gen.Workspace("fixture", "fixture", self.root, self.root,
                                {"fixture": self.pkg}, ["fixture"], ["fixture"])
        for name, value in (("WORK", self.root / "work"),
                            ("JOURNAL", self.root / "journal")):
            self.ctx.enter_context(mock.patch.object(gen, name, value))
        self.ctx.enter_context(mock.patch.object(gen, "_locked"))
        self.ctx.enter_context(mock.patch.object(gen, "home_workspace", return_value=self.ws))
        self.ctx.enter_context(mock.patch.object(gen.suite, "snapshot_dir", return_value=Path("snapshot")))

    def diagnostic(self, text, code=None, file=None, level="error"):
        return {"reason": "compiler-message", "message": {
            "level": level, "message": text, "rendered": text,
            "code": {"code": code} if code else None,
            "spans": [{"file_name": str(file), "text": []}] if file else []}}

    def run_probe(self, diagnostics=(), stderr="", returncode=101):
        result = SimpleNamespace(stdout="\n".join(map(json.dumps, diagnostics)),
                                 stderr=stderr, returncode=returncode)
        with mock.patch.object(gen.subprocess, "run", return_value=result):
            try:
                return gen._probe_run(self.ws, self.pkg, {0: (self.source, 0)}, False)
            finally:
                self.assertEqual(self.source.read_text(), self.original)
                self.assertFalse(gen.JOURNAL.exists())

    def test_expected_mismatch_is_live(self):
        self.assertEqual(self.run_probe([self.diagnostic('jrprobe_0', 'E0308', self.source)]),
                         ({0}, set(), set()))

    def test_dependency_error_is_fatal_even_with_live_probe(self):
        diagnostics = [self.diagnostic('jrprobe_0', 'E0308', self.source),
                       self.diagnostic('unsupported compiler feature', 'E0658', self.root / 'dep.rs')]
        with self.assertRaisesRegex(SystemExit, 'unsupported compiler feature'):
            self.run_probe(diagnostics)

    def test_unrelated_type_error_in_edited_file_is_fatal(self):
        with self.assertRaisesRegex(SystemExit, 'unrelated errors'):
            self.run_probe([self.diagnostic('missing item', 'E0425', self.source)])

    def test_spanless_error_is_fatal(self):
        with self.assertRaisesRegex(SystemExit, 'cannot find crate'):
            self.run_probe([self.diagnostic('cannot find crate', 'E0463')])

    def test_shell_lost_binary_environment_is_not_dead_code(self):
        diagnostics = [self.diagnostic('jrprobe_0', 'E0308', self.source),
                       self.diagnostic('environment variable `CARGO_BIN_EXE_fixture-bin` '
                                       'not defined at compile time',
                                       file=self.root / 'tests/version.rs')]
        with self.assertRaisesRegex(SystemExit, 'CARGO_BIN_EXE_fixture-bin'):
            self.run_probe(diagnostics)

    def test_internal_compiler_error_mentioning_probe_is_fatal(self):
        with self.assertRaisesRegex(SystemExit, 'compiler crashed at jrprobe_0'):
            self.run_probe([self.diagnostic('compiler crashed at jrprobe_0',
                                           level='error: internal compiler error')])

    def test_cargo_only_failure_is_fatal(self):
        with self.assertRaisesRegex(SystemExit, 'could not compile dependency'):
            self.run_probe(stderr='error: could not compile dependency')

    def test_local_parse_error_can_retry(self):
        self.assertEqual(self.run_probe([self.diagnostic('expected expression', file=self.source)]),
                         (set(), set(), {self.source.resolve()}))

    def test_probe_parse_error_is_broken_not_live(self):
        self.assertEqual(self.run_probe([self.diagnostic('unexpected jrprobe_0', file=self.source)]),
                         (set(), {0}, set()))

    def test_abort_summary_does_not_hide_real_error(self):
        with self.assertRaisesRegex(SystemExit, 'failed'):
            self.run_probe([self.diagnostic('aborting due to 1 previous error')])

    def test_no_live_locations_are_not_cached(self):
        with mock.patch.object(gen, '_probe_run', return_value=(set(), set(), set())):
            with self.assertRaisesRegex(SystemExit, 'no live locations'):
                gen.probe(self.ws, 'fixture')
        self.assertFalse((gen.WORK / 'probe/snapshot/fixture.json').exists())

    def test_declaration_only_zero_candidates_skip_compiler_and_cache(self):
        self.source.write_text('pub struct DeclarationOnly;\n')
        with mock.patch.object(gen, '_probe_run') as run:
            self.assertEqual(gen.probe(self.ws, 'fixture'), {})
        run.assert_not_called()
        self.assertFalse((gen.WORK / 'probe/snapshot/fixture.json').exists())

    def test_empty_cache_does_not_hide_new_candidates(self):
        cache = gen.WORK / 'probe/snapshot/fixture.json'
        cache.parent.mkdir(parents=True)
        cache.write_text('{}')
        self.source.write_text('pub struct DeclarationOnly;\n')
        with mock.patch.object(gen, '_probe_run') as run:
            self.assertEqual(gen.probe(self.ws, 'fixture'), {})
        run.assert_not_called()
        self.source.write_text(self.original)
        with mock.patch.object(gen, '_probe_run', return_value=(set(), set(), set())):
            with self.assertRaisesRegex(SystemExit, 'no live locations among 1 candidates'):
                gen.probe(self.ws, 'fixture')

    def test_stock_env_pins_compiler_and_rustdoc(self):
        with mock.patch.dict(gen.os.environ, {'RUSTC': '/bad/rustc',
                                              'RUSTDOC': '/bad/rustdoc',
                                              'RUSTC_WRAPPER': '/bad/wrapper'}):
            with mock.patch.object(gen.Path, 'exists', return_value=True):
                env = gen.stock_env()
        self.assertEqual(env['RUSTC'], '/usr/bin/rustc')
        self.assertEqual(env['RUSTDOC'], '/usr/bin/rustdoc')
        self.assertNotIn('RUSTC_WRAPPER', env)

    def test_empty_cache_is_reprobed(self):
        cache = gen.WORK / 'probe/snapshot/fixture.json'
        cache.parent.mkdir(parents=True)
        cache.write_text('{}')
        with mock.patch.object(gen, '_probe_run', return_value=({0}, set(), set())) as run:
            result = gen.probe(self.ws, 'fixture')
        self.assertEqual(run.call_count, 2)
        self.assertEqual(result, {'src.rs': {'body': [0], 'test': []}})
        self.assertEqual(json.loads(cache.read_text()), result)

    def test_retry_exhaustion_is_not_cached(self):
        with mock.patch.object(gen, '_probe_run', return_value=(set(), {0}, set())):
            with self.assertRaisesRegex(SystemExit, 'did not converge'):
                gen.probe(self.ws, 'fixture')
        self.assertFalse((gen.WORK / 'probe/snapshot/fixture.json').exists())


if __name__ == '__main__':
    unittest.main()
