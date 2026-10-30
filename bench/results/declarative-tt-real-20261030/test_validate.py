#!/usr/bin/env python3
"""Fast mutation checks for the retained dp41 state contract, no native work."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("dp41_validate", ROOT / "validate.py")
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


class StateMutations(unittest.TestCase):
    def setUp(self):
        self.state = json.loads((ROOT / "state.json").read_text())

    def reject(self, change):
        changed = copy.deepcopy(self.state)
        change(changed)
        with self.assertRaises(AssertionError):
            audit.check_state(changed)

    def test_original(self):
        audit.check_state(self.state)

    def test_missing_workflow(self):
        self.reject(lambda s: s["rows"].pop())

    def test_source_not_restored(self):
        self.reject(lambda s: s["source_after"].update({"src/summary.rs": "0" * 64}))

    def test_tool_changed(self):
        self.reject(lambda s: s["identities_after"]["rustc_driver_0"].update(sha256="0" * 64))

    def test_edit_count_not_633(self):
        self.reject(lambda s: s["rows"][1]["profiles"][0]["markers"].update(jr_declarative_macro_eligible=633))

    def test_warm_edit_provider_not_zero(self):
        self.reject(lambda s: s["rows"][1]["profiles"][0]["markers"].update(jr_declarative_macro_cache_provider=0))

    def test_proc_cache_not_allowed(self):
        self.reject(lambda s: s["rows"][1]["environment"].update(JUSTRUST_CACHE_PROC_MACROS="1"))

    def test_process_failure(self):
        self.reject(lambda s: s["rows"][1].update(status=1))

    def test_edit_must_change_source(self):
        self.reject(lambda s: s["rows"][1].update(source_sha256=s["rows"][0]["source_sha256"]))


if __name__ == "__main__":
    unittest.main()
