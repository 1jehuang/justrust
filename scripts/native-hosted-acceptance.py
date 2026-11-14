#!/usr/bin/env python3
"""Exercise an explicit candidate on hosted native transport, without installing it.

The control plane must already advertise native transport. This never deploys,
activates a client, changes the default backend, or copies account credentials.
All justrust state and recorded runs belong to a fresh private JUSTRUST_HOME.
A temporary PATH guard refuses SSH. Fault injection kills only that isolated
home's live native session client, closing its real established TCP socket.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time


def digest_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def environment(home, guard, build_base=None):
    env = {key: value for key, value in os.environ.items()
           if not key.startswith("JUSTRUST_")}
    for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "CARGO_BUILD_BUILD_DIR",
                "CARGO_BUILD_TARGET", "RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
                "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTUP_TOOLCHAIN"):
        env.pop(key, None)
    env.update(JUSTRUST_HOME=str(home), JUSTRUST_REMOTE="force",
               JUSTRUST_REMOTE_REQUIRED="1", JUSTRUST_HOSTED_NATIVE="require",
               JUSTRUST_HOSTED_SESSION="1", JUSTRUST_HOSTED_DEV="1",
               JUSTRUST_NO_UPDATE="1", JUSTRUST_TEST_REUSE="0",
               JUSTRUST_EARLY="0", JUSTRUST_SLOTS="off",
               PATH=str(guard) + os.pathsep + env.get("PATH", "/usr/bin:/bin"))
    if build_base:
        env["JUSTRUST_BUILD_BASE"] = build_base
    return env


def select_record(rows, previous, cwd, subcommand, exit_code):
    matches = [row for row in rows if row["id"] not in previous
               and row.get("cwd") == str(cwd)
               and row.get("args", [])[:1] == [subcommand]]
    if len(matches) != 1:
        raise RuntimeError(f"expected one new recorded {subcommand}, got {len(matches)}")
    row = matches[0]
    if (row.get("exit") != exit_code
            or row.get("place", row.get("remote")) != "hosted"):
        raise RuntimeError(f"run {row['id']} has incorrect exit/provenance")
    return row["id"]


def session_script(script):
    try:
        words = shlex.split(script)
        index = words.index("__remote-session")
        mode, token = words[index + 1:index + 3]
        if mode in ("create", "attach") and token:
            return mode, token
    except (ValueError, IndexError):
        pass
    return None


def process_start(proc):
    # comm may contain spaces or parentheses. Fields after its final ')' begin
    # at field 3, so starttime (field 22) is index 19 of that suffix.
    return (proc / "stat").read_text().rsplit(")", 1)[1].split()[19]


def established_socket(proc, port):
    inodes = set()
    for fd in (proc / "fd").iterdir():
        try:
            link = os.readlink(fd)
            if link.startswith("socket:["):
                inodes.add(link[8:-1])
        except OSError:
            pass
    for name in ("tcp", "tcp6"):
        try:
            rows = (proc / "net" / name).read_text().splitlines()[1:]
        except OSError:
            continue
        for row in rows:
            fields = row.split()
            if (fields[3] == "01" and fields[9] in inodes
                    and int(fields[2].rsplit(":", 1)[1], 16) == port):
                return True
    return False


def native_sessions(home, require_socket=False, proc_root=Path("/proc")):
    found = []
    home_entry = b"JUSTRUST_HOME=" + os.fsencode(home)
    for proc in proc_root.iterdir():
        if not proc.name.isdigit():
            continue
        try:
            if proc.stat().st_uid != os.getuid():
                continue
            if home_entry not in (proc / "environ").read_bytes().split(b"\0"):
                continue
            args = [arg.decode(errors="replace") for arg in
                    (proc / "cmdline").read_bytes().split(b"\0") if arg]
            if args[1:3] != ["__native", "client"] or len(args) != 5:
                continue
            session = session_script(args[4])
            if session is None:
                continue
            endpoint = json.loads(args[3])
            if require_socket and not established_socket(proc, endpoint["port"]):
                continue
            found.append((int(proc.name), process_start(proc), *session))
        except (OSError, ValueError, KeyError, IndexError):
            continue
    return found


def drop_session(home):
    candidates = native_sessions(home, require_socket=True)
    if len(candidates) != 1:
        raise RuntimeError(f"expected one isolated live native session, got {len(candidates)}")
    pid, started, mode, token = candidates[0]
    proc = Path("/proc") / str(pid)
    if process_start(proc) != started:
        raise RuntimeError("native transport process changed before fault injection")
    # pidfds avoid PID-reuse races where supported. Never signal a build, daemon,
    # broker, or any process from the normal user's JUSTRUST_HOME.
    if not hasattr(os, "pidfd_open") or not hasattr(signal, "pidfd_send_signal"):
        raise RuntimeError("safe native socket-drop acceptance requires Linux pidfds")
    fd = os.pidfd_open(pid)
    try:
        if process_start(proc) != started:
            raise RuntimeError("native client changed before pidfd signal")
        signal.pidfd_send_signal(fd, signal.SIGKILL)
    finally:
        os.close(fd)
    print(f"Closed established native TCP stream: client={pid}, mode={mode}, session={token}", flush=True)
    return pid, token


class Acceptance:
    def __init__(self, candidate, root, build_base, timeout):
        self.root = root
        # Never race a coordinator relinking target/debug/justrust. The private
        # snapshot also exercises quoted executable paths in rsync's -e parser.
        self.candidate = root / "candidate executable with 'quotes'"
        with candidate.open("rb") as source, self.candidate.open("xb") as target:
            shutil.copyfileobj(source, target)
        self.candidate.chmod(0o500)
        self.candidate_sha256 = digest_file(self.candidate)
        if self.candidate_sha256 != digest_file(candidate):
            raise RuntimeError("candidate changed while making the isolated executable snapshot")
        self.home = root / "isolated state with 'quotes'"
        self.home.mkdir(mode=0o700)
        self.project = root / "project with 'quotes'"
        self.project.mkdir()
        self.guard = root / "ssh guard"
        self.guard.mkdir()
        self.ssh_used = root / "forbidden-ssh-invoked"
        ssh = self.guard / "ssh"
        ssh.write_text("#!/bin/sh\n: > " + shlex.quote(str(self.ssh_used))
                       + "\nprintf 'native-only acceptance forbids SSH\\n' >&2\nexit 125\n")
        ssh.chmod(0o700)
        self.env = environment(self.home, self.guard, build_base)
        self.timeout = timeout
        self.run_ids = []
        self.seen = set()
        self.nonce = root.name.replace("'", "")

    def run(self, *args, expected=0, quiet=False):
        print("+ candidate " + shlex.join(args), flush=True)
        result = subprocess.run([str(self.candidate), *args], cwd=self.project,
                                env=self.env, text=True, errors="replace",
                                capture_output=True, timeout=self.timeout)
        if not quiet or result.returncode != expected:
            print(result.stdout, end="", flush=True)
            print(result.stderr, end="", file=sys.stderr, flush=True)
        if result.returncode != expected:
            raise RuntimeError(f"candidate command exited {result.returncode}, expected {expected}")
        if self.ssh_used.exists():
            raise RuntimeError("native-only workflow attempted an SSH process")
        return result

    def record(self, subcommand, expected=0):
        result = self.run("runs", "--here", "--json", "-n", "100", quiet=True)
        rows = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
        run_id = select_record(rows, self.seen, self.project, subcommand, expected)
        self.seen.update(row["id"] for row in rows)
        self.run_ids.append(run_id)
        print(f"Recorded hosted {subcommand}: {run_id}", flush=True)
        return run_id

    def build(self, subcommand, *args, expected=0):
        result = self.run(subcommand, "--remote", *args, expected=expected)
        self.record(subcommand, expected)
        return result

    def remote(self, script, expected=0):
        return self.run("remote", "ssh", script, expected=expected)

    def write_project(self, revision):
        source = self.project / "src"
        source.mkdir(exist_ok=True)
        payload = bytes([0, 255, revision, 39, 34, 10])
        (source / "quoted ' payload.bin").write_bytes(payload)
        (self.project / "Cargo.toml").write_text(
            '[package]\nname = "native-acceptance-workload"\nversion = "0.0.0"\nedition = "2021"\n\n[workspace]\n')
        (self.project / ".gitignore").write_text("/target/\n/.justrust/\n/ignored-payload\n")
        (self.project / "ignored-payload").write_text("must not be uploaded")
        (source / "main.rs").write_text('fn main() { println!("native acceptance binary"); }\n')
        marker = json.dumps(str(self.project / "target" / "native-invocations"), ensure_ascii=False)
        nonce = json.dumps(self.nonce)
        (source / "lib.rs").write_text(f'''#[cfg(test)]
mod tests {{
    #[test]
    fn mirror_smoke() {{
        assert_eq!(include_bytes!("quoted ' payload.bin"), &{list(payload)!r});
    }}
    #[test]
    fn resume_once() {{
        use std::io::Write;
        let marker = std::path::Path::new({marker});
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(marker).unwrap();
        writeln!(f, "{{}}:{{}}", {nonce}, std::process::id()).unwrap();
        f.sync_all().unwrap();
        println!("NATIVE_ACCEPTANCE_BEGIN {{}}", {nonce});
        std::io::stdout().flush().unwrap();
        std::thread::sleep(std::time::Duration::from_secs(16));
        println!("NATIVE_ACCEPTANCE_END {{}}", {nonce});
    }}
    #[test]
    fn failing_status() {{
        eprintln!("NATIVE_ACCEPTANCE_TEST_STDERR");
        panic!("NATIVE_ACCEPTANCE_EXPECTED_FAILURE");
    }}
}}
''')
        bulk = self.project / "bulk"
        bulk.mkdir(exist_ok=True)
        for index in range(305):
            (bulk / f"file-{index:03}.txt").write_text(f"revision {revision}\n")

    def resume(self):
        begin = "NATIVE_ACCEPTANCE_BEGIN " + self.nonce
        end = "NATIVE_ACCEPTANCE_END " + self.nonce
        event = threading.Event()
        output = [[], []]
        command = [str(self.candidate), "test", "--remote", "--lib", "resume_once", "--", "--nocapture"]
        process = subprocess.Popen(command, cwd=self.project, env=self.env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   text=True, errors="replace", bufsize=1)

        def consume(stream, target, destination):
            for line in stream:
                target.append(line)
                print(line, end="", file=destination, flush=True)
                if begin in line:
                    event.set()

        readers = [threading.Thread(target=consume, args=(process.stdout, output[0], sys.stdout), daemon=True),
                   threading.Thread(target=consume, args=(process.stderr, output[1], sys.stderr), daemon=True)]
        for reader in readers:
            reader.start()
        try:
            deadline = time.monotonic() + self.timeout
            while not event.wait(0.1):
                if process.poll() is not None:
                    raise RuntimeError("hosted command exited before the resume fixture started")
                if time.monotonic() >= deadline:
                    raise RuntimeError("resume fixture never started")
            original_pid, token = drop_session(self.home)
            deadline = time.monotonic() + 12
            attached = False
            while time.monotonic() < deadline and process.poll() is None:
                if any(pid != original_pid and mode == "attach" and session == token
                       for pid, _, mode, session in native_sessions(self.home)):
                    attached = True
                    break
                time.sleep(0.02)
            if not attached:
                raise RuntimeError("did not observe reattachment to the original durable session")
            if process.wait(timeout=self.timeout) != 0:
                raise RuntimeError("interrupted hosted test failed instead of resuming")
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            for reader in readers:
                reader.join(timeout=5)
        text = "".join(output[0])
        if text.count(begin) != 1 or text.count(end) != 1:
            raise RuntimeError("test output was lost or replayed after socket drop")
        self.record("test")
        receipt = self.remote("cat -- " + shlex.quote(str(self.project / "target/native-invocations")))
        rows = receipt.stdout.splitlines()
        if len(rows) != 1 or not rows[0].startswith(self.nonce + ":"):
            raise RuntimeError("original test executed more than once")
        if self.ssh_used.exists():
            raise RuntimeError("recovery attempted SSH fallback")

    def execute(self):
        self.run("--version")
        self.run("remote", "use", "hosted")
        self.run("remote", "up")
        state = json.loads((self.home / "remote/hosted/state.json").read_text())
        if not state.get("native"):
            raise RuntimeError("control plane did not select native transport")
        subprocess.run(["git", "init", "-q", str(self.project)], check=True)
        self.write_project(1)
        stale = self.project / "stale ' source.txt"
        stale.write_text("must be deleted on next bulk mirror")
        self.build("check")
        uploaded = self.remote(
            'p="$HOME/.justrust/remote/session-executables/' + self.candidate_sha256
            + '/justrust"; test -f "$p" && test ! -L "$p" && sha256sum "$p"')
        if uploaded.stdout.split()[0:1] != [self.candidate_sha256]:
            raise RuntimeError("remote pinned candidate bytes do not match the explicit snapshot")
        self.build("build")
        self.build("test", "--lib", "mirror_smoke", "--", "--nocapture")
        project = shlex.quote(str(self.project))
        self.remote(f"set -eu; cd {project}; mkdir -p target .justrust; printf keep > target/native-protected; printf keep > .justrust/native-protected")
        stale.unlink()
        self.write_project(2)
        self.build("test", "--lib", "mirror_smoke", "--", "--nocapture")
        self.remote(f"set -eu; cd {project}; test ! -e \"stale ' source.txt\"; test ! -e ignored-payload; test \"$(cat target/native-protected)\" = keep; test \"$(cat .justrust/native-protected)\" = keep; printf NATIVE_MIRROR_OK")
        self.resume()
        # Exercise the exact byte-stream client without control-plane readiness
        # diagnostics sharing stderr with the command under test.
        channels = self.run("__native", "client", json.dumps(state["native"]),
                            "printf NATIVE_STDOUT; printf NATIVE_STDERR >&2; exit 37", expected=37)
        if channels.stdout != "NATIVE_STDOUT" or channels.stderr != "NATIVE_STDERR":
            raise RuntimeError("native shell corrupted stdout/stderr channels")
        failure = self.build("test", "--lib", "failing_status", "--", "--nocapture", expected=101)
        if "NATIVE_ACCEPTANCE_TEST_STDERR" not in failure.stdout + failure.stderr:
            raise RuntimeError("failing hosted test lost its stderr")
        return {"candidate_sha256": self.candidate_sha256,
                "state": str(self.home), "run_ids": self.run_ids,
                "native_only": True, "socket_drop_resumed_once": True}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path, required=True,
                        help="explicit unpublished executable, never installed")
    parser.add_argument("--work-dir", type=Path, default=Path.home() / ".cache/justrust-native-acceptance")
    parser.add_argument("--build-base", help="explicit already-deployed control-plane URL")
    parser.add_argument("--timeout", type=float, default=900)
    args = parser.parse_args(argv)
    if sys.platform != "linux":
        parser.error("real socket/process fault injection currently requires Linux /proc")
    candidate = args.candidate.resolve(strict=True)
    if not candidate.is_file() or not os.access(candidate, os.X_OK):
        parser.error("candidate must be an existing executable")
    args.work_dir.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="native 'acceptance'-", dir=args.work_dir.resolve()))
    print(f"Isolated fixture and automatic build records retained at {root}", flush=True)
    acceptance = Acceptance(candidate, root, args.build_base, args.timeout)
    failed = True
    try:
        receipt = acceptance.execute()
        failed = False
    finally:
        # Only this fixture's daemon/backend is stopped. Never terminate the
        # hosted machine, release an account lease, or touch normal client state.
        try:
            acceptance.run("remote", "use", "off")
        except Exception as error:
            if not failed:
                raise
            print(f"Isolated daemon cleanup also failed: {error}", file=sys.stderr)
    print(json.dumps(receipt), flush=True)


if __name__ == "__main__":
    main()
