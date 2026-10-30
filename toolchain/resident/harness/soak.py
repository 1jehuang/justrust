#!/usr/bin/env python3
"""soak.py: one daemon, asked by `cargo ra-jit` and by the direct client, for hours.

    soak.py [--seed N] [--steps N] [--hours H] [--work DIR] [--resume DIR]
            [--projects a,b,...] [--worktrees N] [--sample RATE] [--stop] [--quiet-daemon]
    soak.py --report DIR

Each project (ripgrep, fd, bat, axum and `soak/`, a workspace with a build script and proc
macros) is copied into the work directory, committed, and checked out into several git
worktrees. Every step is drawn from the seed and the number of the step alone:
  edit       a body, a signature, a type or a constant changes; a function or a file is added
             or removed; the code is broken, and mended by the next edit
  manifest   a path dependency comes or goes, the version changes, a profile of the package
             changes, a dependency is locked at another version
  macros     the build script, its data or a proc macro changes (the `soak/` workspace)
  git        commit, checkout of another commit, a stash and its pop, a rebase onto a commit
             that touches many files
  worktree   a worktree is added, or removed and later added again at the same path
  burst      dozens of clients at once, over every worktree
  cancel     clients interrupted or killed in the middle of a request
  kill       the daemon or one of its workers killed, alone or under a burst
  collect    `ra-jit cache gc` and `ra-jit cache verify` under a burst
After a step, the worktrees it touched are built by `cargo ra-jit` (run, test or build, with
or without the feature) or by `ra-jit sync` to the worker's own socket, and at the sampled
rate by plain cargo into a target directory of its own: what the program prints and how it
ends, or the result of every test, has to be the same. A difference is asked for again
before it counts, and one that goes away is counted as flaky.

`metrics.jsonl` in the work directory has a line a step: the resident memory, the open files
and the threads of the daemon and of every worker, the size of the store, and how long each
request took and who built it. A failure prints the seed and the step, and keeps what both
sides said in `failures/STEP`. `--resume` goes on from the last whole step of a work
directory; `--report` prints what a work directory measured.
"""
import argparse, collections, concurrent.futures, glob, json, os, random, re, shutil, signal, subprocess, sys, tempfile, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import cargo_ra_jit
from cargo_ra_jit import RA_JIT, PROJECTS, base_env, how_built, program_output, test_results, difference, fetch

BEGIN, END = "// soak:begin", "// soak:end"
MODEL = "soak-model.json"
REQUEST_SECS = 1800
ODD = ("link failed", "did not answer", "worker ended", "panicked")
WARM_SECS = 600
GIT = ["git", "-c", "user.name=ra-jit", "-c", "user.email=ra-jit@localhost", "-c", "advice.detachedHead=false", "-c", "commit.gpgsign=false"]

# Each project: where it is built, the package the feature and the dependency are added to,
# and its sites, each a file and the line of a function after which the soak's call goes.
# The first site is of the program: there the call ends the process when `SOAK_EXIT` is set.
SOAKED = {
    "ripgrep": dict(manifest="Cargo.toml", package="ripgrep", sites=[
        ("crates/core/main.rs", "fn main() -> ExitCode {"),
        ("crates/printer/src/standard.rs", "pub fn new() -> StandardBuilder {")]),
    "fd": dict(manifest="Cargo.toml", package="fd-find", sites=[
        ("src/main.rs", "fn main() {"),
        ("src/filesystem.rs", "pub fn is_existing_directory(path: &Path) -> bool {")]),
    "bat": dict(manifest="Cargo.toml", package="bat", sites=[
        ("src/bin/bat/main.rs", "fn main() {"),
        ("src/pretty_printer.rs", "    pub fn new() -> Self {")]),
    "axum": dict(manifest="examples/hello-world/Cargo.toml", package="example-hello-world", exits=True, sites=[
        ("examples/hello-world/src/main.rs", '    let app = Router::new().route("/", get(handler));'),
        ("examples/testing/src/main.rs", "fn app() -> Router {"),
        ("axum/src/routing/mod.rs", "    pub fn new() -> Self {")]),
    "macros": dict(manifest="app/Cargo.toml", package="soak-app", build=["-p", "soak-app"], run=[], test=[], sites=[
        ("app/src/main.rs", "fn main() {"),
        ("core/src/lib.rs", "pub fn summary() -> String {")]),
}
FIELD_TYPES = ["u8", "u32", "u64", "i16", "String", "Vec<u8>", "Option<u32>", "(u32, u8)", "[u16; 3]", "bool", "char", "f64"]
PARAM_TYPES = ["u64", "u32", "u8", "i64"]
RETURN_TYPES = ["u64", "u32", "i64", "usize", "u8"]
SHAPES = 7
PROFILES = [None, "opt-level = 1", "debug = false", "overflow-checks = false", "debug-assertions = false", "codegen-units = 1"]


def literal(kind, v):
    """A value of the type `kind` made from the number `v`."""
    return {
        "u8": f"{v % 256}u8", "u32": f"{v}u32", "u64": f"{v}u64", "i16": f"{v % 30000 - 15000}i16",
        "String": f'String::from("s{v}")', "Vec<u8>": f"vec![{v % 256}u8; {v % 5}]",
        "Option<u32>": "None" if v % 3 == 0 else f"Some({v}u32)", "(u32, u8)": f"({v}u32, {v % 200}u8)",
        "[u16; 3]": f"[{v % 60000}u16; 3]", "bool": "true" if v % 2 else "false", "char": f"'{chr(97 + v % 26)}'",
        "f64": f"{v}.5f64", "i64": f"{v}i64",
    }[kind]


def function(name, spec):
    """The source of one function of a site: `spec` is its parameters, what it returns, the
    number its body is about and which of the shapes the body has."""
    k, count = spec["k"], spec["params"]
    params = ", ".join(f"a{i}: {PARAM_TYPES[(k + i) % 4]}" for i in range(count))
    mixed = "".join(f".wrapping_add(a{i} as u64)" for i in range(count))
    body = [
        f"let acc = {k}u64.wrapping_mul(31).wrapping_add(a);",
        f"let mut acc = {k}u64;\n        for i in 0..{k % 7 + 1}u64 {{\n            acc = acc.rotate_left(3) ^ i.wrapping_add(a);\n        }}",
        f"let acc = match {k}u64.wrapping_add(a) % 3 {{\n            0 => a.wrapping_mul(3),\n            1 => a ^ {k}u64,\n            _ => a.wrapping_add({k}u64),\n        }};",
        f"let step = |x: u64| x.wrapping_mul({k | 1}u64) ^ a;\n        let acc = (0..4u64).map(step).fold(0u64, u64::wrapping_add);",
        f'let acc = format!("{{}}-{{}}", {k}u64, a).len() as u64 + a % 10;',
        f"fn pick<T: Copy + Into<u64>>(it: T) -> u64 {{\n            it.into()\n        }}\n        let acc = pick({k % 250}u8).wrapping_add(pick({k}u32)).wrapping_add(a);",
        f"let each: Vec<Box<dyn Fn(u64) -> u64>> = vec![Box::new(|x| x.wrapping_add({k}u64)), Box::new(move |x| x ^ a)];\n        let acc = each.iter().fold(1u64, |sum, it| it(sum));",
    ][spec["shape"] % SHAPES]
    return f"    pub fn {name}({params}) -> {spec['ret']} {{\n        let a: u64 = 0u64{mixed};\n        {body}\n        acc as {spec['ret']}\n    }}\n"


def block(at, site, model):
    """The source the soak adds to the file of site `at`: a module made from the model, the
    modules that are files of their own, and a test whose result depends on the model."""
    name, first = f"soak_s{at}", at == 0
    out = [BEGIN]
    for mod in sorted(site["mods"]):
        out.append(f'#[path = "{name}_{mod}.rs"]\nmod {name}_{mod};')
    out.append("#[allow(dead_code, unused, unreachable_pub, missing_docs, missing_debug_implementations)]")
    out.append(f"mod {name} {{")
    for const, value in sorted(site["consts"].items()):
        out.append(f"    pub const {const}: u64 = {value};")
    fields = "".join(f"        pub {field}: {kind},\n" for field, kind, _ in site["fields"])
    out.append(f"    #[derive(Debug, Clone, PartialEq)]\n    pub struct State {{\n{fields}    }}")
    for fn_name, spec in sorted(site["fns"].items()):
        out.append(function(fn_name, spec).rstrip("\n"))
    if site.get("broken"):
        out.append('    pub fn broken() -> u32 {\n        "not a number"\n    }')
    values = "".join(f"            {field}: {literal(kind, v)},\n" for field, kind, v in site["fields"])
    add = lambda what: f"        sum = sum.wrapping_mul(1099511628211).wrapping_add({what});"
    body = [f"        let state = State {{\n{values}        }};", "        let mut sum = tag;"]
    body += [add(const) for const in sorted(site["consts"])]
    for fn_name, spec in sorted(site["fns"].items()):
        args = ", ".join(literal(PARAM_TYPES[(spec["k"] + i) % 4], spec["k"] + i) for i in range(spec["params"]))
        body.append(add(f"{fn_name}({args}) as u64"))
    body += [add(f"super::{name}_{mod}::value()") for mod in sorted(site["mods"])]
    body.append(add('format!("{:?}", state).bytes().fold(0u64, |sum, byte| sum.wrapping_mul(131).wrapping_add(byte as u64))'))
    if first:
        body.append(add('env!("CARGO_PKG_VERSION").bytes().fold(0u64, |sum, byte| sum.wrapping_mul(131).wrapping_add(byte as u64))'))
        body.append('        #[cfg(feature = "soak")]\n        {\n            sum = sum.wrapping_add(1);\n        }')
        if model.get("dep") is not None:
            body.append(add("soak_dep::value()"))
    out.append("    pub fn report(tag: u64) -> u64 {\n" + "\n".join(body) + "\n        sum\n    }")
    exits = '\n        if std::env::var_os("SOAK_EXIT").is_some() {\n            std::process::exit(0);\n        }' if first else ""
    out.append(f'    pub fn announce(tag: u64) {{\n        let sum = report(tag);\n        if std::env::var_os("SOAK_REPORT").is_some() {{\n            eprintln!("soak s{at} {{sum}}");\n        }}{exits}\n    }}')
    out.append("}")
    out.append(f'#[cfg(test)]\nmod {name}_tests {{\n    #[test]\n    fn soak_parity() {{\n        assert!(super::{name}::report(1) % 4 != 0, "the soak\'s number is a multiple of four");\n    }}\n}}')
    out.append(END)
    return "\n".join(out) + "\n"


def save(path, text, rng=None):
    """Writes the file the way one of several editors does: over the old one, by a rename,
    by a truncation and a later write, or twice in a row with something else first."""
    how = rng.randrange(4) if rng else 0
    if how == 1:
        tmp = f"{path}.soak-tmp"
        open(tmp, "w").write(text)
        os.replace(tmp, path)
        return
    with open(path, "w") as file:
        if how == 2:
            file.flush()
            time.sleep(0.005)
        if how == 3:
            file.write(text[: len(text) // 2])
            file.flush()
            file.seek(0)
            file.truncate()
        file.write(text)


class Tree:
    """A checkout of a project: the main one or a worktree of it."""

    def __init__(self, project, root):
        self.project, self.root, self.spec = project, root, SOAKED[project]

    def path(self, *parts):
        return os.path.join(self.root, *parts)

    def model(self):
        return json.load(open(self.path(MODEL)))

    def git(self, *args, check=True):
        ran = subprocess.run([*GIT, *args], cwd=self.root, capture_output=True, text=True)
        if check and ran.returncode:
            raise RuntimeError(f"git {' '.join(args)} in {self.root}: {ran.stderr.strip()[-400:]}")
        return ran

    def write(self, model, rng=None):
        """Makes the files of the checkout what `model` says."""
        save(self.path(MODEL), json.dumps(model, indent=1, sort_keys=True) + "\n")
        for at, ((file, anchor), site) in enumerate(zip(self.spec["sites"], model["sites"])):
            path = self.path(file)
            text = open(path).read()
            text = re.sub(rf"\n?{BEGIN}\n.*?{END}\n", "", text, flags=re.S)
            text = re.sub(r"^\s*soak_s\d+::announce\(\d+\);\n", "", text, flags=re.M)
            if anchor not in text:
                raise RuntimeError(f"{path} has no `{anchor}`")
            text = text.replace(anchor, f"{anchor}\n    soak_s{at}::announce({site['tag']});", 1)
            text = text.rstrip("\n") + "\n\n" + block(at, site, model)
            here = os.path.dirname(path)
            wanted = {f"soak_s{at}_{mod}.rs": f"pub fn value() -> u64 {{\n    {v}\n}}\n" for mod, v in site["mods"].items()}
            for old in glob.glob(os.path.join(here, f"soak_s{at}_*.rs")):
                if os.path.basename(old) not in wanted:
                    os.remove(old)
            for name, source in wanted.items():
                if not os.path.exists(os.path.join(here, name)) or open(os.path.join(here, name)).read() != source:
                    save(os.path.join(here, name), source, rng)
            if open(path).read() != text:
                save(path, text, rng)
        self.write_manifests(model, rng)

    def write_manifests(self, model, rng):
        manifest = self.path(self.spec["manifest"])
        text = open(manifest).read()
        new = re.sub(r"^.*# soak\n", "", text, flags=re.M)
        dep_dir = os.path.join(os.path.dirname(manifest), "soak-dep")
        if model.get("dep") is None:
            shutil.rmtree(dep_dir, ignore_errors=True)
        else:
            new = new.replace("[dependencies]\n", '[dependencies]\nsoak-dep = { path = "soak-dep" } # soak\n', 1)
            os.makedirs(os.path.join(dep_dir, "src"), exist_ok=True)
            wanted = {"Cargo.toml": '[package]\nname = "soak-dep"\nversion = "0.1.0"\nedition = "2021"\n\n[workspace]\n' if self.spec.get("alone") else '[package]\nname = "soak-dep"\nversion = "0.1.0"\nedition = "2021"\n',
                      "src/lib.rs": f"pub fn value() -> u64 {{\n    {model['dep']}\n}}\n"}
            for name, source in wanted.items():
                path = os.path.join(dep_dir, name)
                if not os.path.exists(path) or open(path).read() != source:
                    save(path, source, rng)
        major, minor, patch = model["base_version"]
        new = re.sub(r'^version = "\d+\.\d+\.\d+"', f'version = "{major}.{minor}.{patch + model["version"]}"', new, count=1, flags=re.M)
        if new != text:
            save(manifest, new, rng)
        root = self.path("Cargo.toml")
        text = open(root).read()
        new = re.sub(r"\n# soak:begin\n.*?# soak:end\n", "", text, flags=re.S)
        if model.get("profile"):
            new += f"\n# soak:begin\n[profile.dev.package.{self.spec['package']}]\n{model['profile']}\n# soak:end\n"
        if new != text:
            save(root, new, rng)


def first_model(tree, rng):
    """The model a project starts with: a little of everything at every site."""
    manifest = open(tree.path(tree.spec["manifest"])).read()
    version = re.search(r'^version = "(\d+)\.(\d+)\.(\d+)"', manifest, re.M)
    if not version:
        raise RuntimeError(f"{tree.spec['manifest']} has no version of three numbers")
    sites = []
    for _ in tree.spec["sites"]:
        sites.append(dict(tag=rng.randrange(1000), consts={"C0": rng.randrange(1000)}, mods={}, broken=False,
                          fields=[["f0", "u32", rng.randrange(1000)], ["f1", "String", rng.randrange(1000)]],
                          fns={"f0": dict(params=2, ret="u64", k=rng.randrange(1000), shape=0)}))
    return dict(n=1, sites=sites, dep=None, profile=None, version=0, base_version=[int(x) for x in version.groups()], wide=0)


# The edits of the source: each takes the model, a site of it and the generator, and is
# `None` when there is nothing of its kind to do.
def edit_body(model, site, rng):
    name = rng.choice(sorted(site["fns"]))
    site["fns"][name]["k"] = rng.randrange(100000)
    site["fns"][name]["shape"] = rng.randrange(SHAPES)
    return f"body of {name}"


def edit_call(model, site, rng):
    site["tag"] = rng.randrange(100000)
    return "the call in the project's own function"


def edit_signature(model, site, rng):
    name = rng.choice(sorted(site["fns"]))
    site["fns"][name]["params"] = rng.randrange(5)
    site["fns"][name]["ret"] = rng.choice(RETURN_TYPES)
    return f"signature of {name}"


def edit_type(model, site, rng):
    fields, how = site["fields"], rng.randrange(3)
    if how == 0 or len(fields) < 2:
        model["n"] += 1
        fields.insert(rng.randrange(len(fields) + 1), [f"f{model['n']}", rng.choice(FIELD_TYPES), rng.randrange(100000)])
        return "a field added"
    if how == 1 and len(fields) > 6:
        del fields[rng.randrange(len(fields))]
        return "a field removed"
    field = rng.choice(fields)
    field[1], field[2] = rng.choice(FIELD_TYPES), rng.randrange(100000)
    return f"type of {field[0]}"


def edit_const(model, site, rng):
    if rng.random() < 0.3 and len(site["consts"]) < 8:
        model["n"] += 1
        site["consts"][f"C{model['n']}"] = rng.randrange(100000)
        return "a constant added"
    if rng.random() < 0.15 and len(site["consts"]) > 1:
        del site["consts"][rng.choice(sorted(site["consts"]))]
        return "a constant removed"
    site["consts"][rng.choice(sorted(site["consts"]))] = rng.randrange(100000)
    return "a constant"


def edit_functions(model, site, rng):
    if len(site["fns"]) > 1 and (len(site["fns"]) > 12 or rng.random() < 0.4):
        name = rng.choice(sorted(site["fns"]))
        del site["fns"][name]
        return f"{name} removed"
    model["n"] += 1
    site["fns"][f"f{model['n']}"] = dict(params=rng.randrange(5), ret=rng.choice(RETURN_TYPES), k=rng.randrange(100000), shape=rng.randrange(SHAPES))
    return f"f{model['n']} added"


def edit_files(model, site, rng):
    mods = site["mods"]
    if mods and (len(mods) > 4 or rng.random() < 0.4):
        if rng.random() < 0.5:
            mod = rng.choice(sorted(mods))
            del mods[mod]
            return f"file {mod} removed"
        mods[rng.choice(sorted(mods))] = rng.randrange(100000)
        return "a file of its own edited"
    model["n"] += 1
    mods[f"m{model['n']}"] = rng.randrange(100000)
    return f"file m{model['n']} added"


def edit_broken(model, site, rng):
    site["broken"] = True
    return "broken"


def edit_dep(model, site, rng):
    model["dep"] = rng.randrange(100000) if model["dep"] is None or rng.random() < 0.4 else None
    return "the path dependency " + ("removed" if model["dep"] is None else "added or edited")


def edit_version(model, site, rng):
    model["version"] = (model["version"] + 1) % 5
    return "the version of the package"


def edit_profile(model, site, rng):
    model["profile"] = rng.choice([p for p in PROFILES if p != model["profile"]])
    return f"the profile of the package: {model['profile']}"


EDITS = [(edit_body, 20), (edit_call, 10), (edit_signature, 10), (edit_type, 10), (edit_const, 10), (edit_functions, 10),
         (edit_files, 8), (edit_broken, 4), (edit_dep, 4), (edit_version, 3), (edit_profile, 3)]


class Failure(Exception):
    pass


class Soak:
    def __init__(self, args):
        self.args = args
        self.work = args.work
        self.reference = threading.Lock()
        self.state_path = os.path.join(self.work, "state.json")
        self.home, self.store = os.path.join(self.work, "home"), os.path.join(self.work, "store")
        self.env = dict(base_env(self.home), RA_JIT_CACHE=self.store, SOAK_REPORT="1", RA_JIT_IDLE_SECS=str(args.idle))
        if not args.debuginfo:
            # A tenth of the disk, for cargo and the daemon alike; the switch of profile to
            # release and back is a step of its own.
            self.env.update(CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0", CARGO_INCREMENTAL="0")
        self.env.pop("RA_JIT_REMOTE", None)
        self.metrics = open(os.path.join(self.work, "metrics.jsonl"), "a")
        self.requests, self.touched, self.killed, self.log_at = [], [], False, {}
        if os.path.exists(self.state_path):
            self.state = json.load(open(self.state_path))
        else:
            self.state = dict(seed=args.seed, step=0, projects={}, failures=[], flaky=0, compared=0, removed={}, backend=backend(), head=head())
            self.setup()

    # The checkouts.
    def trees(self, project=None):
        return [Tree(name, root) for name, it in sorted(self.state["projects"].items()) if project in (None, name) for root in it["trees"]]

    def setup(self):
        rng = random.Random(f"{self.state['seed']}/setup")
        for name in self.args.projects:
            root = os.path.join(self.work, name, "main")
            source = os.path.join(HERE, "soak") if name == "macros" else fetch(name, PROJECTS[name])
            shutil.copytree(source, root, symlinks=True, ignore=shutil.ignore_patterns("target", "target-ref"))
            tree = Tree(name, root)
            if not os.path.isdir(tree.path(".git")):
                tree.git("init", "-q")
            manifest = tree.path(tree.spec["manifest"])
            text = open(manifest).read()
            text = text.replace("[features]\n", "[features]\nsoak = []\n", 1) if "[features]\n" in text else text + "\n[features]\nsoak = []\n"
            open(manifest, "w").write(text)
            info = os.path.join(root, tree.git("rev-parse", "--git-common-dir").stdout.strip(), "info")
            os.makedirs(info, exist_ok=True)
            with open(os.path.join(info, "exclude"), "a") as exclude:
                exclude.write("target\ntarget-ref\n*.soak-tmp\n")
            tree.write(first_model(tree, rng))
            subprocess.run(["cargo", "generate-lockfile"] if not os.path.exists(tree.path("Cargo.lock")) else ["cargo", "fetch"], cwd=root, env=self.env, capture_output=True)
            tree.git("add", "-A"), tree.git("commit", "-qm", "soak: the start")
            self.state["projects"][name] = dict(trees=[root], commits=[tree.git("rev-parse", "HEAD").stdout.strip()], made=0)
            for _ in range(self.args.worktrees - 1):
                self.add_worktree(name, rng)
            print(f"{name}: {self.args.worktrees} checkouts in {os.path.dirname(root)}", flush=True)
        self.keep()

    def add_worktree(self, name, rng):
        project = self.state["projects"][name]
        gone = self.state["removed"].get(name, [])
        if gone and rng.random() < 0.7:
            root = gone.pop(rng.randrange(len(gone)))
            again = " (at a path one was removed from)"
        else:
            project["made"] += 1
            root, again = os.path.join(self.work, name, f"wt{project['made']}"), ""
        commit = rng.choice(project["commits"])
        Tree(name, project["trees"][0]).git("worktree", "add", "-q", "--detach", root, commit)
        project["trees"].append(root)
        return Tree(name, root), f"worktree {os.path.basename(root)} of {name} added at {commit[:8]}{again}"

    def keep(self):
        tmp = self.state_path + ".tmp"
        json.dump(self.state, open(tmp, "w"), indent=1)
        os.replace(tmp, self.state_path)

    # Asking.
    def reference_env(self, tree):
        # Nothing is built from it twice, so what an incremental build keeps is only room.
        return dict(self.env, CARGO_TARGET_DIR=tree.path("target-ref"), CARGO_INCREMENTAL="0")

    def command(self, tree, kind, feature):
        """The arguments after `cargo [ra-jit]` and the variables of a request of `kind`."""
        spec, real = tree.spec, PROJECTS.get(tree.project, tree.spec)
        build = list(real.get("build", [])) + (["--features", "soak"] if feature else [])
        if kind == "test":
            test = list(real.get("test", []))
            # The feature is of the package the program is in.
            return ["test", *test, *(["--features", "soak"] if feature and not test else [])], {}
        if kind == "build":
            return ["build", *build], {}
        if kind == "exit" or spec.get("exits") or "run" not in real:
            return ["run", *build], {"SOAK_EXIT": "1"}
        return ["run", *build, "--", *real["run"]], {}

    def ask(self, tree, argv, env, ours=True, cancel=None):
        """Runs one request to its end. `cancel` is how long to wait and the signal to send."""
        base = self.env if ours else self.reference_env(tree)
        if not ours:
            # Cold: cargo starts from nothing, and what it built does not stay on the disk.
            # One at a time: each is as large as the project's whole build.
            with self.reference:
                return self.ask_cargo(tree, argv, env, base)
        return self.ask_cargo(tree, argv, env, base, ours, cancel)

    def ask_cargo(self, tree, argv, env, base, ours=False, cancel=None):
        if not ours:
            shutil.rmtree(tree.path("target-ref"), ignore_errors=True)
        began = time.time()
        child = subprocess.Popen(["cargo", *(["ra-jit"] if ours else []), *argv], cwd=tree.root, env=dict(base, **env),
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, stdin=subprocess.DEVNULL, start_new_session=True)
        cancelled = None
        try:
            if cancel:
                try:
                    child.wait(timeout=cancel[0])
                except subprocess.TimeoutExpired:
                    cancelled = cancel[1]
                    (os.killpg if cancel[2] else os.kill)(child.pid, cancel[1])
            out, err = child.communicate(timeout=REQUEST_SECS)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGKILL)
            child.communicate()
            raise Failure(f"`cargo {'ra-jit ' if ours else ''}{' '.join(argv)}` in {tree.root} did not end in {REQUEST_SECS}s")
        finally:
            # What a killed client leaves running would hold the checkout's lock.
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        ran = subprocess.CompletedProcess(argv, child.returncode, out, err)
        if "No space left on device" in err:
            shutil.rmtree(tree.path("target-ref"), ignore_errors=True)
            raise RuntimeError(f"the disk of {self.work} is full")
        if ours:
            self.requests.append(dict(tree=os.path.relpath(tree.root, self.work), kind=argv[0], secs=round(time.time() - began, 3),
                                      built=how_built(err), cancelled=cancelled))
            if any(it in err for it in ODD):
                self.requests[-1]["said"] = err[-6000:]
        else:
            shutil.rmtree(tree.path("target-ref"), ignore_errors=True)
        return ran

    def status(self, tree=None):
        tree = tree or self.trees()[0]
        return subprocess.run(["cargo", "ra-jit", "status"], cwd=tree.root, env=self.env, capture_output=True, text=True, timeout=120).stdout

    def wait_warm(self, tree):
        began = time.time()
        while time.time() - began < WARM_SECS:
            states = re.findall(rf"^    {re.escape(os.path.realpath(tree.root))}: (.*)$", self.status(tree), re.M)
            if not any(s.startswith("loading") for s in states):
                return
            time.sleep(0.25)

    @staticmethod
    def seen(kind, ran):
        """What of a request has to be the same on both sides."""
        if ran.returncode == 101 and "could not compile" in ran.stderr:
            return "does not compile", sorted(set(re.findall(r"^error(?:\[E\d+\])?: .*$", ran.stderr, re.M)))
        if kind == "test":
            return test_results(ran)
        if kind == "build":
            return ran.returncode
        return program_output(ran)

    def compare(self, tree, kind, feature, mine=None):
        """Asks both sides and compares. A difference that is not there the second time is
        flaky and not a failure."""
        argv, env = self.command(tree, kind, feature)
        for attempt in range(2):
            mine = mine or self.ask(tree, argv, env)
            other = self.ask(tree, argv, env, ours=False)
            wrong = difference(self.seen(kind, mine), self.seen(kind, other))
            self.state["compared"] += 1
            if not wrong:
                self.state["flaky"] += attempt
                return
            first = first if attempt else (mine, other, wrong)
            mine = None
        raise Failure(f"`cargo ra-jit {' '.join(argv)}` in {tree.root} ({how_built(first[0].stderr)}): {first[2]}",
                      {"ra-jit.txt": first[0].stdout + first[0].stderr, "cargo.txt": first[1].stdout + first[1].stderr})

    def direct(self, tree):
        """Asks the worker that holds `tree` for its program by `ra-jit sync`, runs it, and
        compares with what cargo's prints. Nothing when no worker has it loaded."""
        root = os.path.realpath(tree.root)
        for path in glob.glob(os.path.join(self.home, "states", "*.json")):
            if path.endswith(".kept.json"):
                continue
            try:
                state = json.load(open(path))
            except (OSError, ValueError):
                continue
            for checkout in state.get("checkouts", []):
                if checkout.get("root") != root or checkout["config"]["cfg_test"] or checkout["config"]["features"] or len(checkout["programs"]) != 1:
                    continue
                worker = self.worker_of(checkout)
                if worker:
                    return self.direct_sync(tree, checkout, worker)
        return None

    def worker_of(self, checkout):
        """The socket of the worker that has `checkout` loaded and ready."""
        text = self.status()
        for found in re.finditer(r"worker: pid \d+.*?log (\S+)\n((?:    .*\n)*)", text):
            if re.search(rf"^    {re.escape(checkout['root'])}: ready", found[2], re.M):
                socket = os.path.join(os.path.dirname(found[1]), "sock")
                project = os.path.join(checkout["root"], checkout["config"]["project"]).rstrip("/")
                if os.path.exists(socket) and project in found[0]:
                    return socket
        return None

    def direct_sync(self, tree, checkout, socket):
        plan = checkout["programs"][0]
        argv = [RA_JIT, "sync", socket, "--project", os.path.join(checkout["root"], checkout["config"]["project"]).rstrip("/")]
        for krate in plan["crates"]:
            argv += ["--crate", os.path.relpath(krate, checkout["root"]) if os.path.isabs(krate) else krate]
        argv += ["--out", plan["object"], "--patch", plan["patch"], "--link-cmd", plan["link_cmd"], "--splice", plan["binary"]]
        for file in plan["symbol_files"]:
            argv += ["--symbols", file]
        began = time.time()
        env = dict(self.env, **checkout["config"]["env"])
        reply = subprocess.run(argv, cwd=checkout["root"], env=env, capture_output=True, text=True, timeout=REQUEST_SECS)
        said = (reply.stdout + reply.stderr).strip()
        self.requests.append(dict(tree=os.path.relpath(tree.root, self.work), kind="sync", secs=round(time.time() - began, 3),
                                  built="daemon" if said.startswith("ok") else f"refused: {said[-200:]}", cancelled=None))
        if not said.startswith("ok"):
            return said
        mine = subprocess.run([plan["binary"]], cwd=tree.root, env=dict(self.env, SOAK_EXIT="1"), capture_output=True, text=True, timeout=600)
        argv, env = self.command(tree, "exit", False)
        other = self.ask(tree, argv, env, ours=False)
        wrong = difference(program_output(mine), program_output(other))
        self.state["compared"] += 1
        if wrong:
            raise Failure(f"`ra-jit sync` of {tree.root}: {wrong}", {"ra-jit.txt": mine.stdout + mine.stderr, "cargo.txt": other.stdout + other.stderr})
        return said

    def verify(self, tree, rng):
        """Builds `tree` one of the ways there are, and compares at the sampled rate."""
        warm = rng.random() < self.args.warm
        if rng.random() < 0.15 and self.direct(tree) is not None:
            return
        kind = rng.choices(["exit", "run", "test", "build"], [35, 30, 20, 15])[0]
        feature = rng.random() < 0.12
        argv, env = self.command(tree, kind, feature)
        if warm:
            # The daemon loads a build when it is first asked for, which cargo answers.
            self.ask(tree, argv, env)
            self.wait_warm(tree)
        if rng.random() < self.args.sample:
            self.compare(tree, kind, feature)
        else:
            self.ask(tree, argv, env)

    def burst(self, rng, count, cancel=False):
        """`count` clients at once over every checkout. Each is compared with cargo
        afterwards, when it was not cancelled."""
        trees = self.trees()
        asked = []
        for _ in range(count):
            tree = rng.choice(trees)
            kind = rng.choices(["exit", "run", "test", "build"], [40, 30, 15, 15])[0]
            feature = rng.random() < 0.1
            how = (rng.uniform(0, 2.0), rng.choice([signal.SIGINT, signal.SIGKILL, signal.SIGTERM]), rng.random() < 0.5) if cancel and rng.random() < 0.7 else None
            asked.append((tree, kind, feature, how))
        with concurrent.futures.ThreadPoolExecutor(max_workers=count) as pool:
            jobs = [pool.submit(self.ask, tree, *self.command(tree, kind, feature), True, how) for tree, kind, feature, how in asked]
            results = [job.result() for job in jobs]
        known = {}
        for (tree, kind, feature, how), mine in zip(asked, results):
            if how or rng.random() >= self.args.sample:
                continue
            key = (tree.root, kind, feature)
            if key not in known:
                argv, env = self.command(tree, kind, feature)
                known[key] = self.ask(tree, argv, env, ours=False)
            wrong = difference(self.seen(kind, mine), self.seen(kind, known[key]))
            self.state["compared"] += 1
            if wrong:
                # Two test runs of one checkout at once can disturb each other.
                self.compare(tree, kind, feature)
                self.state["flaky"] += 1
        return f"{count} clients at once" + (", most cancelled" if cancel else "")

    # The steps.
    def step_edit(self, rng):
        tree = rng.choice(self.trees())
        model = tree.model()
        said = []
        for _ in range(rng.choice([1, 1, 1, 2, 3])):
            at = rng.randrange(len(model["sites"]))
            site = model["sites"][at]
            if site["broken"]:
                site["broken"] = False
                said.append(f"site {at} mended")
            edit = rng.choices([e for e, _ in EDITS], [w for _, w in EDITS])[0]
            said.append(f"site {at}: {edit(model, site, rng)}")
        tree.write(model, rng)
        self.touched.append(tree)
        return f"{self.name(tree)}: " + "; ".join(said)

    def step_lock(self, rng):
        tree = rng.choice(self.trees())
        lock = open(tree.path("Cargo.lock")).read()
        now = re.search(r'name = "log"\nversion = "([^"]+)"', lock)
        if not now:
            return self.step_edit(rng)
        to = "0.4.25" if now[1] != "0.4.25" else "0.4.26"
        ran = subprocess.run(["cargo", "update", "-p", "log", "--precise", to], cwd=tree.root, env=self.env, capture_output=True, text=True)
        self.touched.append(tree)
        return f"{self.name(tree)}: log locked at {to} from {now[1]}" + ("" if ran.returncode == 0 else f" (cargo would not: {ran.stderr.strip()[-120:]})")

    def step_macros(self, rng):
        trees = self.trees("macros")
        if not trees:
            return self.step_edit(rng)
        tree, how = rng.choice(trees), rng.randrange(3)
        if how == 0:
            path, find, new, what = tree.path("core/build.rs"), r"const SCALE: u64 = \d+;", f"const SCALE: u64 = {rng.randrange(1, 50)};", "the build script"
        elif how == 1:
            path, find, new, what = tree.path("gen/src/lib.rs"), r"const SALT: u64 = \d+;", f"const SALT: u64 = {rng.randrange(1, 50)};", "the proc macro"
        else:
            path, find, new, what = tree.path("core/data.txt"), r"alpha \d+", f"alpha {rng.randrange(1, 50)}", "the build script's data"
        save(path, re.sub(find, new, open(path).read()), rng)
        self.touched.append(tree)
        return f"{self.name(tree)}: {what} edited"

    def commit(self, tree):
        tree.git("add", "-A")
        if tree.git("diff", "--cached", "--quiet", check=False).returncode:
            tree.git("commit", "-qm", f"soak: step {self.state['step']}")
        commit = tree.git("rev-parse", "HEAD").stdout.strip()
        commits = self.state["projects"][tree.project]["commits"]
        if commit not in commits:
            commits.append(commit)
            del commits[:-24]
        return commit

    def widen(self, tree, rng, share):
        """Changes the first line of a share of the Rust files git knows of."""
        model = tree.model()
        model["wide"] += 1
        files = [f for f in tree.git("ls-files", "*.rs").stdout.split("\n") if f and "tests/" not in f and "soak_s" not in f and os.path.isfile(tree.path(f))]
        files = [f for f in files if rng.random() < share]
        for file in files:
            lines = open(tree.path(file)).read().split("\n")
            if lines[0].startswith("// soak wide"):
                lines = lines[1:]
            save(tree.path(file), "\n".join([f"// soak wide {model['wide']}", *lines]))
        save(tree.path(MODEL), json.dumps(model, indent=1, sort_keys=True) + "\n")
        return len(files)

    def step_git(self, rng):
        tree = rng.choice(self.trees())
        self.touched.append(tree)
        how = rng.choices(["commit", "checkout", "stash", "rebase"], [3, 4, 2, 2])[0]
        here = self.commit(tree)
        if how == "commit":
            return f"{self.name(tree)}: committed {here[:8]}"
        if how == "checkout":
            to = rng.choice(self.state["projects"][tree.project]["commits"])
            tree.git("checkout", "-q", "--detach", to)
            return f"{self.name(tree)}: checkout of {to[:8]} from {here[:8]}"
        if how == "stash":
            count = self.widen(tree, rng, rng.choice([0.2, 1.0]))
            self.verify(tree, rng)
            tree.git("stash", "push", "-q", "--include-untracked")
            self.verify(tree, rng)
            tree.git("stash", "pop", "-q")
            return f"{self.name(tree)}: {count} files changed, stashed and brought back"
        parent = tree.git("rev-parse", "-q", "--verify", "HEAD~1", check=False)
        if parent.returncode:
            return f"{self.name(tree)}: nothing to rebase"
        tree.git("checkout", "-q", "--detach", parent.stdout.strip())
        count = self.widen(tree, rng, 1.0)
        onto = self.commit(tree)
        tree.git("checkout", "-q", "--detach", here)
        if tree.git("rebase", "-q", onto, check=False).returncode:
            tree.git("rebase", "--abort", check=False)
            tree.git("checkout", "-q", "--detach", onto)
            return f"{self.name(tree)}: the rebase onto {count} changed files conflicts; left at them"
        self.commit(tree)
        return f"{self.name(tree)}: rebased onto {onto[:8]}, which changes {count} files"

    def step_worktree(self, rng):
        name = rng.choice(sorted(self.state["projects"]))
        project = self.state["projects"][name]
        if len(project["trees"]) > 1 and (len(project["trees"]) > self.args.worktrees or rng.random() < 0.5):
            root = project["trees"].pop(rng.randrange(1, len(project["trees"])))
            Tree(name, project["trees"][0]).git("worktree", "remove", "--force", root, check=False)
            shutil.rmtree(root, ignore_errors=True)
            Tree(name, project["trees"][0]).git("worktree", "prune")
            self.state["removed"].setdefault(name, []).append(root)
            self.touched.append(Tree(name, project["trees"][0]))
            return f"worktree {os.path.basename(root)} of {name} removed"
        tree, said = self.add_worktree(name, rng)
        self.touched.append(tree)
        return said

    def step_burst(self, rng):
        return self.burst(rng, rng.randrange(12, 41))

    def step_cancel(self, rng):
        said = self.step_edit(rng)
        return said + "; " + self.burst(rng, rng.randrange(2, 12), cancel=True)

    def pids(self):
        text = self.status()
        daemon = re.search(r"^daemon: pid (\d+)", text, re.M)
        return (int(daemon[1]) if daemon else None), [int(p) for p in re.findall(r"worker: pid (\d+)", text)]

    def step_kill(self, rng):
        daemon, workers = self.pids()
        victims = [("the daemon", daemon)] * 2 + [("a worker", pid) for pid in workers] if daemon else []
        if not victims:
            return self.step_burst(rng)
        what, pid = rng.choice(victims)
        self.killed = True
        if rng.random() < 0.6:
            said = self.step_edit(rng)
            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                burst = pool.submit(self.burst, random.Random(rng.random()), rng.randrange(4, 16))
                time.sleep(rng.uniform(0, 1.5))
                self.kill(pid)
                return f"{said}; {what} killed under {burst.result()}"
        self.kill(pid)
        return f"{what} killed"

    @staticmethod
    def kill(pid):
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass

    def cache(self, *args):
        return subprocess.run([RA_JIT, "cache", *args], env=self.env, capture_output=True, text=True, timeout=REQUEST_SECS)

    def step_collect(self, rng):
        said = self.step_edit(rng)
        size = rng.choice(["1M", "16M", "64M", "256M"])
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
            burst = pool.submit(self.burst, random.Random(rng.random()), rng.randrange(4, 16))
            time.sleep(rng.uniform(0, 1.0))
            collected = [self.cache("gc", "--max", size) for _ in range(rng.randrange(1, 4))]
            said += f"; the store collected to {size} under {burst.result()}"
        verified = self.cache("verify")
        for ran in [*collected, verified]:
            if ran.returncode:
                raise Failure(f"`ra-jit cache {' '.join(ran.args[2:])}` ended with {ran.returncode}", {"cache.txt": ran.stdout + ran.stderr})
        return said

    STEPS = [("edit", 48), ("lock", 2), ("macros", 6), ("git", 12), ("worktree", 4), ("burst", 8), ("cancel", 8), ("kill", 4), ("collect", 4)]

    def name(self, tree):
        return os.path.relpath(tree.root, self.work)

    # What is measured.
    def measure(self, step, kind, said, took):
        daemon, workers = (None, [])
        try:
            daemon, workers = self.pids()
        except (subprocess.TimeoutExpired, OSError):
            pass
        processes = [p for p in (process(pid) for pid in [daemon, *workers] if pid) if p]
        store = sum(os.path.getsize(os.path.join(d, f)) for d, _, fs in os.walk(self.store) for f in fs if os.path.exists(os.path.join(d, f)))
        line = dict(step=step, at=round(time.time(), 1), kind=kind, said=said, secs=round(took, 2), daemon=daemon, workers=len(workers),
                    rss_kb=sum(p["rss_kb"] for p in processes), fds=sum(p["fds"] for p in processes), threads=sum(p["threads"] for p in processes),
                    daemon_rss_kb=processes[0]["rss_kb"] if daemon and processes else 0, daemon_fds=processes[0]["fds"] if daemon and processes else 0,
                    store_bytes=store, killed=self.killed, requests=self.requests)
        self.metrics.write(json.dumps(line) + "\n")
        self.metrics.flush()
        return line

    def panics(self):
        """What the daemon and its workers logged since the last look that says one of them
        panicked."""
        found = []
        logs = [os.path.join(self.home, "daemon.log"), *glob.glob(os.path.join(self.home, "**", "log"), recursive=True)]
        for log in logs:
            try:
                with open(log, errors="replace") as file:
                    file.seek(self.log_at.get(log, 0) if self.log_at.get(log, 0) <= os.path.getsize(log) else 0)
                    text = file.read()
                    self.log_at[log] = file.tell()
            except OSError:
                continue
            found += [f"{log}: {line}" for line in text.splitlines() if "panicked" in line or "RUST_BACKTRACE" in line and "panicked" in line]
        return found

    def keep_odd(self, step):
        """Keeps the logs of the daemon and its workers beside a refusal that is not the
        ordinary kind (a link that failed, a worker that ended or did not answer)."""
        odd = [r for r in self.requests if any(it in (r["built"] or "") for it in ODD)]
        if not odd or self.state.setdefault("odd", 0) >= 40:
            return
        self.state["odd"] += 1
        where = os.path.join(self.work, "odd", f"step-{step}")
        os.makedirs(where, exist_ok=True)
        with open(os.path.join(where, "requests.json"), "w") as out:
            json.dump(odd, out, indent=1)
        self.keep_logs(where)

    def keep_logs(self, where):
        home = self.env["RA_JIT_HOME"]
        for log in [os.path.join(home, "daemon.log"), *glob.glob(os.path.join(home, "workers", "*", "log"))]:
            if os.path.exists(log):
                shutil.copy(log, os.path.join(where, os.path.relpath(log, home).replace(os.sep, "_")))

    def fail(self, step, why, files=None):
        seed = self.state["seed"]
        where = os.path.join(self.work, "failures", str(step))
        os.makedirs(where, exist_ok=True)
        for name, text in (files or {}).items():
            open(os.path.join(where, name), "w").write(text)
        try:
            open(os.path.join(where, "status.txt"), "w").write(self.status())
        except (subprocess.TimeoutExpired, OSError):
            pass
        self.keep_logs(where)
        for tree in self.touched:
            # What the daemon linked of the tree that differs.
            # Between links the daemon keeps it packed, as `ra-jit.o.zst`.
            for at, built in enumerate(glob.glob(os.path.join(tree.root, "target", "ra-jit", "*", "programs", "*", "ra-jit.o*"))):
                kept = f"{self.name(tree).replace('/', '-')}-{at}-{os.path.basename(os.path.dirname(built))}.o"
                shutil.copy(built, os.path.join(where, kept + (".zst" if built.endswith(".zst") else "")))
            diff = tree.git("diff", "HEAD", check=False).stdout if os.path.isdir(tree.root) else ""
            open(os.path.join(where, self.name(tree).replace("/", "-") + ".diff"), "w").write(diff)
        self.state["failures"].append(dict(step=step, why=why[:2000]))
        print(f"FAIL at step {step} of seed {seed}: {why}\n  kept in {where}\n  again: soak.py --seed {seed} --steps {step} --projects {','.join(self.args.projects)} --worktrees {self.args.worktrees} --sample {self.args.sample}  ({self.state['backend']})", flush=True)

    def make_room(self, least=4 << 30):
        """Keeps the soak from filling its disk: the reference builds go first, which are
        then cold again, and then what cargo and the daemon built, which is built again."""
        for name in ("target-ref", "target"):
            for tree in self.trees():
                if shutil.disk_usage(self.work).free >= least:
                    return
                shutil.rmtree(tree.path(name), ignore_errors=True)
                print(f"       room: {tree.path(name)} removed", flush=True)

    def run(self):
        began = time.time()
        while self.state["step"] < self.args.steps and time.time() - began < self.args.hours * 3600:
            step = self.state["step"] + 1
            rng = random.Random(f"{self.state['seed']}/{step}")
            kind = rng.choices([k for k, _ in self.STEPS], [w for _, w in self.STEPS])[0]
            self.requests, self.touched, self.killed = [], [], False
            started, said, failed = time.time(), "", False
            self.make_room()
            try:
                said = getattr(self, f"step_{kind}")(rng)
                for tree in {t.root: t for t in self.touched}.values():
                    if os.path.isdir(tree.root):
                        self.verify(tree, rng)
                found = self.panics()
                if found:
                    raise Failure(f"{len(found)} panics logged: {found[0][:400]}", {"panics.txt": "\n".join(found)})
            except Failure as failure:
                failed = True
                self.fail(step, f"[{kind}] {said}: {failure.args[0]}", failure.args[1] if len(failure.args) > 1 else None)
            except (RuntimeError, subprocess.TimeoutExpired, OSError) as error:
                failed = True
                self.fail(step, f"[{kind}] {said}: the harness could not go on: {error}")
            self.state["step"] = step
            self.keep_odd(step)
            line = self.measure(step, kind, said, time.time() - started)
            self.keep()
            if not self.args.quiet or failed or step % 25 == 0:
                served = sum(1 for r in self.requests if r["built"] == "daemon")
                print(f"{step:6} {kind:8} {time.time() - started:6.1f}s  {served}/{len(self.requests)} by the daemon  rss {line['rss_kb'] >> 10} MB  fds {line['fds']}  threads {line['threads']}  "
                      f"workers {line['workers']}  store {line['store_bytes'] >> 20} MB  {said[:150]}", flush=True)
            if failed and self.args.stop:
                break
        print(f"seed {self.state['seed']}: {self.state['step']} steps, {self.state['compared']} comparisons with cargo, {len(self.state['failures'])} failures, {self.state['flaky']} flaky", flush=True)
        return 1 if self.state["failures"] else 0


def process(pid):
    """The resident memory, the open files and the threads of a process."""
    try:
        status = open(f"/proc/{pid}/status").read()
        number = lambda name: int(re.search(rf"^{name}:\s+(\d+)", status, re.M)[1])
        return dict(pid=pid, rss_kb=number("VmRSS"), threads=number("Threads"), fds=len(os.listdir(f"/proc/{pid}/fd")))
    except (OSError, TypeError):
        return None


def backend():
    name = os.environ.get("RA_JIT_BACKEND", "cranelift")
    return name if name != "llvm" else f"llvm opt {os.environ.get('RA_JIT_OPT', '?')} emit {os.environ.get('RA_JIT_EMIT', '?')}"


def head():
    return subprocess.run(["git", "rev-parse", "--short=10", "HEAD"], cwd=HERE, capture_output=True, text=True).stdout.strip()


def slope(points):
    """The least-squares slope of `(x, y)` points."""
    n = len(points)
    if n < 2:
        return 0.0
    mx, my = sum(x for x, _ in points) / n, sum(y for _, y in points) / n
    over = sum((x - mx) ** 2 for x, _ in points)
    return sum((x - mx) * (y - my) for x, y in points) / over if over else 0.0


def report(work):
    """Prints what a work directory measured."""
    state = json.load(open(os.path.join(work, "state.json")))
    lines = [json.loads(line) for line in open(os.path.join(work, "metrics.jsonl")) if line.strip()]
    if not lines:
        return print("nothing measured")
    hours = (lines[-1]["at"] - lines[0]["at"] + lines[0]["secs"]) / 3600
    requests = [r for line in lines for r in line["requests"]]
    served = [r for r in requests if r["built"] == "daemon"]
    print(f"head {state.get('head')}, {state.get('backend')}, seed {state['seed']}: {len(lines)} steps in {hours:.2f} h, {len(requests)} requests, "
          f"{len(served)} built by the daemon, {sum(1 for r in requests if r['cancelled'])} cancelled, {state['compared']} compared with cargo, "
          f"{len(state['failures'])} failures, {state['flaky']} flaky")
    print("steps: " + ", ".join(f"{k} {n}" for k, n in sorted(collections.Counter(line["kind"] for line in lines).items())))
    print(f"daemon kills: {sum(1 for line in lines if line['killed'])}")
    for name, unit, scale in (("rss_kb", "MB", 1024), ("fds", "", 1), ("threads", "", 1), ("daemon_rss_kb", "MB (the daemon alone)", 1024),
                              ("daemon_fds", "(the daemon alone)", 1), ("store_bytes", "MB", 1 << 20)):
        values = [line[name] / scale for line in lines]
        per = slope([(line["step"], line[name] / scale) for line in lines]) * 1000
        half = slope([(line["step"], line[name] / scale) for line in lines[len(lines) // 2:]]) * 1000
        print(f"  {name:14} first {values[0]:9.1f}  last {values[-1]:9.1f}  most {max(values):9.1f}  {per:+10.2f} per 1000 steps, {half:+10.2f} over the second half  {unit}")
    for kind in sorted({r["kind"] for r in requests}):
        for who, some in (("daemon", [r for r in served if r["kind"] == kind]), ("cargo", [r for r in requests if r["kind"] == kind and r["built"] != "daemon" and not r["cancelled"]])):
            secs = sorted(r["secs"] for r in some)
            if secs:
                quarter = max(1, len(some) // 4)
                early, late = sorted(r["secs"] for r in some[:quarter]), sorted(r["secs"] for r in some[-quarter:])
                print(f"  {kind:5} by {who:6} {len(secs):6} requests  median {secs[len(secs) // 2]:6.2f}s  p95 {secs[int(len(secs) * 0.95)]:6.2f}s  most {secs[-1]:7.2f}s  "
                      f"median of the first quarter {early[len(early) // 2]:.2f}s, of the last {late[len(late) // 2]:.2f}s")
    reasons = collections.Counter(re.sub(r"/\S+", "PATH", re.sub(r"\d+(\.\d+)?m?s", "N", r["built"]))[:110] for r in requests if r["built"] != "daemon")
    print("built by cargo, why:")
    for reason, count in reasons.most_common(25):
        print(f"  {count:6}  {reason}")
    for failure in state["failures"]:
        print(f"FAIL step {failure['step']}: {failure['why'][:300]}")


def main():
    parser = argparse.ArgumentParser(description="a soak of one ra-jit daemon")
    parser.add_argument("--seed", type=int, default=None)
    parser.add_argument("--steps", type=int, default=10**9)
    parser.add_argument("--hours", type=float, default=10**6)
    parser.add_argument("--work")
    parser.add_argument("--resume")
    parser.add_argument("--report")
    parser.add_argument("--projects", default="ripgrep,fd,bat,axum,macros")
    parser.add_argument("--worktrees", type=int, default=3)
    parser.add_argument("--sample", type=float, default=0.5, help="the share of requests compared with cargo")
    parser.add_argument("--warm", type=float, default=0.7, help="the share of builds that wait for the daemon to have loaded")
    parser.add_argument("--idle", type=int, default=1800, help="RA_JIT_IDLE_SECS of the daemon")
    parser.add_argument("--debuginfo", action="store_true", help="build with the debug info of the profiles, which takes the disk for it")
    parser.add_argument("--stop", action="store_true", help="stop at the first failure")
    parser.add_argument("--quiet", action="store_true", help="a line every 25 steps and for every failure")
    args = parser.parse_args()
    if args.report:
        return report(args.report)
    args.projects = args.projects.split(",")
    if args.resume:
        args.work = args.resume
        kept = json.load(open(os.path.join(args.work, "state.json")))
        args.seed, args.projects = kept["seed"], sorted(kept["projects"])
    else:
        args.seed = args.seed if args.seed is not None else random.SystemRandom().randrange(10**9)
        args.work = os.path.realpath(args.work or tempfile.mkdtemp(prefix="ra-jit-soak-"))
        os.makedirs(args.work, exist_ok=True)
    print(f"seed {args.seed}, {backend()}, in {args.work}", flush=True)
    soak = Soak(args)
    if not shutil.which("cargo-ra-jit", path=soak.env["PATH"].split(os.pathsep)[0]):
        sys.exit("no cargo-ra-jit beside the ra-jit of RA_JIT_BIN")
    if args.resume:
        for tree in soak.trees():
            tree.write(tree.model())
    try:
        return soak.run()
    finally:
        subprocess.run(["cargo", "ra-jit", "stop"], cwd=args.work, env=soak.env, capture_output=True, timeout=120)


if __name__ == "__main__":
    sys.exit(main())
