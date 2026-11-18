"""Bootstrap contract tests using fake tools, never invoking a Rust compiler."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class ResidentBootstrapTests(unittest.TestCase):
    def test_required_reason_default_and_operator_override(self):
        source = Path(__file__).resolve().parents[1] / 'toolchain/resident'
        for override in (None, 'operator-host exact compiler bootstrap'):
            with self.subTest(override=override), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                engine = root / 'source'
                engine.mkdir()
                shutil.copyfile(source / 'build-jr.sh', engine / 'build-jr.sh')
                pins = engine / 'crates/ra-jit/src/cargo/toolchain/pins.txt'
                pins.parent.mkdir(parents=True)
                pins.write_text('commit exact-test-commit\n')
                for name in ('LICENSE-MIT', 'LICENSE-APACHE', 'JUSTRUST-SOURCE.md'):
                    (engine / name).write_text('fixture\n')
                toolchain = root / 'toolchain'
                (toolchain / 'bin').mkdir(parents=True)
                library = toolchain / 'lib/rustlib/src/rust/library'
                library.mkdir(parents=True)
                (library / 'Cargo.toml').touch()
                rustc = toolchain / 'bin/rustc'
                rustc.write_text('#!/bin/sh\nprintf "commit-hash: exact-test-commit\\n"\n')
                rustc.chmod(0o755)
                client = toolchain / 'bin/justrust'
                client.write_text('''#!/bin/sh
set -eu
: "${JUSTRUST_LOCAL_REASON:?concrete reason required}"
printf '%s' "$JUSTRUST_LOCAL_REASON" > "$CAPTURE_REASON"
[ "$JUSTRUST_REMOTE" = off ]
[ "$JUSTRUST_REAL_RUSTC" = "$RUSTC" ]
mkdir -p "$CARGO_TARGET_DIR/release"
printf '#!/bin/sh\\nprintf "jr-engine experimental fixture (build resident-v1-%064d)\\\\n"\\n' 0 > "$CARGO_TARGET_DIR/release/jr-engine"
chmod 755 "$CARGO_TARGET_DIR/release/jr-engine"
''')
                client.chmod(0o755)
                env = os.environ.copy()
                env.pop('JUSTRUST_LOCAL_REASON', None)
                env.update(PATH=str(toolchain / 'bin') + os.pathsep + env['PATH'],
                           CARGO_TARGET_DIR=str(root / 'target'),
                           CAPTURE_REASON=str(root / 'reason'))
                if override is not None:
                    env['JUSTRUST_LOCAL_REASON'] = override
                result = subprocess.run(['bash', str(engine / 'build-jr.sh'),
                                         str(toolchain), str(root / 'install')],
                                        env=env, text=True, capture_output=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                reason = (root / 'reason').read_text()
                if override:
                    self.assertEqual(reason, override)
                else:
                    self.assertIn('exact pinned native compiler', reason)
                self.assertTrue((root / 'install/jr-native').is_file())


if __name__ == '__main__':
    unittest.main()
