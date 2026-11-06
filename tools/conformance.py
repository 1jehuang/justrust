#!/usr/bin/env python3
"""Executable compatibility evidence index. No third-party Python dependencies.

A green run is evidence for its exact selected gates, never full Rust support.
Use plan before run. Runtime reports/logs belong in scratch or ignored target/.
"""
from __future__ import annotations

import argparse
import ast
import fcntl
import hashlib
import json
import marshal
import os
from pathlib import Path
import re
import signal
import shutil
import stat
import subprocess
import sys
import time
from types import CodeType, FunctionType

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = Path("data/compiler-conformance.json")
KINDS = {"tests", "lib", "all-tests", "clippy", "fmt", "case"}
STATES = {"open", "partial", "implemented-slice"}
ENV_KEYS = ("JUSTRUST_REMOTE_REQUIRED", "JUSTRUST_REMOTE", "JUSTRUST_TEST_PARALLEL",
            "JR_STOCK_RUSTC", "JUSTRUST_REAL_RUSTC", "CARGO_PROFILE_DEV_DEBUG",
            "CARGO_PROFILE_TEST_DEBUG", "CARGO_INCREMENTAL", "RUSTFLAGS", "RUSTUP_TOOLCHAIN")


class InvalidEvidence(ValueError):
    pass


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def encoded(value) -> bytes:
    return (json.dumps(value, sort_keys=True, indent=2) + "\n").encode()


def load_json(path: Path):
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError) as error:
        raise InvalidEvidence(f"cannot read JSON {path}: {error}") from error


def integration_suites(root: Path = ROOT) -> list[str]:
    return sorted(path.stem for path in (root / "tests").glob("*.rs"))


def unique_ids(rows: list, label: str) -> set[str]:
    if not isinstance(rows, list) or not rows:
        raise InvalidEvidence(f"{label} must be a nonempty array")
    ids = []
    for row in rows:
        name = row.get("id") if isinstance(row, dict) else None
        if not isinstance(name, str) or not re.fullmatch(r"[a-z][a-z0-9-]*", name):
            raise InvalidEvidence(f"invalid {label} identifier: {name!r}")
        ids.append(name)
    if len(ids) != len(set(ids)):
        raise InvalidEvidence(f"duplicate {label} identifier")
    return set(ids)


def validate(data: dict, root: Path = ROOT) -> dict:
    if not isinstance(data, dict) or data.get("schema_version") != 1:
        raise InvalidEvidence("unsupported manifest schema")
    contract = data.get("contract", {})
    if contract.get("semantic_inventory") != "incomplete":
        raise InvalidEvidence("this finite inventory cannot certify specification completeness")
    if contract.get("edition") != "2024" or contract.get("target") != "x86_64-unknown-linux-gnu":
        raise InvalidEvidence("unreviewed edition/target contract")
    oracle = contract.get("oracle", {})
    verifier = root / oracle.get("verifier", "absent")
    text = verifier.read_text() if verifier.is_file() else ""
    if not all(isinstance(oracle.get(key), str) and oracle[key] in text
               for key in ("release", "commit", "llvm")):
        raise InvalidEvidence("oracle contract is not bound to the exact existing verifier")
    gates = unique_ids(data.get("gates"), "gate")
    requirements = unique_ids(data.get("requirements"), "requirement")
    suites = set(integration_suites(root))
    for gate in data["gates"]:
        if gate.get("kind") not in KINDS:
            raise InvalidEvidence(f"unknown gate kind: {gate['id']}")
        if not isinstance(gate.get("profiles"), list) or any(
                name not in {"smoke", "full"} for name in gate["profiles"]):
            raise InvalidEvidence(f"invalid profiles: {gate['id']}")
        if gate["kind"] == "tests":
            selected = gate.get("suites")
            if not selected or len(set(selected)) != len(selected) or not set(selected) <= suites:
                raise InvalidEvidence(f"missing or duplicate integration suites: {gate['id']}")
        elif gate["kind"] == "case":
            if (gate.get("suite") not in suites
                    or not re.fullmatch(r"[a-zA-Z_][a-zA-Z0-9_]*", gate.get("test", ""))):
                raise InvalidEvidence(f"invalid exact test selector: {gate['id']}")
        elif "suites" in gate:
            raise InvalidEvidence(f"unexpected suite filter on {gate['id']}")
    for requirement in data["requirements"]:
        if requirement.get("state") not in STATES or not requirement.get("remaining"):
            raise InvalidEvidence(f"missing honest scope/remainder: {requirement['id']}")
        if not requirement.get("gates") or not set(requirement["gates"]) <= gates:
            raise InvalidEvidence(f"unmapped requirement: {requirement['id']}")
    gap_ids = set()
    for gap in data.get("known_gaps", []):
        if gap.get("id") in gap_ids or gap.get("requirement") not in requirements:
            raise InvalidEvidence("duplicate or unmapped known gap")
        gap_ids.add(gap.get("id"))
        if gap.get("suite") not in suites or gap.get("classification") not in {
                "valid-rejected", "invalid-accepted", "wrong-code", "untested"}:
            raise InvalidEvidence("invalid known gap classification/suite")
        if not gap.get("test") or gap["test"] not in (root / "tests" / (gap["suite"] + ".rs")).read_text():
            raise InvalidEvidence("known gap no longer references an existing test")
    return data


def command(gate: dict, target_dir: str) -> list[str]:
    kind = gate["kind"]
    if kind == "fmt":
        return ["cargo", "fmt", "--all", "--check"]
    if kind == "clippy":
        return ["justrust", "clippy", "--all-targets", "--target-dir", target_dir, "--", "-D", "warnings"]
    args = ["justrust", "test", "--no-fail-fast", "--target-dir", target_dir]
    if kind == "tests":
        for suite in gate["suites"]:
            args.extend(["--test", suite])
    elif kind == "case":
        args.extend(["--test", gate["suite"]])
        return args + ["--", gate["test"], "--exact", "--include-ignored", "--nocapture", "--test-threads=1"]
    elif kind == "lib":
        args.append("--lib")
    else:
        args.append("--all-targets")
    return args + ["--", "--test-threads=4"]


def plan(data: dict, profile: str, selected: list[str], target_dir: str) -> list[dict]:
    if profile not in {"smoke", "full"} or not isinstance(selected, list) or not target_dir:
        raise InvalidEvidence("invalid profile, gate selection or target directory")
    known = {gate["id"] for gate in data["gates"]}
    if set(selected) - known or len(selected) != len(set(selected)):
        raise InvalidEvidence("unknown or duplicate selected gates")
    gates = [gate for gate in data["gates"]
             if gate["id"] in selected or (not selected and profile in gate["profiles"])]
    if not gates:
        raise InvalidEvidence("empty plan")
    return [{"id": gate["id"], "kind": gate["kind"], "command": command(gate, target_dir)}
            for gate in gates]


def source_identity(root: Path = ROOT) -> dict:
    try:
        revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root).decode().strip()
        paths = subprocess.check_output(
            ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=root)
    except (OSError, subprocess.CalledProcessError) as error:
        raise InvalidEvidence(f"git provenance unavailable: {error}") from error
    # Include all compiler, library, fixture, runner and build configuration inputs.
    # Historical measurement logs and unrelated agent locks are not compiler inputs.
    inputs = sorted({os.fsdecode(path) for path in paths.split(b"\0") if path and (
        os.fsdecode(path).startswith(("src/", "libraries/", "tests/", "tools/", ".cargo/", "docs/compiler/"))
        or os.fsdecode(path) in {"Cargo.toml", "Cargo.lock", str(MANIFEST),
                                "data/compiler-compatibility-scope.json",
                                "data/compiler-compatibility-contract.json",
                                "data/compiler-language-inventory.json",
                                "data/compiler-language-reference-rules.json",
                                "data/compiler-library-surface-provenance.json",
                                "data/compiler-library-project-inventory.json",
                                ".github/workflows/compiler-conformance.yml"})})
    state = hashlib.sha256()
    for name in inputs:
        path = root / name
        if path.is_symlink():
            value = ["symlink", os.readlink(path), digest(path.read_bytes()) if path.is_file() else None]
        elif path.is_file():
            value = ["file", digest(path.read_bytes())]
        else:
            value = ["deleted"]
        state.update(encoded([name, value]))
    return {"revision": revision, "inputs_sha256": state.hexdigest(), "input_files": len(inputs)}


def loaded_runner_identity(module) -> dict:
    """Hash actual loaded code and reject stale/foreign patched callables.

    Compiling for comparison executes no module code or descriptors. This is a
    bounded runner authenticity check, not arbitrary Python state attestation.
    """
    path = Path(module.__file__).resolve()
    source = path.read_bytes()
    syntax = ast.parse(source, filename=str(path))
    compiled = compile(source, str(path), "exec", dont_inherit=True)

    def normalized(value):
        if isinstance(value, CodeType):
            return value.replace(co_filename=str(path), co_consts=tuple(normalized(item) for item in value.co_consts))
        if isinstance(value, frozenset):
            return ("runner-frozenset", tuple(sorted((normalized(item) for item in value), key=marshal.dumps)))
        if isinstance(value, tuple):
            return tuple(normalized(item) for item in value)
        return sys.intern(value) if isinstance(value, str) else value

    def functions(nodes, constants, namespace, prefix=""):
        outputs = {}
        codes = {value.co_name: value for value in constants if isinstance(value, CodeType)}
        for node in nodes:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                value = namespace.get(node.name)
                if type(value) in {staticmethod, classmethod}:
                    value = value.__func__
                expected = codes[node.name]
                if (type(value) is not FunctionType
                        or Path(value.__code__.co_filename).resolve() != path
                        or value.__code__ != expected):
                    raise InvalidEvidence(f"stale or foreign loaded runner callable: {prefix + node.name}")
                outputs[prefix + node.name] = digest(marshal.dumps(normalized(value.__code__)))
            elif isinstance(node, ast.ClassDef):
                value = namespace.get(node.name)
                if type(value) is not type or value.__module__ != module.__name__:
                    raise InvalidEvidence(f"foreign loaded runner class: {node.name}")
                outputs.update(functions(node.body, codes[node.name].co_consts, vars(value), prefix + node.name + "."))
        return outputs

    return {"source_sha256": digest(source),
            "loaded_code_sha256": functions(syntax.body, compiled.co_consts, vars(module))}


class GitSnapshot:
    """Private, read-only Git blob materialization, not a working checkout.

    The lock excludes cooperating campaigns. Exact pre/post verification detects
    accidental drift, not malicious transient writes by another same-uid process.
    No manifest of self-authored hashes is accepted as Git object authority.
    """

    def __init__(self, directory, repository, commit):
        if not repository.is_absolute():
            raise InvalidEvidence("snapshot repository must be an absolute Git working-root path")
        self.directory = directory.parent.resolve(strict=True) / directory.name
        self.repository = repository.resolve(strict=True)
        self.commit = commit
        self.ROOT = self.directory / "source"
        self.lock = None
        if not re.fullmatch(r"[0-9a-f]{40}", commit):
            raise InvalidEvidence("snapshot requires a full lowercase Git commit ID")
        if self.directory.is_relative_to(self.repository) or ".git" in self.directory.parts:
            raise InvalidEvidence("snapshot must be outside the source repository")

    def git(self, *args, payload=None):
        try:
            env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
            return subprocess.check_output(
                ["git", "--no-replace-objects", "-C", str(self.repository), *args],
                input=payload, stderr=subprocess.PIPE, env=env)
        except (OSError, subprocess.CalledProcessError) as error:
            raise InvalidEvidence(f"snapshot Git objects unavailable: {error}") from error

    def objects(self):
        if Path(os.fsdecode(self.git("rev-parse", "--show-toplevel")).strip()).resolve() != self.repository:
            raise InvalidEvidence("snapshot repository must name its exact Git working root")
        if self.git("rev-parse", "--verify", self.commit + "^{commit}").decode().strip() != self.commit:
            raise InvalidEvidence("snapshot commit identity mismatch")
        tree = self.git("rev-parse", "--verify", self.commit + "^{tree}").decode().strip()
        def verify_object(kind, oid):
            value = self.git("cat-file", kind, oid)
            actual = hashlib.sha1(kind.encode() + b" " + str(len(value)).encode() + b"\0" + value).hexdigest()
            if actual != oid:
                raise InvalidEvidence(f"snapshot Git {kind} hash mismatch")
        verify_object("commit", self.commit)
        verify_object("tree", tree)
        entries = []
        tree_paths = set()
        total = 0
        for row in self.git("ls-tree", "-rtlz", "--full-tree", self.commit).split(b"\0"):
            if not row:
                continue
            header, raw_name = row.split(b"\t", 1)
            mode, kind, oid, size = header.split()
            name = os.fsdecode(raw_name)
            parts = name.split("/")
            if (any(part in {"", ".", "..", ".git"} for part in parts)
                    or "\\" in name or len(raw_name) > 4095
                    or any(len(os.fsencode(part)) > 255 for part in parts)):
                raise InvalidEvidence(f"unsupported or unsafe snapshot entry: {name!r}")
            if kind == b"tree" and mode == b"040000":
                tree_paths.add(name)
                if len(tree_paths) > 100000:
                    raise InvalidEvidence("snapshot infrastructure limit: 100000 directories")
                verify_object("tree", oid.decode())
                continue
            if mode not in {b"100644", b"100755"} or kind != b"blob":
                raise InvalidEvidence(f"unsupported or unsafe snapshot entry: {name!r}")
            total += int(size)
            if len(entries) >= 100000 or total > 512 * 1024 * 1024:
                raise InvalidEvidence("snapshot infrastructure limit: 100000 files / 512 MiB")
            entries.append((name, 0o555 if mode == b"100755" else 0o444, oid))
        if not entries:
            raise InvalidEvidence("empty snapshot tree")
        implied_directories = {str(parent) for name, _, _ in entries for parent in Path(name).parents
                               if str(parent) != "."}
        if tree_paths != implied_directories:
            raise InvalidEvidence("unsupported empty Git subtree")
        # Raw blobs deliberately bypass export-ignore/export-subst, filters and
        # external attributes. Every committed path must be materialized exactly.
        packed = self.git("cat-file", "--batch", payload=b"".join(oid + b"\n" for _, _, oid in entries))
        offset = 0
        result = []
        for name, mode, oid in entries:
            end = packed.index(b"\n", offset)
            actual, kind, size = packed[offset:end].split()
            size = int(size)
            start = end + 1
            value = packed[start:start + size]
            if actual != oid or kind != b"blob" or packed[start + size:start + size + 1] != b"\n":
                raise InvalidEvidence("invalid snapshot Git blob response")
            # cat-file must supply the object named by ls-tree, not arbitrary bytes.
            if hashlib.sha1(b"blob " + str(size).encode() + b"\0" + value).hexdigest().encode() != oid:
                raise InvalidEvidence("snapshot Git blob hash mismatch")
            result.append((name, mode, value))
            offset = start + size + 1
        if offset != len(packed):
            raise InvalidEvidence("extra snapshot Git blob response")
        return tree, result

    @staticmethod
    def owned(path, directory=False, mode=None):
        info = path.lstat()
        if (info.st_uid != os.getuid() or info.st_mode & 0o022
                or not (stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode))
                or (not directory and info.st_nlink != 1)
                or (mode is not None and stat.S_IMODE(info.st_mode) != mode)):
            raise InvalidEvidence(f"unsafe snapshot ownership/type/mode: {path}")
        return info

    def create(self):
        if self.directory.exists() or self.directory.is_symlink():
            raise InvalidEvidence("snapshot creation requires a new private container")
        _, entries = self.objects()
        if shutil.disk_usage(self.directory.parent).free < sum(len(value) for _, _, value in entries) + 64 * 1024 * 1024:
            raise InvalidEvidence("snapshot infrastructure limit: insufficient free disk")
        # Only a newly created container is ever written or chmodded. On failure
        # retain it for diagnosis, never recursively delete caller-owned paths.
        self.directory.mkdir(mode=0o700)
        descriptor = os.open(self.directory / "owner.lock", os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(descriptor, "wb") as output:
            output.write(b"justrust-git-snapshot-v1\n" + os.urandom(32).hex().encode() + b"\n")
        self.ROOT.mkdir(mode=0o700)
        for name, mode, value in entries:
            path = self.ROOT / name
            path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            with path.open("xb") as output:
                output.write(value)
            path.chmod(mode)
        for parent, directories, _ in os.walk(self.ROOT, topdown=False):
            for name in directories:
                (Path(parent) / name).chmod(0o500)
        self.ROOT.chmod(0o500)

    def check_lock(self):
        current = self.owned(self.directory, directory=True, mode=0o700)
        if (current.st_dev, current.st_ino) != self.container_inode:
            raise InvalidEvidence("snapshot container replaced")
        path = self.directory / "owner.lock"
        current = self.owned(path, mode=0o600)
        opened = os.fstat(self.lock.fileno())
        if (current.st_dev, current.st_ino) != (opened.st_dev, opened.st_ino):
            raise InvalidEvidence("snapshot ownership lock replaced")
        if opened.st_size != len(b"justrust-git-snapshot-v1\n") + 65:
            raise InvalidEvidence("invalid snapshot ownership lock size")
        if "owner" in self.__dict__:
            if digest(os.pread(self.lock.fileno(), opened.st_size, 0)) != self.owner:
                raise InvalidEvidence("snapshot ownership token changed")
        if set(path.name for path in self.directory.iterdir()) != {"source", "owner.lock"}:
            raise InvalidEvidence("unexpected snapshot container entries")

    def __enter__(self):
        # Snapshot provenance inventories loaded behavior before any receipt is
        # taken. Legacy mode retains its existing lazy-import behavior.
        import compatibility_scope
        info = self.owned(self.directory, directory=True, mode=0o700)
        self.container_inode = (info.st_dev, info.st_ino)
        path = self.directory / "owner.lock"
        self.owned(path, mode=0o600)
        descriptor = os.open(path, os.O_RDWR | os.O_NOFOLLOW)
        self.lock = os.fdopen(descriptor, "r+b")
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.check_lock()
            token = self.lock.read()
            if not re.fullmatch(rb"justrust-git-snapshot-v1\n[0-9a-f]{64}\n", token):
                raise InvalidEvidence("invalid snapshot ownership lock")
            self.owner = digest(token)
            source = self.owned(self.ROOT, directory=True, mode=0o500)
            self.source_inode = (source.st_dev, source.st_ino)
            self.source_identity()
            return self
        except BaseException:
            self.lock.close()
            raise

    def __exit__(self, *args):
        self.lock.close()

    def source_identity(self, root=None):
        if root is not None and root != self.ROOT:
            raise InvalidEvidence("snapshot runner requested a different source root")
        self.check_lock()
        tree, entries = self.objects()
        expected = {name: (mode, value) for name, mode, value in entries}
        directories = {""}
        for name in expected:
            directories.update(str(parent) for parent in Path(name).parents if str(parent) != ".")
        observed = set()
        observed_dirs = {""}
        source = self.owned(self.ROOT, directory=True, mode=0o500)
        if (source.st_dev, source.st_ino) != self.source_inode:
            raise InvalidEvidence("snapshot source directory replaced")
        for parent, children, files in os.walk(self.ROOT, followlinks=False):
            for name in children:
                path = Path(parent) / name
                self.owned(path, directory=True, mode=0o500)
                observed_dirs.add(str(path.relative_to(self.ROOT)))
            for name in files:
                path = Path(parent) / name
                relative = str(path.relative_to(self.ROOT))
                if relative not in expected:
                    raise InvalidEvidence(f"extra snapshot source: {relative}")
                mode, value = expected[relative]
                self.owned(path, mode=mode)
                if path.read_bytes() != value:
                    raise InvalidEvidence(f"snapshot source bytes changed: {relative}")
                observed.add(relative)
        if observed != set(expected) or observed_dirs != directories:
            raise InvalidEvidence("missing or extra snapshot source paths")
        # Hash the actual host runner and every Python helper available to its
        # dynamic imports, separately from the unchanged archived compiler tree.
        runner_root = Path(__file__).resolve().parent
        for name in ("compatibility_scope", "compatibility_inventory"):
            module = sys.modules.get(name)
            if module and Path(module.__file__).resolve() != runner_root / (name + ".py"):
                raise InvalidEvidence(f"snapshot runner imported foreign helper: {name}")
        runner = {path.name: digest(path.read_bytes()) for path in sorted(runner_root.glob("*.py"))}
        loaded = {"conformance.py": loaded_runner_identity(sys.modules[__name__])}
        for name in ("compatibility_scope", "compatibility_inventory"):
            if name in sys.modules:
                loaded[name + ".py"] = loaded_runner_identity(sys.modules[name])
        inventory = [[name, mode, digest(value)] for name, mode, value in entries]
        self.check_lock()
        return {"provenance": "git-archive-v1", "revision": self.commit, "tree": tree,
                "inputs_sha256": digest(encoded(inventory)), "input_files": len(entries),
                "snapshot_owner_sha256": self.owner, "executed_runner": runner,
                "loaded_runner": loaded,
                "snapshot_source": {"path": str(self.ROOT), "device": source.st_dev, "inode": source.st_ino},
                "python": {"executable": str(Path(sys.executable).resolve()),
                           "sha256": digest(Path(sys.executable).read_bytes()), "version": sys.version}}

    def __getattr__(self, name):
        # Tracker passes ROOT explicitly to source consumers and threaded cases.
        # Keep default module functions and their legacy defaults untouched.
        try:
            return globals()[name]
        except KeyError:
            raise AttributeError(name) from None


def discover(root: Path = ROOT) -> dict:
    """Lexical discovery flags needing triage, not semantic classifications."""
    markers = []
    oracle_selectors = []
    test_functions = []
    for path in sorted((root / "tests").rglob("*.rs")):
        lines = path.read_text().splitlines()
        pending_test = False
        for number, line in enumerate(lines, 1):
            if re.match(r"\s*#\[test\]", line):
                pending_test = True
            function = re.match(r"\s*(?:pub\s+)?fn\s+(\w+)\s*\(", line)
            if pending_test and function:
                test_functions.append({"file": str(path.relative_to(root)), "line": number,
                                       "test": function.group(1)})
                pending_test = False
            if re.search(r"(?i)unsupported|#\[ignore|known.?gap|wrong.?code", line):
                markers.append({"file": str(path.relative_to(root)), "line": number,
                                "text": line.strip()[:240]})
            if re.search(r'Command::new\s*\(\s*"rustc"\s*\)|var(?:_os)?\s*\(\s*"(?:RUSTC|JUSTRUST_REAL_RUSTC)"', line):
                oracle_selectors.append({"file": str(path.relative_to(root)), "line": number,
                                         "text": line.strip()[:240]})
    manifest = root / "tests/fixtures/full_programs/manifest.tsv"
    return {"integration_suites": integration_suites(root), "source_test_functions": test_functions,
            "triage_markers": markers,
            "oracle_selector_leads": oracle_selectors,
            "warning": "Lexical markers include rejection assertions, comments and source strings. Oracle selector leads require identity review, not automatic rewriting. Do not count them as features or proven gaps.",
            "full_program_manifest": str(manifest.relative_to(root)) if manifest.exists() else None}


def test_count(text: str) -> int:
    # Stable client's summary or direct libtest output. Never infer support from exit zero alone.
    counts = re.findall(r"justrust: ok: (\d+) tests? passed", text)
    if not counts:
        counts = re.findall(r"test result: ok\. (\d+) passed;", text)
    return sum(map(int, counts))


def client_retried_failures(text: str) -> bool:
    # Match complete compiler-owned diagnostics, not user test names containing
    # "retry", "parallel" or "failures". The installed client can replay failed
    # tests itself even though this driver never retries a command.
    markers = (
        r"justrust: [^:\r\n]+: \S+ failed while other test processes ran in parallel "
        r"and passed when rerun alone by name; not counted as failures",
        r"justrust: [^:\r\n]+: confirming \d+ failures by replaying the original test "
        r"group alone instead of retrying each test serially",
    )
    return any(re.fullmatch(marker, line) for line in text.splitlines() for marker in markers)


FAILURE_ANNOTATIONS = (
    "valid-source-rejected", "invalid-source-accepted",
    "semantic-stage-unproven", "rejection-proof-unproven",
)


def failure_annotation(text: str) -> str:
    """Diagnostic only. Ambiguous or legacy logs remain explicitly unannotated."""
    markers = {"[justrust-conformance-failure:v1] " + value: value
               for value in FAILURE_ANNOTATIONS}
    # Split only on LF, accepting CRLF, not arbitrary Unicode line separators.
    found = {markers[line.removesuffix("\r")] for line in text.split("\n")
             if line.removesuffix("\r") in markers}
    return next(iter(found)) if len(found) == 1 else "unannotated"


def outcome(returncode: int | None, text: str, kind: str, timed_out: bool = False) -> str:
    if timed_out:
        return "timeout"
    if returncode is None:
        return "infrastructure"
    if returncode != 0:
        return "failed"
    if client_retried_failures(text):
        return "unverified"
    if "0 tests matched" in text:
        return "unverified"
    if kind in {"tests", "lib", "all-tests", "case"} and test_count(text) == 0:
        return "unverified"
    if kind == "case" and test_count(text) != 1:
        return "unverified"
    return "passed"


def execute(entry: dict, directory: Path, timeout: int, root: Path = ROOT) -> dict:
    log = directory / (entry["id"] + ".log")
    started = time.monotonic()
    code, timed_out = None, False
    env = dict(os.environ)
    env["JUSTRUST_TEST_PARALLEL"] = "0"
    # Do not inject an oracle executable into the compiler's native path.
    # Individual differential harnesses validate their stock oracle separately.
    with log.open("wb") as output:
        try:
            child = subprocess.Popen(entry["command"], cwd=root, env=env, stdout=output,
                                     stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = child.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
        except OSError as error:
            output.write(str(error).encode())
    payload = log.read_bytes()
    text = payload.decode(errors="replace")
    result = {**entry, "outcome": outcome(code, text, entry["kind"], timed_out),
              "returncode": code, "timed_out": timed_out,
              "seconds": round(time.monotonic() - started, 3),
              "tests_passed": test_count(text), "log": log.name, "log_sha256": digest(payload)}
    # Preserve the full original log, including any client-owned retry. Such a
    # retry cannot turn this gate green even when the client exits successfully.
    print(f"{entry['id']}: {result['outcome']} ({result['seconds']}s), {log}", flush=True)
    if result["outcome"] != "passed":
        print(text, flush=True)
    return result


def assess(data: dict, report: dict, directory: Path, identity: dict, root: Path = ROOT) -> dict:
    problems = []
    if report.get("schema_version") != 1:
        raise InvalidEvidence("unsupported report schema")
    if report.get("manifest_sha256") != digest(encoded(data)):
        problems.append("manifest changed")
    if report.get("source_before") != identity or report.get("source_after") != identity:
        problems.append("source revision or compiler inputs changed")
    expected = plan(data, report.get("profile", ""), report.get("selected", []), report.get("target_dir", ""))
    entries = report.get("results", [])
    if not isinstance(entries, list) or not all(isinstance(row, dict) for row in entries):
        raise InvalidEvidence("malformed gate results")
    if [row.get("id") for row in entries] != [row["id"] for row in expected]:
        problems.append("missing, reordered or duplicate gate results")
    verified = set()
    for entry, required in zip(entries, expected):
        if any(entry.get(key) != required[key] for key in ("id", "kind", "command")):
            problems.append(f"changed invocation: {required['id']}")
            continue
        log = directory / str(entry.get("log", ""))
        if log.resolve().parent != directory.resolve() or not log.is_file():
            problems.append(f"missing or unsafe log: {required['id']}")
            continue
        payload = log.read_bytes()
        text = payload.decode(errors="replace")
        measured = outcome(entry.get("returncode"), text, required["kind"], entry.get("timed_out", False))
        if (digest(payload) != entry.get("log_sha256") or measured != entry.get("outcome")
                or entry.get("tests_passed") != test_count(text)):
            problems.append(f"tampered/inconsistent gate evidence: {required['id']}")
        elif measured == "passed":
            verified.add(required["id"])
        else:
            problems.append(f"{required['id']}: {measured}")
    full_gates = {gate["id"] for gate in data["gates"] if "full" in gate["profiles"]}
    all_suites = integration_suites(root)
    if report.get("integration_suites") != all_suites:
        problems.append("integration suite inventory changed")
    return {"selected_gates_passed": not problems,
            "full_gate_set_passed": not problems and full_gates <= verified,
            "full_rust_support_established": False,
            "reason": "Finite suites do not establish complete language/library/project coverage. The semantic inventory and open requirements remain explicit.",
            "problems": problems,
            "requirements": [{"id": row["id"], "scope_state": row["state"],
                               "mapped_gates_passed": not problems and set(row["gates"]) <= verified,
                               "remaining": row["remaining"]} for row in data["requirements"]],
            "known_gaps": data["known_gaps"]}


def main(argv=None, _snapshot=None) -> int:
    root = _snapshot.ROOT if _snapshot else ROOT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["validate", "plan", "discover", "run", "assess", "coverage", "track",
                                           "catalog", "assess-catalog", "catalog-coverage"])
    parser.add_argument("--manifest", type=Path, default=root / MANIFEST)
    parser.add_argument("--profile", choices=["smoke", "full"], default="smoke")
    parser.add_argument("--gate", action="append", default=[])
    parser.add_argument("--target-dir", default=str(ROOT / "target/justrust-slots/0"))
    parser.add_argument("--out", type=Path, help="new output directory for run, existing report directory for assess")
    parser.add_argument("--timeout", type=int, default=3600, help="seconds per gate")
    parser.add_argument("--scope", type=Path, default=root / "data/compiler-compatibility-scope.json")
    parser.add_argument("--area", action="append", default=[], help="track only these compatibility areas")
    parser.add_argument("--case", action="append", default=[], help="track exact mapped suite::test")
    parser.add_argument("--suite", action="append", default=[], help="catalog only: explicitly selected integration suite")
    parser.add_argument("--evidence", type=Path, action="append", default=[], help="coverage: retained track directory; catalog-coverage: retained catalog directory")
    parser.add_argument("--require-complete", action="store_true",
                        help="coverage only: exit zero only when every mapped exact case has current passing evidence")
    parser.add_argument("--require-measured", action="store_true",
                        help="coverage only: require current authenticated pass-or-fail results for every mapped case, not conformance")
    parser.add_argument("--json", action="store_true", help="machine-readable layered coverage")
    parser.add_argument("--dry-run", action="store_true", help="show exact track invocations without executing")
    parser.add_argument("--jobs", type=int, default=1, help="track: independent exact-case processes, 1..8")
    parser.add_argument("--shard", action="append", default=[], metavar="INDEX/COUNT",
                        help="track only: zero-based partition of sorted area/case-selected IDs")
    parser.add_argument("--snapshot", type=Path, help="track/coverage: private immutable snapshot container")
    parser.add_argument("--snapshot-repository", type=Path, help="Git object authority, never its working tree")
    parser.add_argument("--snapshot-commit", help="exact 40-character committed Git object identity")
    parser.add_argument("--create-snapshot", action="store_true", help="track: exclusively create a new snapshot")
    args = parser.parse_args(argv)
    try:
        snapshot_options = (args.snapshot, args.snapshot_repository, args.snapshot_commit)
        if any(snapshot_options) or args.create_snapshot:
            if (not all(snapshot_options) or args.action not in {"track", "coverage"}
                    or (args.create_snapshot and args.action != "track")):
                raise InvalidEvidence("snapshot requires track/coverage and --snapshot, --snapshot-repository, --snapshot-commit")
            if _snapshot is None:
                if args.suite or (args.shard and args.action != "track"):
                    raise InvalidEvidence("snapshot track/coverage does not accept --suite; --shard is track-only")
                if args.require_complete and args.require_measured:
                    raise InvalidEvidence("--require-complete and --require-measured are mutually exclusive")
                if ((args.require_complete or args.require_measured)
                        and (args.action != "coverage" or not args.evidence)):
                    raise InvalidEvidence("--require-complete/--require-measured require coverage with --evidence")
                if args.action == "track" and (not 1 <= args.jobs <= 8
                        or (not args.dry_run and (args.out is None or args.timeout <= 0))):
                    raise InvalidEvidence("snapshot track requires --jobs 1..8, new --out and positive --timeout")
                context = GitSnapshot(*snapshot_options)
                git_directory = Path(os.fsdecode(context.git("rev-parse", "--absolute-git-dir")).strip()).resolve()
                if context.directory.is_relative_to(git_directory):
                    raise InvalidEvidence("snapshot must be outside the Git directory")
                # Relative target paths are interpreted by the child in source,
                # so normalize explicitly to the caller's external scratch path.
                for path in ((Path(args.target_dir), args.out) if args.action == "track" else ()):
                    if path is not None:
                        resolved = path.resolve()
                        if (".git" in resolved.parts or any(resolved.is_relative_to(forbidden) for forbidden in
                                (context.directory, context.repository, git_directory))):
                            raise InvalidEvidence("snapshot requires external --target-dir/--out outside source repository, Git directory and snapshot")
                if args.create_snapshot:
                    context.create()
                with context:
                    return main(argv, context)
            for path in (args.manifest, args.scope):
                if not path.resolve(strict=True).is_relative_to(root):
                    raise InvalidEvidence("snapshot manifest/scope must be committed snapshot files")
            args.target_dir = str(Path(args.target_dir).resolve())
            if args.out is not None:
                args.out = args.out.resolve()
        catalog = args.action in {"catalog", "assess-catalog", "catalog-coverage"}
        if args.suite and args.action != "catalog":
            raise InvalidEvidence("--suite is only valid with catalog")
        if args.action == "catalog-coverage":
            if (not args.evidence or args.out is not None or args.dry_run
                    or args.area or args.case or args.gate or args.jobs != 1
                    or args.profile != "smoke"):
                raise InvalidEvidence("catalog-coverage requires --evidence only, without execution filters, --out or --dry-run")
        elif catalog:
            if (args.area or args.case or args.gate or args.evidence or args.jobs != 1
                    or args.profile != "smoke"):
                raise InvalidEvidence("catalog does not accept execution coverage filters or evidence")
            if args.out is None or (args.action == "assess-catalog" and args.dry_run):
                raise InvalidEvidence("catalog actions require --out; assess-catalog cannot --dry-run")
            if args.action == "catalog" and (not args.suite or len(set(args.suite)) != len(args.suite)
                                              or args.timeout <= 0):
                raise InvalidEvidence("catalog requires unique --suite selections and positive timeout")
        if args.shard and args.action != "track":
            raise InvalidEvidence("--shard is only valid with track")
        if args.require_complete and args.require_measured:
            raise InvalidEvidence("--require-complete and --require-measured are mutually exclusive")
        if args.require_measured and (args.action != "coverage" or not args.evidence):
            raise InvalidEvidence("--require-measured requires coverage with --evidence")
        if args.require_complete and (args.action != "coverage" or not args.evidence):
            raise InvalidEvidence("--require-complete requires coverage with --evidence")
        data = validate(load_json(args.manifest), root) if _snapshot else validate(load_json(args.manifest))
        if catalog:
            import compatibility_catalog
            return compatibility_catalog.main(args, data, sys.modules[__name__])
        if args.action in {"coverage", "track"}:
            import compatibility_scope
            return compatibility_scope.main(args, data, _snapshot or sys.modules[__name__])
        if args.action == "validate":
            import compatibility_scope
            scope = compatibility_scope.validate(load_json(args.scope), sys.modules[__name__], ROOT)
            print(json.dumps({"manifest": "valid", "requirements": len(data["requirements"]),
                              "scope_requirements": sum(len(area["requirements"]) for area in scope["areas"]),
                              "suites": len(integration_suites()), "full_rust_support_established": False}))
        elif args.action == "plan":
            print(encoded({"contract": data["contract"], "gates": plan(data, args.profile, args.gate, args.target_dir),
                           "known_gaps": data["known_gaps"]}).decode(), end="")
        elif args.action == "discover":
            print(encoded(discover()).decode(), end="")
        elif args.action == "assess":
            if args.out is None:
                raise InvalidEvidence("assess requires --out report directory")
            result = assess(data, load_json(args.out / "report.json"), args.out, source_identity())
            print(encoded(result).decode(), end="")
            return 0 if result["selected_gates_passed"] else 1
        else:
            if args.out is None or args.timeout <= 0:
                raise InvalidEvidence("run requires a new --out directory and positive timeout")
            entries = plan(data, args.profile, args.gate, args.target_dir)
            args.out.mkdir(parents=True, exist_ok=False)
            before = source_identity()
            report = {"schema_version": 1, "manifest_sha256": digest(encoded(data)),
                      "source_before": before, "profile": args.profile, "selected": args.gate,
                      "target_dir": args.target_dir, "integration_suites": integration_suites(),
                      "environment": {**{key: os.environ.get(key) for key in ENV_KEYS},
                                      "JUSTRUST_TEST_PARALLEL": "0"}, "results": []}
            (args.out / "discovery.json").write_bytes(encoded(discover()))
            for entry in entries:
                report["results"].append(execute(entry, args.out, args.timeout))
                (args.out / "report.json").write_bytes(encoded(report))
            report["source_after"] = source_identity()
            (args.out / "report.json").write_bytes(encoded(report))
            result = assess(data, report, args.out, source_identity())
            (args.out / "assessment.json").write_bytes(encoded(result))
            print(encoded(result).decode(), end="")
            return 0 if result["selected_gates_passed"] else 1
        return 0
    except (InvalidEvidence, OSError, TypeError, KeyError) as error:
        print(f"conformance: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
