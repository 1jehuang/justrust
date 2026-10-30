#!/usr/bin/env python3
"""own_registry.py: `cargo blaze search`, `owner`, `yank`, `login` and `logout` against cargo's.
cargo is the oracle; blaze never runs it outside this harness.

usage: own_registry.py [--blaze BIN] [--out FILE] [--no-network]

A made-up registry is served on 127.0.0.1 (a sparse index's `config.json` and the web API,
with fixed `Date` and `Server` headers), as `mock` (with an API), `auth` (`auth-required`) and
`noapi` (no API) in a `$CARGO_HOME/config.toml`. Each case runs `cargo ARGS` and
`BIN cargo ARGS` (BIN: `ra-jit`, default `target/release/ra-jit`), each with a `$CARGO_HOME` of
its own made from the same files, and compares the exit status, standard output and error (the
homes and the port named alike), every request the registry got (method, path, `Authorization`,
`Content-Type`, `Accept` and body), the bytes and mode of the credentials files left, and what a
credential provider of the home's (`provider.py`) was asked.
A few cases ask crates.io itself, read-only and with no token (`--no-network` leaves them out).
"""
import argparse, http.server, json, os, shutil, subprocess, sys, tempfile, threading, tomllib, urllib.parse

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

REQUESTS = []
LOCK = threading.Lock()

CRATES = [
    {"name": "foo", "max_version": "1.2.3", "description": "A foo for every foo"},
    {"name": "foo-bar", "max_version": "0.1.0", "description": "Line one\nline two of the foo-bar"},
    {"name": "foolish", "max_version": "2.0.0", "description": None},
    {"name": "long-foo", "max_version": "0.0.1",
     "description": "A description that goes on and on, well past the eighty characters cargo leaves it, so that it is cut short with an ellipsis"},
]
OWNERS = [
    {"id": 1, "login": "alice", "name": "Alice A", "email": "alice@example.com"},
    {"id": 2, "login": "bob", "name": "Bob B", "email": None},
    {"id": 3, "login": "carol", "name": None, "email": "carol@example.com"},
    {"id": 4, "login": "github:org:team", "name": None, "email": None},
]


class Registry(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def version_string(self):
        return "mock"

    def date_time_string(self, timestamp=None):
        return "Mon, 26 Oct 2026 00:00:00 GMT"

    def log_message(self, *args):
        pass

    def answer(self, code, body, kind="application/json"):
        data = body.encode() if isinstance(body, str) else json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def handle_any(self):
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length).decode() if length else ""
        url = urllib.parse.urlsplit(self.path)
        port = self.server.server_address[1]
        parts = url.path.strip("/").split("/")
        if parts[-1] == "config.json":
            config = {"dl": f"http://127.0.0.1:{port}/dl"}
            if parts[0] != "noapi":
                config["api"] = f"http://127.0.0.1:{port}"
            if parts[0] == "auth":
                config["auth-required"] = True
            return self.answer(200, config)
        with LOCK:
            REQUESTS.append([self.command, self.path, self.headers.get("Authorization"),
                             self.headers.get("Content-Type"), self.headers.get("Accept"), body])
        if parts[:2] != ["api", "v1"]:
            return self.answer(404, "not here", "text/plain")
        rest = parts[2:]
        if rest == ["crates"]:
            query = urllib.parse.parse_qs(url.query)
            q, per_page = query.get("q", [""])[0], int(query.get("per_page", ["10"])[0])
            if q == "many":
                found = [{"name": f"many-{i}", "max_version": "1.0.0", "description": "one of many"} for i in range(300)]
            else:
                found = [c for c in CRATES if q and q in c["name"]]
            return self.answer(200, {"crates": found[:per_page], "meta": {"total": len(found)}})
        krate = rest[1] if len(rest) > 1 else ""
        if krate == "missing":
            return self.answer(404, {"errors": [{"detail": f"crate `{krate}` does not exist"}]})
        if krate == "broken":
            return self.answer(500, "down for maintenance", "text/plain")
        if len(rest) == 3 and rest[2] == "owners":
            if self.command == "GET":
                return self.answer(200, {"users": OWNERS})
            users = json.loads(body or "{}").get("users", [])
            verb = "invited" if self.command == "PUT" else "removed"
            return self.answer(200, {"ok": True, "msg": f"{', '.join(users)} {verb} for crate {krate}"})
        if len(rest) == 4 and rest[3] in ("yank", "unyank"):
            return self.answer(200, {"ok": True})
        return self.answer(404, {"errors": [{"detail": "Not Found"}]})

    do_GET = do_PUT = do_DELETE = handle_any


PACKAGE = "[package]\nname = \"mypkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"
VIRTUAL = "[workspace]\nmembers = []\n"


def home(port, extra=""):
    return (f"[registries.mock]\nindex = \"sparse+http://127.0.0.1:{port}/index/\"\n\n"
            f"[registries.auth]\nindex = \"sparse+http://127.0.0.1:{port}/auth/\"\n\n"
            f"[registries.noapi]\nindex = \"sparse+http://127.0.0.1:{port}/noapi/\"\n" + extra)


CREDENTIALS = """# kept by hand
token = "old-top-level"

[registries.other]
token = "other-token"

[registries.mock]
credential-provider = "cargo:token"
token = "mock-token"
"""

# A credential provider in each $CARGO_HOME, as a credential process (`--cargo-plugin`) or as
# `cargo:token-from-stdout`'s command (`stdout`): it keeps what it was given in `provider.log`
# and answers as its first argument says.
PROVIDER = r'''#!/usr/bin/env python3
import json, os, sys
log = os.path.join(os.path.dirname(os.path.abspath(__file__)), "provider.log")
def keep(line):
    with open(log, "a") as f:
        f.write(line + "\n")
if sys.argv[1:2] == ["stdout"]:
    keep(json.dumps({"argv": sys.argv[2:], "index": os.environ.get("CARGO_REGISTRY_INDEX_URL"),
                     "name": os.environ.get("CARGO_REGISTRY_NAME_OPT")}))
    mode = sys.argv[2] if len(sys.argv) > 2 else ""
    if mode == "fail":
        sys.exit(3)
    print("line-one\nline-two" if mode == "two" else "stdout-token")
    sys.exit(0)
keep(json.dumps({"argv": sys.argv[1:]}))
line = sys.stdin.readline
if os.path.basename(__file__) == "provider-v2.py":
    print(json.dumps({"v": [2]}), flush=True)
    sys.exit(0)
print(json.dumps({"v": [1]}), flush=True)
request = line()
keep(request.rstrip("\n"))
request = json.loads(request)
mode = (request.get("args") or ["token"])[0]
answers = {
    "not-found": {"Err": {"kind": "not-found"}},
    "url": {"Err": {"kind": "url-not-supported"}},
    "unsupported": {"Err": {"kind": "operation-not-supported"}},
    "other": {"Err": {"kind": "other", "message": "the vault is locked", "caused-by": ["no key", "none at all"]}},
    "never": {"Ok": {"kind": "get", "token": "never-token", "cache": "never", "operation_independent": True}},
    "expires": {"Ok": {"kind": "get", "token": "expiring-token", "cache": "expires", "expiration": 4102444800,
                       "operation_independent": False}},
}
if mode == "garbage":
    print("no json", flush=True)
elif mode in answers:
    print(json.dumps(answers[mode]), flush=True)
elif request["kind"] == "get":
    print(json.dumps({"Ok": {"kind": "get", "token": "process-token", "cache": "session",
                             "operation_independent": True}}), flush=True)
else:
    print(json.dumps({"Ok": {"kind": request["kind"]}}), flush=True)
sys.stdin.read()
sys.exit(3 if mode == "exit" else 0)
'''


def providers(*listed):
    return "\n[registry]\nglobal-credential-providers = [" + ", ".join(json.dumps(p) for p in listed) + "]\n"


PROC = "@HOME@/provider.py"
STDOUT = "cargo:token-from-stdout @HOME@/provider.py stdout"
M = ["--registry", "mock"]
# (name, arguments, standard input, where it runs, files of $CARGO_HOME, extra
# config.toml, environment, network). The credentials of a case in UNORDERED are compared as
# TOML, not as text: cargo writes the merged `[registries]` of a login from a `HashMap`, in an
# order that changes from run to run.
CASES = [
    ("search", ["search", *M, "foo"], None, "plain", {}, "", {}, False),
    ("search, a limit", ["search", *M, "foo", "--limit", "2"], None, "plain", {}, "", {}, False),
    ("search, colored", ["search", *M, "foo", "--color", "always"], None, "plain", {}, "", {}, False),
    ("search, nothing found", ["search", *M, "none"], None, "plain", {}, "", {}, False),
    ("search, more than 100", ["search", *M, "many", "--limit", "150"], None, "plain", {}, "", {}, False),
    ("search, two words", ["search", *M, "foo", "bar"], None, "plain", {}, "", {}, False),
    ("search, no query", ["search", *M], None, "plain", {}, "", {}, False),
    ("search, quiet", ["search", "-q", *M, "foo"], None, "plain", {}, "", {}, False),
    ("search by --index", ["search", "--index", "sparse+http://127.0.0.1:PORT/index/", "foo"], None, "plain", {}, "", {}, False),
    ("search, a registry with no API", ["search", "--registry", "noapi", "foo"], None, "plain", {}, "", {}, False),
    ("search, a registry not configured", ["search", "--registry", "nope", "foo"], None, "plain", {}, "", {}, False),
    ("search, a name that is no name", ["search", "--registry", "a b", "foo"], None, "plain", {}, "", {}, False),
    ("search, a limit that is no number", ["search", *M, "--limit", "abc", "foo"], None, "plain", {}, "", {}, False),
    ("search, --registry and --index", ["search", *M, "--index", "x", "foo"], None, "plain", {}, "", {}, False),
    ("search, an unknown flag", ["search", "--frob"], None, "plain", {}, "", {}, False),
    ("search, a flag like --limit", ["search", "--limt", "3", "x"], None, "plain", {}, "", {}, False),
    ("search, a flag like none", ["search", "--zzzz"], None, "plain", {}, "", {}, False),
    ("owner, a flag like --token", ["owner", "--tokn", "x"], None, "plain", {}, "", {}, False),
    ("owner, --registry and --index", ["owner", *M, "--index", "x"], None, "plain", {}, "", {}, False),
    ("yank, --registry and --index", ["yank", *M, "--index", "x", "krate"], None, "plain", {}, "", {}, False),
    ("logout, an unknown flag", ["logout", "--zzzz"], None, "plain", {}, "", {}, False),
    ("search --help", ["search", "--help"], None, "plain", {}, "", {}, False),
    ("search, registry.default", ["search", "foo"], None, "plain", {}, "\n[registry]\ndefault = \"mock\"\n", {}, False),
    ("search, crates.io replaced by a registry source",
     ["search", "foo"], None, "plain", {},
     "\n[source.crates-io]\nreplace-with = \"mirror\"\n\n[source.mirror]\nregistry = \"sparse+http://127.0.0.1:PORT/index/\"\n", {}, False),
    ("search, crates.io replaced by a named registry",
     ["search", "foo"], None, "plain", {}, "\n[source.crates-io]\nreplace-with = \"mock\"\n", {}, False),
    ("owner, no token", ["owner", *M, "--list", "krate"], None, "plain", {}, "", {}, False),
    ("owner --list", ["owner", *M, "--list", "--token", "t0", "krate"], None, "plain", {}, "", {}, False),
    ("owner --add", ["owner", *M, "-a", "alice", "--add", "bob", "--token", "t0", "krate"], None, "plain", {}, "", {}, False),
    ("owner --remove", ["owner", *M, "-r", "alice", "--token", "t0", "krate"], None, "plain", {}, "", {}, False),
    ("owner, add, remove and list", ["owner", *M, "-a", "x", "-r", "y", "-l", "--token", "t0", "krate"], None, "plain", {}, "", {}, False),
    ("owner, a crate the registry has not", ["owner", *M, "--list", "--token", "t0", "missing"], None, "plain", {}, "", {}, False),
    ("owner, a registry that is down", ["owner", *M, "--list", "--token", "t0", "broken"], None, "plain", {}, "", {}, False),
    ("owner, the package here, its token kept", ["owner", *M, "--list"], None, "pkg", {"credentials.toml": CREDENTIALS}, "", {}, False),
    ("owner, a virtual manifest", ["owner", *M, "--list", "--token", "t0"], None, "virt", {}, "", {}, False),
    ("owner, --index with no token", ["owner", "--index", "sparse+http://127.0.0.1:PORT/index/", "-l", "krate"], None, "plain", {}, "", {}, False),
    ("owner, --index and --token", ["owner", "--index", "sparse+http://127.0.0.1:PORT/index/", "-l", "--token", "t0", "krate"], None, "plain", {}, "", {}, False),
    ("owner, the token from the environment", ["owner", *M, "-l", "krate"], None, "plain", {}, "", {"CARGO_REGISTRIES_MOCK_TOKEN": "from-env"}, False),
    ("owner, a token that cannot be sent", ["owner", *M, "-l", "krate"], None, "plain", {}, "", {"CARGO_REGISTRIES_MOCK_TOKEN": "caf\u00e9"}, False),
    ("owner, two crates", ["owner", "a", "b"], None, "plain", {}, "", {}, False),
    ("owner, cargo:token listed", ["owner", *M, "-l", "krate"], None, "plain", {"credentials.toml": CREDENTIALS},
     "\n[registry]\nglobal-credential-providers = [\"cargo:token\"]\n", {}, False),
    ("owner, an --index that is a registry", ["owner", "--index", "sparse+http://127.0.0.1:PORT/index/", "-l", "--token", "t0", "krate"], None, "plain", {"credentials.toml": CREDENTIALS}, "", {}, False),
    ("yank", ["yank", *M, "--version", "1.0.0", "--token", "t0", "krate"], None, "plain", {}, "", {}, False),
    ("yank --undo", ["yank", *M, "krate@1.0.0", "--undo", "--token", "t0"], None, "plain", {}, "", {}, False),
    ("yank, no version", ["yank", *M, "krate"], None, "plain", {}, "", {}, False),
    ("yank, two versions", ["yank", *M, "krate@1.0.0", "--version", "1.0.0"], None, "plain", {}, "", {}, False),
    ("yank, no crate name", ["yank", *M, "@1.0.0"], None, "plain", {}, "", {}, False),
    ("yank, a v in the version", ["yank", *M, "krate", "--version", "v1.0.0"], None, "plain", {}, "", {}, False),
    ("yank, a version that is none", ["yank", *M, "krate", "--vers", "x"], None, "plain", {}, "", {}, False),
    ("yank, the package here", ["yank", "--vers", "0.1.0", *M, "--token", "t0"], None, "pkg", {}, "", {}, False),
    ("yank, no token", ["yank", *M, "krate@1.0.0"], None, "plain", {}, "", {}, False),
    ("yank, quiet", ["yank", "-q", *M, "krate@1.0.0", "--token", "t0"], None, "plain", {}, "", {}, False),
    ("yank, a crate the registry has not", ["yank", *M, "missing@1.0.0", "--token", "t0"], None, "plain", {}, "", {}, False),
    ("login", ["login", *M], "tok123\n", "plain", {}, "", {}, False),
    ("login, pasted with cargo login", ["login", *M], "cargo login tok456\n", "plain", {}, "", {}, False),
    ("login, the token as an argument", ["login", *M, "tok789"], None, "plain", {}, "", {}, False),
    ("login, an empty token", ["login", *M], "", "plain", {}, "", {}, False),
    ("login, a token that cannot be sent", ["login", *M], "t\u00f6k\n", "plain", {}, "", {}, False),
    ("login, over kept credentials", ["login", *M], "new-token\n", "plain", {"credentials.toml": CREDENTIALS}, "", {}, False),
    ("login, the old credentials file", ["login", *M], "new-token\n", "plain", {"credentials": "[registries.mock]\ntoken = \"x\"\n"}, "", {}, False),
    ("login, --index", ["login", "--index", "x"], None, "plain", {}, "", {}, False),
    ("login, provider arguments", ["login", *M, "--", "a", "b"], "tok\n", "plain", {}, "", {}, False),
    ("login, an auth-required registry", ["login", "--registry", "auth"], "tok\n", "plain", {}, "", {}, False),
    ("logout, not logged in", ["logout", *M], None, "plain", {}, "", {}, False),
    ("logout", ["logout", *M], None, "plain", {"credentials.toml": CREDENTIALS}, "", {}, False),
    ("logout, quiet", ["logout", "-q", *M], None, "plain", {"credentials.toml": CREDENTIALS}, "", {}, False),
    ("logout of crates.io", ["logout"], None, "plain", {"credentials.toml": CREDENTIALS}, "", {}, False),
    ("logout of crates.io, not logged in", ["logout"], None, "plain", {}, "", {}, False),
    ("logout, a word", ["logout", "x"], None, "plain", {}, "", {}, False),
    ("logout, verbose", ["logout", "-v", *M], None, "plain", {"credentials.toml": CREDENTIALS}, "", {}, False),
    ("owner, a credential process", ["owner", *M, "-l", "krate"], None, "plain", {}, providers(PROC), {}, False),
    ("owner, a credential process's arguments", ["owner", *M, "-l", "krate"], None, "plain", {},
     providers(PROC + " token extra"), {}, False),
    ("owner, verbose, a credential process", ["owner", "-v", *M, "-l", "krate"], None, "plain", {}, providers(PROC), {}, False),
    ("owner, a credential process with no token", ["owner", *M, "-l", "krate"], None, "plain", {},
     providers(PROC + " not-found"), {}, False),
    ("owner, a credential process not for the registry, then cargo:token", ["owner", *M, "-l", "krate"], None, "plain",
     {"credentials.toml": CREDENTIALS}, providers("cargo:token", PROC + " url"), {}, False),
    ("owner, a credential process that fails", ["owner", *M, "-l", "krate"], None, "plain", {},
     providers(PROC + " other"), {}, False),
    ("owner, a credential process that does not do owners", ["owner", *M, "-l", "krate"], None, "plain", {},
     providers(PROC + " unsupported"), {}, False),
    ("owner, a credential process of protocol 2", ["owner", *M, "-l", "krate"], None, "plain", {},
     providers("@HOME@/provider-v2.py"), {}, False),
    ("owner, a credential process that answers no JSON", ["owner", *M, "-l", "krate"], None, "plain", {},
     providers(PROC + " garbage"), {}, False),
    ("owner, a credential process that exits with 3", ["owner", *M, "-l", "krate"], None, "plain", {},
     providers(PROC + " exit"), {}, False),
    ("owner, a credential process that is not there", ["owner", *M, "-l", "krate"], None, "plain", {},
     providers("@HOME@/nothing"), {}, False),
    ("owner, a token never kept", ["owner", *M, "-a", "x", "-r", "y", "-l", "krate"], None, "plain", {},
     providers(PROC + " never"), {}, False),
    ("owner, a token that expires", ["owner", *M, "-l", "krate"], None, "plain", {}, providers(PROC + " expires"), {}, False),
    ("owner, a credential-alias", ["owner", *M, "-l", "krate"], None, "plain", {},
     "\n[credential-alias]\nmine = \"@HOME@/provider.py token aliased\"\n" + providers("mine"), {}, False),
    ("owner, the registry's own credential process", ["owner", *M, "-l", "krate"], None, "plain",
     {"credentials.toml": CREDENTIALS.replace('credential-provider = "cargo:token"\n', "")}, "",
     {"CARGO_REGISTRIES_MOCK_CREDENTIAL_PROVIDER": PROC}, False),
    ("search, an auth-required registry, a credential process", ["search", "--registry", "auth", "foo"], None, "plain", {},
     providers(PROC), {}, False),
    ("yank, a credential process", ["yank", *M, "krate@1.0.0"], None, "plain", {}, providers(PROC), {}, False),
    ("login, a credential process", ["login", *M], "tok\n", "plain", {}, providers(PROC), {}, False),
    ("login, a credential process, its arguments", ["login", *M, "--", "a", "b"], None, "plain", {},
     providers(PROC), {}, False),
    ("logout, a credential process", ["logout", *M], None, "plain", {}, providers(PROC), {}, False),
    ("logout, a credential process with no token", ["logout", *M], None, "plain", {}, providers(PROC + " not-found"), {}, False),
    ("owner, cargo:token-from-stdout", ["owner", *M, "-l", "krate"], None, "plain", {},
     providers(STDOUT + " {index_url}"), {}, False),
    ("owner, cargo:token-from-stdout of two lines", ["owner", *M, "-l", "krate"], None, "plain", {},
     providers(STDOUT + " two"), {}, False),
    ("owner, cargo:token-from-stdout that fails", ["owner", *M, "-l", "krate"], None, "plain", {},
     providers(STDOUT + " fail"), {}, False),
    ("owner, cargo:token-from-stdout with no command", ["owner", *M, "-l", "krate"], None, "plain", {},
     providers("cargo:token-from-stdout"), {}, False),
    ("login, cargo:token-from-stdout", ["login", *M], "tok\n", "plain", {}, providers(STDOUT), {}, False),
    ("owner, cargo:paseto", ["owner", *M, "-l", "krate"], None, "plain", {}, providers("cargo:paseto"), {}, False),
    ("owner, the built-in providers of other platforms", ["owner", *M, "-l", "krate"], None, "plain", {},
     providers("cargo:macos-keychain", "cargo:wincred"), {}, False),
    ("search crates.io", ["search", "serde_json", "--limit", "2"], None, "plain", {}, "", {}, True),
    ("owner on crates.io, no token", ["owner", "--list", "serde"], None, "plain", {}, "", {}, True),
    ("owner on crates.io, registry.default set", ["owner", "--registry", "crates-io", "-l", "serde"], None, "plain", {},
     "\n[registry]\ndefault = \"mock\"\n", {}, True),
    ("login to crates.io", ["login"], "cio-token\n", "plain", {}, "", {}, True),
]


UNORDERED = {"login, over kept credentials"}


def credentials(top):
    out = {}
    for name in ("credentials", "credentials.toml", "provider.log"):
        path = os.path.join(top, name)
        if os.path.exists(path):
            with open(path, "rb") as f:
                out[name] = (f.read().decode(), oct(os.stat(path).st_mode & 0o777))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--blaze", default=os.path.join(ROOT, "target", "release", "ra-jit"))
    ap.add_argument("--out")
    ap.add_argument("--no-network", action="store_true")
    opts = ap.parse_args()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Registry)
    port = server.server_address[1]
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = {k: v for k, v in os.environ.items() if not k.startswith("CARGO_")}
    work = tempfile.mkdtemp(prefix="own-registry-")
    for name, text in (("pkg", PACKAGE), ("virt", VIRTUAL)):
        os.makedirs(os.path.join(work, name, "src"))
        with open(os.path.join(work, name, "Cargo.toml"), "w") as f:
            f.write(text)
        with open(os.path.join(work, name, "src", "lib.rs"), "w") as f:
            f.write("\n")
    os.makedirs(os.path.join(work, "plain"))
    cases = []
    for name, args, stdin, where, files, extra, env, network in CASES:
        if network and opts.no_network:
            continue
        args = [a.replace("PORT", str(port)) for a in args]
        outs = {}
        for who in ("cargo", "blaze"):
            top = os.path.join(work, f"home-{who}")
            shutil.rmtree(top, ignore_errors=True)
            os.makedirs(top)
            with open(os.path.join(top, "config.toml"), "w") as f:
                f.write(home(port, extra.replace("PORT", str(port)).replace("@HOME@", top)))
            for file, text in files.items():
                with open(os.path.join(top, file), "w") as f:
                    f.write(text)
            for file in ("provider.py", "provider-v2.py"):
                with open(os.path.join(top, file), "w") as f:
                    f.write(PROVIDER)
                os.chmod(os.path.join(top, file), 0o755)
            case_env = {k: v.replace("@HOME@", top) for k, v in env.items()}
            with LOCK:
                REQUESTS.clear()
            argv = (["cargo"] if who == "cargo" else [opts.blaze, "cargo"]) + args
            done = subprocess.run(argv, cwd=os.path.join(work, where), input=stdin or "", capture_output=True,
                                  text=True, env={**base, "CARGO_HOME": top, **case_env})
            name_home = lambda text: text.replace(top, "@home").replace(str(port), "PORT")
            with LOCK:
                requests = [[name_home(str(x)) for x in r] for r in REQUESTS]
            kept = {k: (name_home(v[0]), v[1]) for k, v in credentials(top).items()}
            if name in UNORDERED:
                kept = {k: (tomllib.loads(v[0]), v[1]) for k, v in kept.items()}
            outs[who] = (done.returncode, name_home(done.stdout), name_home(done.stderr), requests, kept)
        a, b = outs["cargo"], outs["blaze"]
        why = None
        for n, what in enumerate(("exit status", "stdout", "stderr", "requests", "credentials")):
            if a[n] != b[n]:
                why = f"{what}: cargo {str(a[n])[:600]!r}, blaze {str(b[n])[:600]!r}"
                break
        verdict = "equal" if why is None else "different"
        cases.append({"case": name, "verdict": verdict, "why": why})
        print(f"{verdict:10} {name}" + (f"  ({why})" if why else ""), flush=True)
    server.shutdown()
    shutil.rmtree(work)
    equal = sum(1 for c in cases if c["verdict"] == "equal")
    print(f"\n{equal}/{len(cases)} equal")
    if opts.out:
        with open(opts.out, "w") as f:
            json.dump(cases, f, indent=1)
    return 0 if equal == len(cases) else 1


if __name__ == "__main__":
    sys.exit(main())
