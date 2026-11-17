"""Strict pinned login-event ownership overlay and real-producer controls."""
from pathlib import Path
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import suite

PATCH = 'bench/patches/jcode-login-event-fixture-v1.patch'
BUS = 'crates/jcode-base/src/bus.rs'
TEST = 'crates/jcode-tui/src/tui/app/tests/state_model_poke_03.rs'
NAME = 'test_tui_cerebras_paste_key_lifecycle_has_no_degraded_success_messages'
ANCHOR = '    while bus_rx.try_recv().is_ok() {}\n'
# Invoke the actual unrelated production login producer, on an unowned thread.
# Its deterministic no-runtime error is the same unscoped LoginCompleted variant
# as the cancelled DNS error in run 44032964275 attempt 2, without network timing.
UNRELATED = '''
    std::thread::spawn(|| {
        let mut unrelated = create_test_app();
        unrelated.start_login_provider(
            crate::provider_catalog::resolve_login_provider("grok-build")
                .expect("Grok Build login provider"),
        );
    }).join().unwrap();
'''
OWNED = '''
    rt.block_on(async {
        tokio::spawn(async {
            crate::bus::Bus::global().publish(crate::bus::BusEvent::LoginCompleted(
                crate::bus::LoginCompleted {
                    provider: "Cerebras".to_string(), success: false,
                    message: "owned lifecycle failure control".to_string(),
                },
            ));
        }).await.unwrap();
    });
'''


def original(source):
    repo, revision = suite.SNAPSHOTS['desktop']['repos']['jcode']
    return subprocess.check_output(['git', '-C', str(repo), 'show', f'{revision}:{source}'], text=True)


def corrected(root):
    for source in (BUS, TEST):
        destination = root / source
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(original(source))
        suite.verify_fixture_input(destination, suite.SNAPSHOTS['desktop']['fixture_input_sha256']['jcode/' + source])
    subprocess.run(['git', 'apply', '--check', str(suite.ROOT / PATCH)], cwd=root, check=True)
    subprocess.run(['git', 'apply', str(suite.ROOT / PATCH)], cwd=root, check=True)
    return {source: (root / source).read_text() for source in (BUS, TEST)}


def copy_acceptance_snapshot(snapshot, diagnostics):
    """Preserve pinned shallow Git objects/refs, excluding only build outputs."""
    shutil.copytree(snapshot, diagnostics, ignore=shutil.ignore_patterns('target'))


class LoginEventFixture(unittest.TestCase):
    def test_diagnostic_snapshot_preserves_exact_git_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'source'
            target = Path(directory) / 'diagnostics'
            for name in ('jcode', 'jcode-desktop'):
                for rel, content in (('.git/HEAD', 'ref: refs/heads/bench-pinned\n'),
                                     ('.git/refs/heads/bench-pinned', 'pinned-object-id\n'),
                                     ('.git/objects/pinned', 'exact commit bytes'),
                                     ('src/lib.rs', '// exact source\n'),
                                     ('target/build-output', 'excluded')):
                    path = source / name / rel
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(content)
            copy_acceptance_snapshot(source, target)
            for path in source.rglob('*'):
                if path.is_file() and 'target' not in path.relative_to(source).parts:
                    self.assertEqual(path.read_bytes(), (target / path.relative_to(source)).read_bytes())
            self.assertFalse((target / 'jcode/target').exists())
            self.assertFalse((target / 'jcode-desktop/target').exists())

    def test_preserves_every_assertion_and_entire_lifecycle(self):
        with tempfile.TemporaryDirectory() as directory:
            after = corrected(Path(directory))
        before = original(TEST)
        start = before.index('fn ' + NAME)
        # Event consumption, all assertions, counts, rendering and late-failure
        # checks are byte-identical. Only fixture setup changes before subscribe.
        boundary = '    let mut bus_rx = crate::bus::Bus::global().subscribe();'
        self.assertEqual(before[start:].split(boundary, 1)[1], after[TEST][start:].split(boundary, 1)[1])
        expected = before.replace(
            'fn ' + NAME + '() {\n',
            'fn ' + NAME + '() {\n'
            '    // Own the complete event stream, including failures, on this test and its\n'
            '    // runtime workers. Other apps\' delayed logins belong to their own streams.\n'
            '    let bus: &\'static std::sync::Arc<crate::bus::Bus> =\n'
            '        Box::leak(Box::new(crate::bus::Bus::new_isolated_for_tests()));\n'
            '    let bus: &\'static crate::bus::Bus = bus.as_ref();\n'
            '    let _bus_guard = crate::bus::install_test_global_bus(bus);\n', 1)
        runtime_start = expected.index('    let rt = tokio::runtime::Runtime::new().unwrap();', start)
        old_runtime = '    let rt = tokio::runtime::Runtime::new().unwrap();'
        new_runtime = '''    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .on_thread_start(move || {
            // Worker TLS dies with its thread. No guard resources are retained.
            std::mem::forget(crate::bus::install_test_global_bus(bus));
        })
        .build()
        .unwrap();'''
        expected = expected[:runtime_start] + expected[runtime_start:].replace(old_runtime, new_runtime, 1)
        self.assertEqual(expected, after[TEST])
        self.assertEqual(before[:start], after[TEST][:start])
        # Outside the selected fixture no runtime or test changes are allowed.
        tail = before.index('\n#[test]', start)
        self.assertTrue(after[TEST].endswith(before[tail:]))
        self.assertEqual(after[TEST].count('install_test_global_bus'), 2)
        self.assertIn('on_thread_start', after[TEST])
        self.assertIn('PhantomData<std::rc::Rc<()>>', after[BUS])
        self.assertIn('slot.set(self.previous)', after[BUS])
        hook = '''        #[cfg(any(test, feature = "test-support"))]
        if let Some(bus) = TEST_GLOBAL_BUS.with(|slot| slot.get()) {
            return bus;
        }
'''
        production_anchor = 'use crate::message::ToolCall;'
        self.assertEqual(after[BUS].split(production_anchor, 1)[1].replace(hook, '', 1),
                         original(BUS).split(production_anchor, 1)[1])
        self.assertEqual(after[BUS].count(hook), 1)
        self.assertNotIn('LoginCompleted', after[BUS][:after[BUS].index('use crate::message')])

    def test_hashes_fail_closed_and_identity_changes(self):
        spec = suite.SNAPSHOTS['desktop']
        self.assertIn(('jcode', PATCH), spec['patches'])
        digest = spec['fixture_patch_sha256'][PATCH]
        self.assertEqual(hashlib.sha256((suite.ROOT / PATCH).read_bytes()).hexdigest(), digest)
        identity = suite.snapshot_fingerprint('desktop')
        with patch.dict(suite.SNAPSHOTS, {'desktop': dict(spec, patches=[p for p in spec['patches'] if p[1] != PATCH])}):
            self.assertNotEqual(identity, suite.snapshot_fingerprint('desktop'))
        with tempfile.TemporaryDirectory() as directory:
            bad = Path(directory) / 'bad'
            bad.write_text('unexpected bytes')
            with self.assertRaisesRegex(ValueError, 'corrupt or unversioned fixture patch'):
                suite.verify_fixture_patch(bad, digest)
            for source in (BUS, TEST):
                with self.assertRaisesRegex(ValueError, 'unexpected pinned fixture input SHA256'):
                    suite.verify_fixture_input(bad, spec['fixture_input_sha256']['jcode/' + source])


def acceptance(directory, full=False):
    import runtime_context
    directory = directory.resolve()
    directory.mkdir(parents=True, exist_ok=True)
    os.environ[runtime_context.SELECTOR] = runtime_context.POLICY
    suite.WORK = directory / 'work'
    snapshot = suite.prepare('desktop')
    diagnostics = directory / 'diagnostics'
    if diagnostics.exists():
        raise ValueError(f'diagnostic destination already exists: {diagnostics}')
    # Exact pinned Git metadata is part of the workload contract. Repository
    # discovery/version fixtures must not see an empty synthetic repository.
    copy_acceptance_snapshot(snapshot, diagnostics)
    repo = diagnostics / 'jcode'
    env = os.environ.copy()
    env['JUSTRUST_TOOLCHAIN'] = 'system'
    env['RUSTUP_TOOLCHAIN'] = '1.99.0'
    if env.get('RUST_TEST_THREADS') or env.get('JUSTRUST_TEST_PARALLEL') == '0':
        raise ValueError('fixture acceptance requires normal test parallelism')
    env.setdefault('CARGO_TARGET_DIR', str(directory / 'target'))
    corrected_test = (repo / TEST).read_text()
    corrected_bus = (repo / BUS).read_text()

    def run(arguments, expected=0, failure=None, cwd=repo):
        with runtime_context.runtime_context(env, cwd) as child:
            result = subprocess.run(
                ['justrust', 'test', '--local=controlled pinned event fixture acceptance requires exact disposable binary', *arguments],
                cwd=cwd, env=child, capture_output=True, text=True)
        print(result.stdout, end='', flush=True)
        print(result.stderr, end='', file=sys.stderr, flush=True)
        if result.returncode != expected:
            raise RuntimeError(f'expected exit {expected}, got {result.returncode}')
        output = result.stdout + result.stderr
        if expected == 0 and 'passed when rerun alone by name' in output:
            raise RuntimeError('acceptance requires clean normal-parallel tests, not single-test retries')
        if failure and failure not in output:
            raise RuntimeError(f'original assertion not reached: {failure}')
        if arguments[-1] == NAME and expected == 0 and '1 passed' not in output:
            raise RuntimeError('corrected lifecycle fixture was not executed')

    for variant, control, expected in [('before', UNRELATED, 101), ('after', UNRELATED, 0), ('after', OWNED, 101)]:
        print('CONTROL', variant, 'unrelated' if control == UNRELATED else 'owned-worker-failure', flush=True)
        text = original(TEST) if variant == 'before' else corrected_test
        start = text.index('fn ' + NAME)
        index = text.index(ANCHOR, start) + len(ANCHOR)
        (repo / TEST).write_text(text[:index] + control + text[index:])
        (repo / BUS).write_text(original(BUS) if variant == 'before' else corrected_bus)
        failure = None
        if expected:
            failure = 'unexpected failed login event: LoginCompleted { provider: "grok-build"' if control == UNRELATED else 'unexpected failed login event: LoginCompleted { provider: "Cerebras", success: false, message: "owned lifecycle failure control"'
        run(['-p', 'jcode-tui', '--lib', NAME], expected, failure)
    # Acceptance uses exactly the measured overlay with all diagnostics removed.
    (repo / TEST).write_text(corrected_test)
    (repo / BUS).write_text(corrected_bus)
    if full:
        for arguments in (['-p', 'jcode-tui', '--lib'], ['-p', 'jcode-tui', '--lib'], ['--workspace'], ['--workspace']):
            print('FULL PINNED FIXTURE ACCEPTANCE', arguments, flush=True)
            run(arguments)
        for _ in range(2):
            print('FULL PINNED DESKTOP WORKSPACE ACCEPTANCE', flush=True)
            run(['--workspace'], cwd=diagnostics / 'jcode-desktop')


if __name__ == '__main__':
    if len(sys.argv) in (3, 4) and sys.argv[1] == '--accept':
        if len(sys.argv) == 4 and sys.argv[3] != '--full':
            raise ValueError('only --full is supported')
        acceptance(Path(sys.argv[2]), full=len(sys.argv) == 4)
    else:
        unittest.main()
