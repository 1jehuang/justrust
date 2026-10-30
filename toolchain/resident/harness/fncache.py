#!/usr/bin/env python3
"""fncache.py: `#[ra_jit::cache]` keeps what a pure function returns across runs, by the code
ra-jit compiled for it and everything that code reaches (docs/fn-cache.md).

usage: fncache.py            (RA_JIT_BIN names the ra-jit built)
       fncache.py --live     (macOS on AArch64: a live edit drops what the old code kept;
                              cargo-ra-jit and ra-jit-rustc are found next to RA_JIT_BIN)

A crate with cached functions is built by a daemon a step at a time, and each step runs the
program the daemon linked and the one `cargo build` linked, which have to print the same.
`RA_JIT_FN_CACHE_LOG` has the program say, for each cached function, whether a result was found
in memory, found on disk or computed, and each step says which it has to be:

- the first run computes `slow` and the second finds it on disk, and is that much faster;
- a comment and a format edit in it, and an edit of a function it does not reach, keep it;
- an edit to a function it reaches through another computes it again;
- `width::<u8>` and `width::<u64>` are kept apart though their arguments are the same bytes;
- `counted`, `noisy` and `addressed` are refused: a static that changes, printing and an
  address formatted (`{:p}`), the first two said with where they are written;
- `exposed` is refused for `addr`, a method ra-jit would build into its caller but for that;
- `shared` takes an `Rc`, `looked` makes a `HashMap` and `given` takes one, and are kept: the
  second run finds them on disk, `given`'s though its map's entries are in another order;
- `holders` reads an `Rc`'s count and `ordered` a `HashMap`'s order, and are refused;
- `spelled` formats numbers, all of it in core, and is kept; `ticks` reads the cycle counter
  through a function of core known by its name, and is refused on x86-64;
- a daemon started again finds what the first kept: the hash is the same in every process;
- cargo's own build runs every call, and prints the same.
"""
import os, platform, re, shutil, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import linkline
from difftest import Fixture, RA_JIT, sysroot_rlibs, RUNNER, TARGET_ARGS, built

TOML = f"""[package]
name = "fcache"
version = "0.1.0"
edition = "2024"

[dependencies]
ra-jit-cache = {{ path = "{ROOT}/crates/ra-jit-cache" }}
serde = {{ version = "1", features = ["rc"] }}

[workspace]
"""
MAIN = "src/main.rs"
MAIN_TEXT = """use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

#[inline(never)]
fn mix(x: u64) -> u64 {
    x.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(17)
}

#[inline(never)]
fn step(acc: u64, i: u64) -> u64 {
    acc ^ mix(acc.wrapping_add(i))
}

/// Pure, and slow enough to see.
#[ra_jit::cache]
fn slow(rounds: u64) -> u64 {
    let mut acc = 1;
    for i in 0..rounds {
        acc = step(acc, i);
    }
    acc
}

#[ra_jit::cache]
fn fib(n: u64) -> u64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

#[ra_jit::cache]
fn width<T>(n: u64) -> u64 {
    n * std::mem::size_of::<T>() as u64
}

static CALLS: AtomicU64 = AtomicU64::new(0);

#[ra_jit::cache]
fn counted(n: u64) -> u64 {
    CALLS.fetch_add(1, Ordering::Relaxed);
    n + 1
}

#[ra_jit::cache]
fn noisy(n: u64) -> u64 {
    println!("noisy {n}");
    n * 2
}

/// What it gives holds an address, another in every run.
#[ra_jit::cache]
fn addressed(n: u64) -> bool {
    format!("{:p}", &n).len() > 2
}

/// An address as a number, by a method small enough to be built in where it is called.
#[ra_jit::cache]
fn exposed(n: u64) -> bool {
    std::ptr::from_ref(&n).addr() > 0
}

/// An `Rc` is its value: its count is not read.
#[ra_jit::cache]
fn shared(values: Rc<Vec<u64>>) -> u64 {
    values.iter().sum()
}

/// What it gives is how many hold the value, which the arguments don't tell.
#[ra_jit::cache]
fn holders(value: Rc<u64>) -> usize {
    Rc::strong_count(&value)
}

/// A map made and looked into: the seed it is made with orders its entries alone.
#[ra_jit::cache]
fn looked(n: u64) -> u64 {
    let squares: HashMap<u64, u64> = (0..n).map(|i| (i, i * i)).collect();
    squares[&(n / 2)] + squares.len() as u64
}

/// What it gives is in the order of a map's entries, another in every run.
#[ra_jit::cache]
fn ordered(n: u64) -> Vec<u64> {
    let map: HashMap<u64, u64> = (0..n).map(|i| (i, i)).collect();
    map.keys().copied().collect()
}

/// A map given: what it is looked up by is its entries, whatever their order.
#[ra_jit::cache]
fn given(map: HashMap<u64, u64>) -> u64 {
    map[&3] + map.len() as u64
}

/// Formatting, all of it core's: what it reaches there is pure.
#[ra_jit::cache]
fn spelled(n: u64) -> String {
    format!("{n:>8}|{:.3}|{:e}", n as f64 / 7.0, n as f32)
}

/// The processor's cycle counter, read by a function of core known by its name.
#[ra_jit::cache]
fn ticks(n: u64) -> bool {
    #[cfg(target_arch = "x86_64")]
    let read: unsafe fn() -> u64 = core::arch::x86_64::_rdtsc;
    #[cfg(not(target_arch = "x86_64"))]
    let read: unsafe fn() -> u64 = || 1;
    let ticks = unsafe { read() };
    ticks > 0 || n > 0
}

#[inline(never)]
fn unrelated() -> u64 {
    7
}

fn main() {
    let rounds = 150_000_000 + std::env::args().count() as u64 - 1;
    println!("slow {}", slow(rounds));
    println!("fib {}", fib(40));
    println!("width {} {}", width::<u8>(5), width::<u64>(5));
    println!("counted {} {} {}", counted(1), counted(1), CALLS.load(Ordering::Relaxed));
    println!("noisy {} {}", noisy(3), noisy(3));
    println!("addressed {} {}", addressed(4), addressed(4));
    println!("unrelated {}", unrelated());
    println!("exposed {}", exposed(6));
    println!("shared {}", shared(Rc::new(vec![1, 2, 3])));
    let one = Rc::new(5);
    println!("holders {} {}", holders(one.clone()), holders(Rc::new(5)));
    println!("looked {}", looked(1000));
    println!("ordered {}", ordered(64).len());
    println!("given {}", given((0..64).map(|i| (i, i + 1)).collect()));
    println!("spelled {}", spelled(1234));
    println!("ticks {}", ticks(2));
}
"""
SLOW_BODY = """    let mut acc = 1;
    for i in 0..rounds {
        acc = step(acc, i);
    }
    acc
"""
# (what the step is, [(file, old, new)], {function: what the run says of it}, run twice)
STEPS = [
    ("first run", [], {"slow": "computed", "fib": "computed", "shared": "computed", "looked": "computed",
                       "given": "computed", "spelled": "computed"}, False),
    ("second run", [], {"slow": "found on disk", "fib": "found on disk", "shared": "found on disk",
                        "looked": "found on disk", "given": "found on disk", "spelled": "found on disk"}, False),
    ("a comment and a format edit in it", [(MAIN, SLOW_BODY,
        "    // Comments and spaces are not code.\n    let mut acc = 1;\n\n    for i in 0..rounds { acc = step(acc, i); }\n    acc\n")],
        {"slow": "found on disk"}, False),
    ("lines added above it", [(MAIN, "/// Pure, and slow", "// One.\n// Two.\n\n/// Pure, and slow")],
        {"slow": "found on disk"}, False),
    ("a function it does not reach", [(MAIN, "    7\n", "    8\n")], {"slow": "found on disk"}, False),
    ("a function it reaches through another", [(MAIN, "rotate_left(17)", "rotate_left(19)")],
        {"slow": "computed", "fib": "found on disk"}, False),
    ("that edit taken back", [(MAIN, "rotate_left(19)", "rotate_left(17)")], {"slow": "found on disk"}, False),
]
GENERIC = {"width": 2}
REFUSED = {
    "counted": re.compile(r"`fcache::counted` is not cached: it reads the static `fcache::CALLS`, which can change, at src/main\.rs:\d+:\d+"),
    "noisy": re.compile(r"`fcache::noisy` is not cached: it reaches `std::io::stdio::_print`.*, at src/main\.rs:\d+:\d+"),
    # What `format_args!` is lowered to has no place in the source, so neither has the call.
    "addressed": re.compile(r"`fcache::addressed` is not cached: it reaches .*, which calls `[^`]+`, which "
                            r"(formats an address|gives an address as a number)(, at src/main\.rs:\d+:\d+)?$"),
    "exposed": re.compile(r"`fcache::exposed` is not cached: it calls `[^`]*\*const u64>::addr`, which gives an address "
                          r"as a number, at src/main\.rs:\d+:\d+"),
    "holders": re.compile(r"`fcache::holders` is not cached: it calls `[^`]*::strong_count[^`]*`, which reads the "
                          r"reference count, at src/main\.rs:\d+:\d+"),
    "ordered": re.compile(r"`fcache::ordered` is not cached: it calls `[^`]*::keys[^`]*`, which gives the entries of "
                          r"a map in the order of its random seed, at src/main\.rs:\d+:\d+"),
}
if platform.machine() in ("x86_64", "AMD64"):
    REFUSED["ticks"] = re.compile(r"`fcache::ticks` is not cached: it calls `[^`]*::_rdtsc`, which calls the foreign "
                                  r"function `llvm\.x86\.rdtsc`, at src/main\.rs:\d+:\d+")


def said(stderr):
    """What the run said of each cached function: its first line about it."""
    found = {}
    for name, what in re.findall(r"^blaze cache: fcache::(\w+)(?:::<[^>]*>)?: (.*)$", stderr, re.M):
        found.setdefault(name, what)
    return found


def main():
    ws = tempfile.mkdtemp(prefix="ra-jit-fncache-")
    os.makedirs(os.path.join(ws, "src"))
    for path, text in (("Cargo.toml", TOML), (MAIN, MAIN_TEXT)):
        open(os.path.join(ws, path), "w").write(text)
    work = tempfile.mkdtemp(prefix="ra-jit-fncache-out-")
    socket, log_path = os.path.join(work, "ra-jit.sock"), os.path.join(work, "daemon.log")
    reference, store = os.path.join(work, "reference"), os.path.join(work, "fn-cache")
    fx = Fixture(ws, ["fcache"], ["-p", "fcache"])
    fx.rustc_build()
    obj, patch, binary = (os.path.join(work, f"fcache.{n}") for n in ("o", "patch.o", "bin"))
    link = linkline.with_output(linkline.ahead(list(fx.link_args), [patch, obj], RA_JIT), binary)
    symbols = [x for rlib in fx.rlibs + sysroot_rlibs() for x in ("--symbols", rlib)]
    ask = ["--crate", "fcache", "--out", obj, "--patch", patch, "--link-cmd", linkline.join(link),
           "--binary", binary, *symbols]
    env = {**os.environ, "RA_JIT_FN_CACHE_DIR": store, "RA_JIT_FN_CACHE_LOG": "1"}

    def start():
        seen = len(open(log_path).read()) if os.path.exists(log_path) else 0
        out = open(log_path, "a")
        daemon = subprocess.Popen([RA_JIT, ws, *TARGET_ARGS, "--socket", socket, *ask], stderr=out, stdout=out)
        while "watching" not in open(log_path).read()[seen:]:
            if daemon.poll() is not None:
                raise SystemExit(f"the daemon did not start: {log_path}")
            time.sleep(0.05)
        return daemon

    def run(path, env=env):
        began = time.time()
        ran = subprocess.run([*RUNNER, path], capture_output=True, text=True, timeout=300, env=env)
        return ran.stdout + (f"[exit {ran.returncode}]" if ran.returncode else ""), ran.stderr, time.time() - began

    def sync():
        reply = subprocess.run([RA_JIT, "sync", socket, *ask], capture_output=True, text=True, timeout=300)
        return (reply.stdout + reply.stderr).strip()

    def cargo():
        build = subprocess.run(["cargo", "build", "-q", *TARGET_ARGS], cwd=ws, capture_output=True, text=True,
                               env={**os.environ, "CARGO_TARGET_DIR": reference})
        assert build.returncode == 0, build.stderr[-2000:]
        return built(reference, "fcache")

    wrong = made = 0

    def verdict(what, ok, detail):
        nonlocal wrong, made
        wrong += not ok
        made += 1
        print(f"{what:44} {'ok' if ok else 'WRONG':6} {detail}", flush=True)

    daemon = start()
    times = []
    try:
        for what, edits, wanted, _ in STEPS:
            for path, old, new in edits:
                text = open(os.path.join(ws, path)).read()
                assert old in text, (what, old)
                open(os.path.join(ws, path), "w").write(text.replace(old, new, 1))
            time.sleep(0.02)
            reply = sync()
            if not reply.startswith("ok"):
                verdict(what, False, f"the daemon refused: {reply[:200]}")
                continue
            ours, stderr, took = run(binary)
            theirs, _, cargo_took = run(cargo(), {**os.environ, "RA_JIT_FN_CACHE_LOG": "1"})
            found = said(stderr)
            missed = {k: found.get(k) for k, v in wanted.items() if not found.get(k, "").startswith(v)}
            times.append((what, took, cargo_took))
            same = ours == theirs
            verdict(what, same and not missed,
                    f"{took * 1000:6.0f} ms (cargo {cargo_took * 1000:.0f} ms)"
                    + ("" if same else "  DIFFERENT OUTPUT") + (f"  said {missed}" if missed else ""))
        # The two instances of `width` have entries of their own, and give their own results.
        ours, stderr, _ = run(binary)
        widths = re.findall(r"^blaze cache: fcache::width::<(\w+)>: (.*)$", stderr, re.M)
        verdict("generic instances kept apart", "width 5 40\n" in ours and len({t for t, _ in widths}) == 2,
                f"{widths}")
        for name, pattern in REFUSED.items():
            told = [line for line in stderr.splitlines() if f"fcache::{name}`" in line]
            verdict(f"`{name}` refused, saying where", any(pattern.search(it) for it in told), (told or ["nothing"])[0][:160])
        calls = re.search(r"^counted 2 2 (\d+)$", ours, re.M)
        verdict("a refused function runs every call", calls is not None and calls[1] == "2" and ours.count("noisy 3\n") == 2,
                ours.splitlines()[3:5])
        # A daemon of its own works out the same hashes.
        daemon.kill()
        daemon.wait()
        os.remove(socket) if os.path.exists(socket) else None
        daemon = start()
        reply = sync()
        ours, stderr, took = run(binary)
        found = said(stderr)
        verdict("a daemon started again finds the results", reply.startswith("ok") and found.get("slow") == "found on disk",
                f"{took * 1000:.0f} ms, slow {found.get('slow')}" + ("" if reply.startswith("ok") else f", the daemon said {reply[:200]}"))
        # `cargo ra-jit cache` lists what each function kept, by its code and the older ones;
        # `clear --stale` leaves the code's alone, and the next run still finds them.
        cargo_ra_jit = os.path.join(os.path.dirname(os.path.abspath(RA_JIT)), "cargo-ra-jit")
        listed = lambda *args: subprocess.run([cargo_ra_jit, "ra-jit", "cache", *args], capture_output=True, text=True,
                                              env={**os.environ, "RA_JIT_FN_CACHE_DIR": store}, timeout=60).stdout
        before = listed()
        cleared = listed("clear", "--stale")
        after = listed()
        _, stderr, _ = run(binary)
        slow = re.search(r"^  fcache::slow: 1 results .*$", before, re.M)
        verdict("the store lists each function, and forgets old code", bool(slow) and "older code" in before
                and "older code" not in after and said(stderr).get("slow") == "found on disk",
                f"{slow[0].strip() if slow else before[:120]!r}; {cleared.strip()}")
        # Under cargo the attribute does nothing: every call runs, and nothing is kept.
        theirs, stderr, cargo_took = run(cargo(), {**os.environ, "RA_JIT_FN_CACHE_LOG": "1", "RA_JIT_FN_CACHE_DIR": store})
        verdict("cargo's build caches nothing", "ra-jit" not in stderr and theirs == ours, f"{cargo_took * 1000:.0f} ms")
    finally:
        daemon.kill()
    entries = sum(len(files) for _, _, files in os.walk(store))
    print(f"{entries} results kept in {store}")
    print(f"{made - wrong} of {made} right; log {log_path}")
    sys.exit(1 if wrong else 0)


LIVE_TEXT = """use std::{io::Write, time::Duration};

#[inline(never)]
fn base() -> u64 {
    1
}

#[ra_jit::cache]
fn answer(n: u64) -> u64 {
    base() * n
}

fn main() {
    let live = match ra_jit_live::Live::start() {
        Ok(live) => live,
        Err(why) => {
            println!("no live edits: {why}");
            std::process::exit(2);
        }
    };
    live.apply_as_they_come(|outcome| {
        println!("outcome: {outcome}");
        std::io::stdout().flush().unwrap();
    });
    println!("ready");
    let mut seen = 0;
    loop {
        let now = answer(3);
        if now != seen {
            println!("answer {now}");
            seen = now;
        }
        std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
}
"""


def live():
    """A running program whose cached function reaches an edited one: after the patch the
    function computes again, and what the old code kept is dropped from memory."""
    import asyncio
    import live_e2e as e2e

    async def go():
        bin_dir = os.path.dirname(os.path.abspath(RA_JIT))
        work = tempfile.mkdtemp(prefix="ra-jit-fncache-live-")
        # A socket's path is short: the daemon's home is too.
        probe, home = None, tempfile.mkdtemp(prefix="rjfc-", dir="/tmp")
        try:
            for crate in ("ra-jit-live", "ra-jit-cache", "ra-jit-cache-macros"):
                shutil.copytree(os.path.join(ROOT, "crates", crate), os.path.join(work, "crates", crate),
                                ignore=shutil.ignore_patterns("target"))
            project = os.path.join(work, "app")
            os.makedirs(os.path.join(project, "src"))
            open(os.path.join(project, "Cargo.toml"), "w").write(
                '[package]\nname = "fcachelive"\nversion = "0.1.0"\nedition = "2024"\n\n[dependencies]\n'
                'ra-jit-live = { path = "../crates/ra-jit-live" }\nra-jit-cache = { path = "../crates/ra-jit-cache" }\n\n[workspace]\n')
            source = os.path.join(project, MAIN)
            open(source, "w").write(LIVE_TEXT)
            env = dict(os.environ, PATH=f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
                       CARGO_TARGET_DIR=os.path.join(work, "target"), RA_JIT_HOME=home,
                       RA_JIT_FN_CACHE_LOG="1", RA_JIT_FN_CACHE="memory")
            env.pop("CARGO_INCREMENTAL", None)
            build = ["cargo-ra-jit", "ra-jit", "build"]
            subprocess.run(["cargo", "generate-lockfile"], cwd=project, env=env, check=True, capture_output=True)

            async def cargo_ra_jit(step):
                proc = await asyncio.create_subprocess_exec(*build, cwd=project, env=env, stdout=asyncio.subprocess.PIPE,
                                                            stderr=asyncio.subprocess.PIPE)
                out, err = await proc.communicate()
                text = (out + err).decode(errors="replace")
                if proc.returncode != 0:
                    raise e2e.StepError(f"{step}: {text[-3000:]}")
                return text

            await cargo_ra_jit("first build")
            for _ in range(120):
                if "[blaze]" in await cargo_ra_jit("wait for the daemon"):
                    break
                await asyncio.sleep(0.5)
            else:
                raise e2e.StepError("no build said `[blaze]`")

            def edit(old, new):
                text = open(source).read()
                needle = f"fn base() -> u64 {{\n    {old}\n}}"
                assert needle in text
                open(source, "w").write(text.replace(needle, f"fn base() -> u64 {{\n    {new}\n}}"))

            edit(1, 2)
            await cargo_ra_jit("edit before start")
            probe = await asyncio.create_subprocess_exec(
                os.path.join(work, "target", "debug", "fcachelive"), cwd=project, env=env,
                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT)
            lines, log = asyncio.Queue(), []
            pumper = asyncio.create_task(e2e.pump(probe.stdout, lines))
            await e2e.wait_for("program start", lines, "ready", log)
            await e2e.wait_for("the cached answer", lines, "answer 6", log)
            for new in (3, 4):
                edit(new - 1, new)
                since = len(log)
                await cargo_ra_jit(f"base {new} while running")
                applied = await e2e.wait_for(f"patch of base {new}", lines, "outcome:", log)
                if "applied" not in applied:
                    raise e2e.StepError(f"the program did not take the patch: {applied}")
                await e2e.wait_for(f"answer {3 * new}", lines, f"answer {3 * new}", log, since)
                await e2e.wait_for("the old results dropped", lines, "results of the old code dropped", log, since)
            print("live: answer 6 -> 9 -> 12, each edit of `base` computed again and the old results dropped")
            pumper.cancel()
        finally:
            if probe is not None and probe.returncode is None:
                probe.terminate()
                await probe.wait()
            subprocess.run(["cargo-ra-jit", "ra-jit", "stop"], env=dict(os.environ, RA_JIT_HOME=home,
                           PATH=f"{os.path.dirname(os.path.abspath(RA_JIT))}{os.pathsep}{os.environ['PATH']}"),
                           capture_output=True)
            shutil.rmtree(work, ignore_errors=True)
            shutil.rmtree(home, ignore_errors=True)

    try:
        asyncio.run(go())
    except e2e.StepError as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    live() if "--live" in sys.argv else main()
