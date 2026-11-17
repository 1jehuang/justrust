#!/usr/bin/env python3
"""Pure Python protocol mocks. Never invokes or builds a native compiler."""

import copy
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import stable_resident_mir_arithmetic as arithmetic


def record(status=0, stdout=b"", stderr=b""):
    return dict(
        status=status,
        stdout_b64=arithmetic.shared.encoded(stdout),
        stderr_b64=arithmetic.shared.encoded(stderr),
        timed_out=False,
    )


class MockNative:
    """Artifact writing stand-in for the fresh process and native JSONL worker."""

    def __init__(self, fault=None):
        self.fault = fault
        self.index = 0
        self.counters = dict.fromkeys(arithmetic.leaf.COUNTERS, 0)
        self.calls = []
        self.worker = None

    def start(self, compiler, root, mode, timeout):
        self.root, self.mode = root, mode
        self.worker = SimpleNamespace(
            process=SimpleNamespace(pid=4242),
            command=[str(compiler), "--jr-resident-owned"],
            compile=self.compile,
            close=lambda: None,
        )
        for name in ("resident.stdout", "resident.stderr", "resident.jsonl"):
            (root / name).write_bytes(b"untouched raw " + name.encode())
        return self.worker

    def producer(self):
        case = arithmetic.cases()[self.index]
        expected = case[2]
        if expected is None:
            return record(1, stderr=b"exact type error\n")
        for name in ("fixture.ll", "fixture.mir", "fixture.o"):
            (self.root / name).write_bytes(b"original artifact " + name.encode())
        return record()

    def compile(self, argv):
        row = self.producer()
        kind = arithmetic.cases()[self.index][3]
        if self.mode == "leaf":
            if kind in ("same", "cross"):
                self.counters["mir_leaf_hits"] += 1
                self.counters["retained_entries"] = 2
            if self.fault == "no-cross-hit" and kind == "cross":
                self.counters["mir_leaf_hits"] -= 1
            if self.fault == "typeck-cross" and kind == "cross":
                self.counters["typeck_hits"] += 1
        elif self.fault == "disabled-counter":
            self.counters["mir_leaf_hits"] = 1
        row.update(
            pid=4243 if self.fault == "pid" else 4242,
            executed=True,
            owned=dict(retirement_reason=None),
            live=dict(self.counters),
        )
        if self.fault == "diagnostics":
            row["stderr_b64"] = arithmetic.shared.encoded(b"different\n")
        self.index += 1
        return row

    def run(self, argv, timeout, cwd, env):
        self.calls.append(list(map(str, argv)))
        if len(argv) == 1:
            # Native compile advances the revision before its consumer runs.
            index = self.index - 1 if self.last_engine == "resident" else self.index
            expected = arithmetic.cases()[index][2]
            output = arithmetic.expected_output(*expected)
            if (
                self.fault == "stale-output"
                and index == 4
                and self.last_engine == "resident"
            ):
                output = arithmetic.expected_output(7, 11)
            return record(stdout=output)
        if str(cwd / "consumer.c") in argv:
            if self.fault == "consumer-compile":
                return record(1, stderr=b"consumer compile failure\n")
            (cwd / "consumer").write_bytes(b"original executable")
            if self.fault == "consumer-diagnostics" and self.last_engine == "resident":
                return record(stderr=b"different consumer diagnostics\n")
            return record()
        self.last_engine = "fresh"
        return self.producer()

    def compile_with_engine(self, argv):
        self.last_engine = "resident"
        return self.compile(argv)


class ArithmeticTests(unittest.TestCase):
    def capture_mock(self, root, mode="leaf", fault=None):
        workspace, evidence = root / "workspace", root / mode
        workspace.mkdir()
        for name, data in arithmetic.sources().items():
            (workspace / name).write_bytes(data)
        native = MockNative(fault)
        original_start = native.start

        def start(*args):
            worker = original_start(*args)
            worker.compile = native.compile_with_engine
            return worker

        with patch.object(
            arithmetic.typed, "TypedResident", side_effect=start
        ), patch.object(arithmetic.shared, "run", side_effect=native.run):
            result = arithmetic.capture_mode(
                Path("/frozen/compiler"), workspace, evidence, mode, 1
            )
        return result, native

    def test_fixture_and_flags(self):
        files = arithmetic.sources()
        self.assertIn(b"#![no_core]", files["fixture.rs"])
        self.assertNotIn(b"#![no_std]", b"".join(files.values()))
        self.assertNotIn(b"extern crate", b"".join(files.values()))
        self.assertNotIn(b"use core", b"".join(files.values()))
        self.assertIn(b'pub extern "C" fn leaf_a', files["module_a.rs"])
        self.assertIn(b'pub extern "C" fn leaf_b', files["module_b.rs"])
        self.assertIn(b"impl Add for u32", files["fixture.rs"])
        self.assertIn(b"impl Copy for u32", files["fixture.rs"])
        self.assertIn(
            "--emit=obj=/work/fixture.o", arithmetic.arguments(Path("/work"))[-1]
        )
        consumer = arithmetic.consumer_source()
        self.assertIn(b"extern uint32_t leaf_a(uint32_t, uint32_t);", consumer)
        self.assertIn(rb'PRIu32 "\n"', consumer)
        self.assertNotIn(b"arithmetic_fixture", consumer)
        self.assertIn(b"x: u32, y: u32", files["module_a.rs"])
        self.assertIn("-Coverflow-checks=off", arithmetic.arguments(Path("/work")))
        cases = arithmetic.cases()
        self.assertEqual(cases[1][1]["module_a.rs"], cases[2][1]["module_a.rs"])
        self.assertNotEqual(cases[1][1]["module_b.rs"], cases[2][1]["module_b.rs"])
        self.assertEqual(cases[3][1]["module_b.rs"], cases[4][1]["module_b.rs"])
        self.assertNotEqual(cases[3][1]["module_a.rs"], cases[4][1]["module_a.rs"])

    def test_independent_wraparound_oracle(self):
        rows = arithmetic.expected_output(7, 9).splitlines()
        self.assertEqual(rows[0], b"21 65527")
        self.assertEqual(rows[4], b"21 65526")
        self.assertNotEqual(
            arithmetic.expected_output(7, 11), arithmetic.expected_output(13, 11)
        )
        self.assertNotEqual(
            arithmetic.expected_output(13, 11), arithmetic.expected_output(13, 15)
        )
        self.assertEqual(
            arithmetic.consumer_source().count(b"printf("), len(arithmetic.INPUTS)
        )

    def test_exact_consumer_rejects_wrong_output_status_timeout_stderr(self):
        good = record(stdout=arithmetic.expected_output(7, 9))
        arithmetic.validate_consumer(good, arithmetic.expected_output(7, 9))
        for key, value in [
            ("status", 1),
            ("timed_out", True),
            ("stdout_b64", arithmetic.shared.encoded(b"21\n")),
            ("stderr_b64", arithmetic.shared.encoded(b"warning")),
        ]:
            bad = copy.deepcopy(good)
            bad[key] = value
            with self.subTest(key=key), self.assertRaises(AssertionError):
                arithmetic.validate_consumer(bad, arithmetic.expected_output(7, 9))

    def test_mock_leaf_and_control_accept(self):
        for mode in ("leaf", "control"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                result, native = self.capture_mock(root, mode)
                self.assertEqual(result["verdict"], "passed")
                self.assertTrue(result["sources_restored"])
                self.assertEqual(len(result["observations"]), 9)
                self.assertEqual(sum(len(argv) == 1 for argv in native.calls), 14)
                self.assertEqual(
                    (root / mode / "resident.jsonl").read_bytes(),
                    b"untouched raw resident.jsonl",
                )
                self.assertEqual(
                    (root / "workspace" / "module_a.rs").read_bytes(),
                    arithmetic.sources()["module_a.rs"],
                )
                self.assertEqual(
                    json.loads((root / mode / "receipt.json").read_text())["verdict"],
                    "passed",
                )

    def test_strict_failures_retain_raw_evidence(self):
        for fault, mode in [
            ("no-cross-hit", "leaf"),
            ("typeck-cross", "leaf"),
            ("disabled-counter", "control"),
            ("pid", "leaf"),
            ("diagnostics", "leaf"),
            ("stale-output", "leaf"),
            ("consumer-compile", "leaf"),
            ("consumer-diagnostics", "leaf"),
        ]:
            with self.subTest(fault=fault), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                with self.assertRaises(AssertionError):
                    self.capture_mock(root, mode, fault)
                receipt = json.loads((root / mode / "receipt.json").read_text())
                self.assertEqual(receipt["verdict"], "failed")
                self.assertTrue(receipt["sources_restored"])
                self.assertEqual(
                    (root / mode / "resident.stderr").read_bytes(),
                    b"untouched raw resident.stderr",
                )
                self.assertEqual(
                    (root / mode / "fixture.rs.rawbackup").read_bytes(), arithmetic.ROOT
                )
                self.assertTrue(receipt["observations"])

    def test_top_level_frozen_identity_failure_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(
                arithmetic.shared,
                "identity",
                side_effect=RuntimeError("frozen identity unavailable"),
            ):
                with self.assertRaisesRegex(RuntimeError, "frozen identity"):
                    arithmetic.capture("/not/invoked", root)
            receipt = json.loads((root / "receipt.json").read_text())
            self.assertEqual(receipt["verdict"], "failed")
            self.assertIn("frozen identity", receipt["error"])


if __name__ == "__main__":
    unittest.main()
