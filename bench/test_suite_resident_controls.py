#!/usr/bin/env python3
"""Mocked native resident suite routing checks, without builds or network."""
import json
import unittest
from unittest.mock import patch

import suite


class ResidentControls(unittest.TestCase):
    VALUES = {
        "JUSTRUST_CARGO_RESIDENT": "serial",
        "JUSTRUST_CARGO_RESIDENT_OWNED": "1",
        "JUSTRUST_CARGO_RESIDENT_EXTERNAL": "1",
        "JUSTRUST_CARGO_RESIDENT_MACROS": "1",
        "JUSTRUST_CARGO_RESIDENT_MACRO_SHM": "1",
        "JUSTRUST_CARGO_RESIDENT_MACRO_SPINS": "32",
        "JUSTRUST_RUSTC_RESIDENT_FRONTEND": "0",
    }
    PATHS = {
        "JUSTRUST_CARGO_RESIDENT_DIR": "/private/worker/session",
        "JUSTRUST_CARGO_RESIDENT_TRACE": "/private/traces/resident.jsonl",
    }
    SECRETS = {
        "CACHE_PUBLISH_TOKEN": "private-token",
        "AWS_SECRET_ACCESS_KEY": "private-key",
        "JUSTRUST_API_KEY": "private-api-key",
        "JUSTRUST_RUSTC_RESIDENT_FD": "91",
        "JUSTRUST_REMOTE": "force",
        "JUSTRUST_RUN_ID": "ambient-run-id",
    }

    def test_absent_controls_do_not_enable_resident_stack(self):
        with patch.dict(suite.os.environ, {}, clear=True):
            self.assertEqual(suite.synthetic_stack_controls(), {})
            self.assertEqual(suite.optimization_env(), {})

    def test_exact_explicit_native_controls_only(self):
        requested = {**self.VALUES, **self.PATHS}
        with patch.dict(suite.os.environ, {**requested, **self.SECRETS}, clear=True):
            self.assertEqual(suite.synthetic_stack_controls(), requested)

    def test_public_provenance_retains_values_not_private_paths_or_credentials(self):
        with patch.dict(suite.os.environ,
                        {**self.VALUES, **self.PATHS, **self.SECRETS}, clear=True):
            public = suite.optimization_env()
        self.assertEqual(public, {
            **self.VALUES,
            **{name + "_DECLARED": "1" for name in self.PATHS},
        })
        serialized = json.dumps(public)
        for value in (*self.PATHS.values(), *self.SECRETS.values()):
            self.assertNotIn(value, serialized)

    def test_explicit_disabled_values_and_empty_declared_paths_are_not_defaulted(self):
        disabled = {name: "0" for name in self.VALUES}
        paths = {name: "" for name in self.PATHS}
        with patch.dict(suite.os.environ, {**disabled, **paths}, clear=True):
            self.assertEqual(suite.synthetic_stack_controls(), {**disabled, **paths})
            self.assertEqual(suite.optimization_env(), {
                **disabled, **{name + "_DECLARED": "1" for name in paths},
            })

    def test_unsupported_shorthand_is_rejected_not_silently_ignored(self):
        for name in suite.UNSUPPORTED_RESIDENT_CONTROLS:
            with self.subTest(name=name), patch.dict(suite.os.environ, {name: "1"}, clear=True):
                with self.assertRaisesRegex(ValueError, "unsupported resident control"):
                    suite.synthetic_stack_controls()

    def test_unsafe_owned_frontend_experiments_are_rejected(self):
        for suffix in ("TYPED", "DECODERS", "LEXER", "AST", "VALUES", "FRONTEND"):
            name = "JUSTRUST_RUSTC_RESIDENT_" + suffix
            with self.subTest(name=name), patch.dict(
                    suite.os.environ, {"JUSTRUST_CARGO_RESIDENT_OWNED": "1", name: "1"},
                    clear=True):
                with self.assertRaisesRegex(ValueError, "unsupported owned resident experiment"):
                    suite.synthetic_stack_controls()

    def test_frontend_explicit_control_is_preserved_outside_owned_mode(self):
        requested = {"JUSTRUST_RUSTC_RESIDENT_FRONTEND": "1"}
        with patch.dict(suite.os.environ, requested, clear=True):
            self.assertEqual(suite.synthetic_stack_controls(), requested)
            self.assertEqual(suite.optimization_env(), requested)


if __name__ == "__main__":
    unittest.main()
