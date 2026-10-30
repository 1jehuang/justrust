import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import resident_build_identity_accept as gate


class IdentityTests(unittest.TestCase):
    def test_cargo_repeated_directive_uses_final_source_identity(self):
        values = gate.directives('cargo:rustc-env=JR_ENGINE_BUILD_ID=resident-v1-unidentified\n'
                                 'cargo:rustc-env=JR_ENGINE_BUILD_ID=resident-v1-final\n'
                                 'cargo:rerun-if-env-changed=RA_JIT_GIT_SHA\n')
        self.assertEqual(values, {'JR_ENGINE_BUILD_ID': 'resident-v1-final'})

    def test_independent_oracle_is_length_delimited_and_salted(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name in ('Cargo.toml', 'Cargo.lock', '.cargo/config.toml', 'harness/toolchain'):
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b'a')
            fields = [b'justrust-resident-v1', gate.ORIGIN.encode()]
            for name in sorted(('Cargo.toml', 'Cargo.lock', '.cargo/config.toml', 'harness/toolchain')):
                fields.extend((name.encode(), b'a'))
            fields.extend((b'', b'identity-test-target', b'\0'))
            expected = hashlib.sha256(b''.join(len(f).to_bytes(8, 'little') + f for f in fields)).hexdigest()
            self.assertEqual(gate.expected_key(root), expected)
            self.assertNotEqual(gate.expected_key(root, debug=True), expected)
            self.assertNotEqual(gate.expected_key(root, features=('CARGO_FEATURE_LLVM',)), expected)

    def test_optimized_python_rejected_before_output_or_process(self):
        with mock.patch.object(gate.sys, 'flags', optimize=1), mock.patch.object(gate.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'assertions enabled'):
                gate.execute(None)
            run.assert_not_called()


if __name__ == '__main__':
    unittest.main()
