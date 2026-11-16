"""Mock-only tests. These do not establish native acceptance or performance."""

import base64
import io
import json
import os
from pathlib import Path
import subprocess
from types import SimpleNamespace

import pytest

import stable_resident_owned_retirement as harness


@pytest.mark.parametrize(
    "mutation", ["correct", "wrong-const", "wrong-fn", "signature", "invalidmetadata"]
)
def test_exact_compiler_consumer(tmp_path, mutation):
    """Opt-in frozen compiler integration, never builds the compiler itself."""
    compiler = os.environ.get("JUSTRUST_RETIREMENT_TEST_COMPILER")
    if not compiler:
        pytest.skip("Set JUSTRUST_RETIREMENT_TEST_COMPILER to an exact frozen compiler")
    source = tmp_path / "fixture.rs"
    source.write_text(
        f"pub const RETIREMENT_VALUE: u64 = {43 if mutation == 'wrong-const' else 42};\n"
        f"pub const fn retirement_value() -> {'u32' if mutation == 'signature' else 'u64'} "
        f"{{ {43 if mutation == 'wrong-fn' else 42} }}\n"
    )
    result = harness.owned.shared.run(
        [compiler, *harness.arguments(tmp_path)],
        30,
        cwd=tmp_path,
        env=harness.owned.environment(),
    )
    assert result["status"] == 0
    artifact = tmp_path / "fixture.rmeta"
    if mutation == "invalidmetadata":
        artifact.write_bytes(b"not rust metadata")
    original = artifact.read_bytes()
    row = {}
    if mutation == "correct":
        harness.consume_metadata(compiler, tmp_path, artifact, mutation, row, 30)
        assert row["consumer_negative"]["status"] != 0
    else:
        with pytest.raises(AssertionError, match="rejected exported"):
            harness.consume_metadata(compiler, tmp_path, artifact, mutation, row, 30)
        assert row["consumer_positive"]["status"] != 0
        assert row["consumer_positive"]["stderr_b64"]
    artifact.unlink()
    assert Path(row["metadata_path"]).read_bytes() == original


def frame(index=64):
    return dict(
        id=index,
        pid=42,
        status=0,
        stdout_b64="",
        stderr_b64="",
        executed=True,
        owned=dict(
            contexts=index,
            revisions=index,
            rss_bytes=1024,
            retirement_reason="context_capacity" if index == 64 else None,
        ),
    )


@pytest.mark.parametrize("index", [1, 63, 64])
def test_valid_frame(index):
    harness.validate_frame(frame(index), index, 42, frame())


@pytest.mark.parametrize(
    "mutation",
    [
        "early",
        "missing",
        "reason",
        "contexts",
        "revisions",
        "failed",
        "boolean-status",
        "pid",
        "timeout",
        "missing-reason",
        "missing-rss",
        "rss-budget",
        "refused",
    ],
)
def test_reject_frame(mutation):
    row = frame(1 if mutation == "early" else 64)
    if mutation == "early":
        row["owned"]["retirement_reason"] = "context_capacity"
    elif mutation == "missing":
        row.pop("executed")
    elif mutation == "reason":
        row["owned"]["retirement_reason"] = "request_limit"
    elif mutation == "missing-reason":
        row["owned"].pop("retirement_reason")
    elif mutation == "missing-rss":
        row["owned"].pop("rss_bytes")
    elif mutation == "rss-budget":
        row["owned"]["rss_bytes"] = 2 * 1024 * 1024 * 1024
    elif mutation == "refused":
        row["executed"] = False
    elif mutation in ("contexts", "revisions"):
        row["owned"][mutation] -= 1
    elif mutation == "failed":
        row["status"] = 1
    elif mutation == "boolean-status":
        row["status"] = False
    elif mutation == "pid":
        row["pid"] = 43
    else:
        row["timed_out"] = True
    with pytest.raises(AssertionError):
        harness.validate_frame(row, row["id"], 42, frame())


@pytest.mark.parametrize(
    "failure",
    [
        None,
        "early",
        "failed",
        "identity",
        "shutdown",
        "wrongvalue",
        "invalidmetadata",
        "consumerfailure",
        "negative-accepted",
        "baseline-changed",
    ],
)
def test_capture_receipt(tmp_path, monkeypatch, failure):
    commands = []
    workers = []
    identities = []

    def identity(compiler):
        identities.append(str(compiler))
        return dict(identity=len(identities) if failure == "identity" else 1)

    def compile_fresh(command, timeout, cwd, env):
        if "--extern" in command:
            negative = command[1].endswith("negative.rs")
            status = 1 if negative else 0
            if not negative and failure in (
                "wrongvalue",
                "invalidmetadata",
                "consumerfailure",
            ):
                status = 1
            if failure == "negative-accepted":
                status = 0
            return dict(
                status=status,
                stdout_b64="",
                stderr_b64=(
                    "changed"
                    if failure == "baseline-changed" and workers and not negative
                    else ""
                ),
            )
        commands.append(command[1:])
        (cwd / "fixture.rmeta").write_bytes(b"valid mock metadata")
        return {k: frame()[k] for k in ("status", "stdout_b64", "stderr_b64")}

    class Worker:
        def __init__(self, compiler, root, timeout):
            self.process = SimpleNamespace(pid=42)
            self.command = [str(compiler), "--jr-resident-owned"]
            self.index = 0
            self.closed = False
            workers.append(self)

        def compile(self, argv):
            assert argv == commands[0]
            self.index += 1
            (tmp_path / "fixture.rmeta").write_bytes(
                f"metadata revision {self.index}".encode()
            )
            result = frame(self.index)
            if failure == "early":
                result["owned"]["retirement_reason"] = "context_capacity"
            if failure == "failed":
                result["status"] = 1
            return result

        def close(self):
            self.closed = True

    def retire(worker, argv, report, timeout):
        assert worker.index == 64
        if failure == "shutdown":
            raise AssertionError("shutdown timeout")
        report["shutdown"] = dict(status=0)

    monkeypatch.setattr(harness.owned.shared, "identity", identity)
    monkeypatch.setattr(harness.owned.shared, "run", compile_fresh)
    monkeypatch.setattr(harness.owned, "OwnedResident", Worker)
    monkeypatch.setattr(harness, "retire", retire)
    if failure:
        with pytest.raises(AssertionError):
            harness.capture(tmp_path / "compiler", tmp_path)
    else:
        report = harness.capture(tmp_path / "compiler", tmp_path)
        assert len(report["observations"]) == 64
        assert commands[0] == commands[1]
        assert report["identity_before"] == report["identity_after"]
        for row in report["observations"]:
            assert not row["metadata_matches_fresh_bytes"]
            assert (
                tmp_path / "artifacts" / f"owned-{row['index']:02}" / "libfixture.rmeta"
            ).read_bytes() == f"metadata revision {row['index']}".encode()
            assert row["consumer_positive"]["status"] == 0
            assert row["consumer_negative"]["status"] == 1
    receipt = json.loads((tmp_path / "receipt.json").read_text())
    assert receipt["verdict"] == ("failed" if failure else "passed")
    if receipt["observations"]:
        assert receipt["observations"][-1]["compile"]
    assert receipt["identity_after"]
    assert len(identities) == 2
    if workers:
        assert workers[0].closed


@pytest.mark.parametrize(
    "failure", ["wrongvalue", "invalidmetadata", "consumerfailure"]
)
def test_consumer_failure_preserves_copy_and_raw_result(tmp_path, monkeypatch, failure):
    artifact = tmp_path / "fixture.rmeta"
    artifact.write_bytes(failure.encode())
    raw = dict(status=1, stdout_b64="b3V0", stderr_b64="ZXJyb3I=")
    monkeypatch.setattr(harness.owned.shared, "run", lambda *a, **k: dict(raw))
    row = {}
    with pytest.raises(AssertionError, match="rejected exported"):
        harness.consume_metadata(
            "/exact/compiler", tmp_path, artifact, "failed", row, 1
        )
    artifact.write_bytes(b"replaced later")
    assert (
        tmp_path / "artifacts/failed/libfixture.rmeta"
    ).read_bytes() == failure.encode()
    assert all(row["consumer_positive"][key] == value for key, value in raw.items())
    command = row["consumer_positive"]["command"]
    assert command[0] == "/exact/compiler"
    assert "--extern" in command
    source = (tmp_path / "artifacts/failed/positive.rs").read_text()
    assert "const _: fn() -> u64" in source
    assert "RETIREMENT_VALUE == 42" in source
    assert "retirement_value() == 42" in source


@pytest.mark.parametrize("mutation", [None, "shutdown", "extra", "nonzero"])
def test_retirement_real_pipe_final_frame_preserved(tmp_path, monkeypatch, mutation):
    """Shared transport parses a real framed pipe, but Popen never runs a compiler."""
    import stable_resident_owned as owned

    class Input(io.BytesIO):
        def __init__(self, fd):
            super().__init__()
            self.fd = fd

        def write(self, payload):
            request = json.loads(payload)
            if request["id"] <= 64:
                result = frame(request["id"])
                result["elapsed_seconds"] = 0.001
                os.write(self.fd, json.dumps(result).encode() + b"\n")
            elif mutation == "extra":
                os.write(self.fd, b'{"id":65}\n')
            if request["id"] == 65:
                os.close(self.fd)
                self.fd = None
            return len(payload)

        def close(self):
            if self.fd is not None:
                os.close(self.fd)
                self.fd = None
            super().close()

    def popen(command, **kwargs):
        process = SimpleNamespace(
            pid=42, returncode=None, stdin=Input(os.dup(kwargs["pass_fds"][0]))
        )

        def wait(timeout=None):
            if mutation == "shutdown" and timeout != 5 and process.returncode is None:
                raise subprocess.TimeoutExpired(command, timeout)
            process.returncode = 1 if mutation == "nonzero" else 0
            return process.returncode

        process.wait = wait
        process.kill = lambda: setattr(process, "returncode", -9)
        return process

    monkeypatch.setattr(owned.subprocess, "Popen", popen)
    worker = owned.OwnedResident("/mock/compiler", tmp_path)
    report = {}
    try:
        for index in range(1, 65):
            result = worker.compile(["fixture.rs"])
            harness.validate_frame(result, index, 42, frame())
        if mutation:
            with pytest.raises(AssertionError):
                harness.retire(worker, ["fixture.rs"], report, 1)
        else:
            harness.retire(worker, ["fixture.rs"], report, 1)
            assert report["shutdown"] == dict(status=0, timed_out=False)
            assert report["trailing_response_b64"] == ""
    finally:
        if mutation == "nonzero":
            with pytest.raises(RuntimeError):
                worker.close()
        else:
            worker.close()
    transcript = [
        json.loads(line)
        for line in (tmp_path / "resident.jsonl").read_bytes().splitlines()
    ]
    responses = [
        json.loads(base64.b64decode(row["response_b64"]))
        for row in transcript
        if "response_b64" in row
    ]
    assert len(responses) == 64
    assert responses[-1]["id"] == 64
    assert responses[-1]["executed"] is True
    assert responses[-1]["owned"]["retirement_reason"] == "context_capacity"
    assert responses[-1]["owned"]["contexts"] == 64
    assert (tmp_path / "resident.stdout").exists()
    assert (tmp_path / "resident.stderr").exists()


def test_cli_requires_explicit_paths_and_new_directory(tmp_path, monkeypatch):
    monkeypatch.setattr("sys.argv", ["retirement"])
    with pytest.raises(SystemExit):
        harness.main()
    monkeypatch.setattr(
        "sys.argv",
        ["retirement", "--compiler", "/mock/compiler", "--output", str(tmp_path)],
    )
    with pytest.raises(FileExistsError):
        harness.main()
    assert "--emit=metadata" in harness.arguments(tmp_path)
    assert "--crate-type=rlib" in harness.arguments(tmp_path)
