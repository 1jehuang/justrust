"""Offline Git, libtest witness, and pre-tag fail-closed fixtures."""
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

import exact_client_gate as gate

SPEC = importlib.util.spec_from_file_location('release_client', Path(__file__).with_name('release-client.py'))
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ExactGateTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name) / 'repo'
        self.root.mkdir()
        def git(*args):
            return subprocess.check_output(['git', *args], cwd=self.root, text=True).strip()
        git('init', '-q')
        self.member = Path('crates/graph/src/lib.rs')
        (self.root / self.member).parent.mkdir(parents=True)
        (self.root / self.member).write_text('$Format:%H$')
        (self.root / '.gitattributes').write_text('crates/graph export-ignore\n*.rs export-subst\n')
        git('add', '.')
        # Construct fixture commits without installing hooks, signing commits,
        # or overriding the user's Git identity in any repository.
        original = subprocess.check_output(['git', 'cat-file', '-p', 'HEAD'],
                                          cwd=Path(__file__).resolve().parents[1])
        authors = [s for s in original.split(b'\n\n', 1)[0].splitlines()
                   if s.startswith((b'author ', b'committer '))]
        def commit():
            payload = b'tree ' + git('write-tree').encode() + b'\n' + b'\n'.join(authors) + b'\n\nfixture\n'
            sha = subprocess.check_output(['git', 'hash-object', '-t', 'commit', '-w', '--stdin'],
                                          cwd=self.root, input=payload).strip().decode()
            git('update-ref', 'HEAD', sha)
        commit()
        self.sha = git('rev-parse', 'HEAD')
        (self.root / self.member).write_text('new revision')
        git('add', '.')
        commit()
        (self.root / self.member).write_text('dirty edit')
        (self.root / 'credential').write_text('not source')
        self.paths = []

    def invoke(self, cmd, *, cwd, **kwargs):
        self.paths.append(cwd)
        name = next((cwd / 'tests').glob('exact_*.rs')).stem
        self.assertIn(self.sha, name)
        self.assertFalse((cwd / '.git').exists())
        self.assertFalse((cwd / 'credential').exists())
        self.assertEqual((cwd / self.member).read_text(), '$Format:%H$')
        if cmd[1] == 'test':
            if self.mode == 'exit':
                raise subprocess.CalledProcessError(101, cmd)
            if self.mode in ('clean', 'changed', 'extra'):
                # Compile only the generated std-only witness offline, directly
                # with rustc. No hosted request, registry, Cargo build or tag.
                target = cwd / 'target'
                target.mkdir()
                exe = target / 'fixture'
                subprocess.run(['rustc', '--test', str(cwd / 'tests' / (name+'.rs')), '-o', exe], check=True)
                if self.mode == 'changed':
                    (cwd / self.member).write_text('wrong bytes')
                if self.mode == 'extra':
                    (cwd / 'build.rs').write_text('unexpected')
                result = subprocess.run([exe], cwd=cwd, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                result.check_returncode()
                self.output = result.stdout
            return mock.Mock(stdout=None)
        if cmd[1] == 'runs':
            row = dict(id='run-1', cwd=str(cwd), remote='local' if self.mode == 'local' else 'hosted',
                       exit=0, args=gate.ARGS)
            return mock.Mock(stdout=json.dumps(row))
        self.assertEqual(cmd[1:], ['log', 'run-1', '--grep', name])
        return mock.Mock(stdout=self.output if self.mode == 'clean' else 'test exact_other ... ok')

    def test_clean_and_closed_failures(self):
        for self.mode in ('clean', 'changed', 'extra', 'exit', 'local', 'wrong-source'):
            with self.subTest(mode=self.mode):
                if self.mode == 'clean':
                    self.assertEqual(gate.gate(self.root, self.sha, invoke=self.invoke), self.sha)
                else:
                    with self.assertRaises((ValueError, subprocess.CalledProcessError)):
                        gate.gate(self.root, self.sha, invoke=self.invoke)
                self.assertFalse(self.paths[-1].exists())

    def test_missing_committed_blob_never_invokes_client(self):
        oid = subprocess.check_output(['git', 'rev-parse', f'{self.sha}:{self.member}'],
                                      cwd=self.root, text=True).strip()
        (self.root / '.git/objects' / oid[:2] / oid[2:]).unlink()
        invoke = mock.Mock()
        with self.assertRaisesRegex(ValueError, 'missing'):
            gate.gate(self.root, self.sha, invoke=invoke)
        invoke.assert_not_called()

    def test_bad_batch_responses(self):
        oid = b'e69de29bb2d1d6434b8b29ae775ad8c2e48c5391'  # empty blob
        for response in (b'0'*40 + b' blob 0\n\n', oid + b' tree 0\n\n', oid + b' blob -1\n',
                         oid + b' blob 1\n', oid + b' blob 0\nx', oid + b' blob 1\nx\n'):
            proc = mock.Mock(stdin=io.BytesIO(), stdout=io.BytesIO(response))
            with self.subTest(response=response), tempfile.TemporaryDirectory() as d:
                with self.assertRaises(ValueError):
                    gate.copy_blob(proc, oid, Path(d) / 'output')

    def test_no_release_without_matched_source_and_success(self):
        args = ['--tag', 'v1.2.3', '--ref', self.sha, '--test-exact', '--execute']
        with mock.patch.object(release, 'ensure_tag') as tag, mock.patch.object(release, 'candidate', return_value=self.sha):
            for value in ['0'*40, subprocess.CalledProcessError(101, ['test'])]:
                with mock.patch.object(gate, 'gate', side_effect=[value]), self.assertRaises((ValueError, subprocess.CalledProcessError)):
                    release.main(args)
            with mock.patch.object(gate, 'gate', return_value=self.sha):
                release.main(args[:-1])
            tag.assert_not_called()
        with self.assertRaises(SystemExit):
            release.main(['--tag', 'v1.2.3', '--test-exact', '--ref', 'stable'])
