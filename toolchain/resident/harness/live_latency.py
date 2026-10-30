#!/usr/bin/env python3
"""Save-to-running latency of live edits (macOS on AArch64, Windows on x86-64), step by step.

usage: live_latency.py [--program probe|life|bevy|egui] [cargo|socket|burst|replay|restart ...]
       (RA_JIT_BIN names the ra-jit built; cargo-ra-jit and ra-jit-rustc are found next to it)

The program is harness/live (`probe`, the default: a few functions and ra-jit-live), the Game
of Life demo (`life`: demo/life with eframe, a 38 MB debug executable, run without its window
with RA_JIT_LIFE_HEADLESS=1), a bevy app (`bevy`: harness/live-bevy, a simulation of ships,
rocks and shots on bevy's ECS with generic systems, without a window, one 600-line file, 21 MB)
or an egui app (`egui`: harness/live-egui, a ledger whose whole UI egui lays out and tessellates
every frame without a window, generic widgets, one 580-line file). Each applies each patch as it
arrives, as the demo's window does. It is built and started as live_e2e.py does, then takes
LATENCY_EDITS edits (default 10) in each mode named:

  cargo   the file is written and `cargo ra-jit build` asks the daemon for the build, as the
          demo's launcher did on each save (its 200 ms mtime poll, 100 ms on average, comes
          on top and is not in these numbers)
  socket  the text goes straight to the daemon (`edit` line on its socket, what the demo's
          editor sends on each keystroke); the file is written after, off the clock. One edit
          at a time, each after the last is running: an isolated edit
  burst   typing: LATENCY_BURSTS bursts (default 3) of LATENCY_KEYS keystrokes (default 20),
          30 to 100 ms apart, each a valid edit of the function. They go to the daemon as the
          demo's editor sends them: one edit on the wire at a time, the newest text next, the
          file written after each answer. Each keystroke is timed from when it was typed to
          the program running code at least as new as it (the program's `answer N at_us=`);
          `queued` is the part before the edit that has it was sent
  replay  real typing: the program's scripts (rewrite a body, delete and retype an
          expression, type a generic helper and call it, a correction now and then) typed key
          by key, LATENCY_ROUNDS rounds (default 1; each round types other numbers and ends
          where it started), 30 to 100 ms between keys, a pause of 150 to 600 ms after three
          in ten word ends, a typo and its backspace in 3% of letters, seeded. Most texts in
          between do not compile. Each key sends the whole file as burst does; a key whose
          text compiles is timed from when it was typed to the program having applied a patch
          of an edit at least as new (the edit's patch is found by the `live: edit SEQ` stamp
          the worker logs between its send and its answer, and the program's `outcome: edit
          SEQ applied` says when; a patch carries every function that differs from the
          executable, so a newer one has the older's code too). A program told to restart,
          or one whose executable the daemon linked again (`ok object=`: it takes no more
          patches, and is not always told), is started again as a launcher would: `cargo
          ra-jit build` once an answer says the text on disk builds, then the executable to
          its answer; the keys up to the one built are then timed to that. Whether a key that
          was typed over before it was sent compiles is asked of the daemon after the round,
          off the clock; an error that is ra-jit's (`` `main` is not compiled by ra-jit``)
          and not a diagnostic counts as compiling, and is counted as refused; `no program
          built has <file>` says nothing of the text, which is asked again after the round
          and left out if the daemon still has no program. Every answer goes to the JSON,
          under counts
  restart a session of the program's edits one at a time, mostly bodies and a few changes of
          a type: the edits answered with a restart, and for each the time from the edit to
          the program started again running it (`cargo ra-jit build`, then the executable to
          its first answer) against plain incremental `cargo build` of the same source and the
          start of what it built (`cargo run`), in a target directory of its own. An edit the
          daemon refuses though it compiles (bevy's `main`, left to rustc) is restarted the
          same way, as a launcher would, and counted as refused. A `cargo ra-jit build` that
          fails is written down as the restart's outcome, cargo's time is still taken, and
          the session ends there

Each edit of cargo and socket is timed from just before it is written or sent (t0) to the
program having the new code in place, and split by the epoch stamps the worker (`request:`,
`live: ... at_us=`) and the program (`outcome: ... at_us=`) print:

  to-worker   t0 to the worker reading the request (cargo and the daemon's hop)
  worker      the request to the patch left for the program (`timeline-us:` splits it)
  delivery    the patch file written to the program's thread reading it
  wait        read to applied: the program's thread taking it up
  apply       mapping the patch and writing the jumps
  total       t0 to applied, what the demo shows as "edit to running" (the old in-process
              prototype timed the same span: from just before the edit left the editor to
              the new code in place, with no frame in between)
  reply       t0 to the client's answer

The tables (median, p90, p95, p99, min, max) go to stdout and to $GITHUB_STEP_SUMMARY when it is
set. $LATENCY_JSON, when set, gets every edit's numbers with what was measured where:
{"program", "date", "commit", "run_url", "host": {"name", "cpu", "cores", "memory_gib", "os",
"battery"}, "modes": {mode: [row, ...]}, "counts": {mode: {...}}}, a row having at least
"total" in ms; harness/benchhistory.py `live` reads it. A mode that fails leaves the modes
before it in the JSON, and its error under counts.failed. The host's name is $RA_JIT_BENCH_MACHINE
or the Mac's model; "battery" is what `pmset -g batt` says ("AC" or "battery 87%").
LATENCY_TARGET_DIR keeps the programs' target directory across runs (bevy's dependencies take
minutes to build). Python 3.13, stdlib only.
"""

import asyncio
import collections
import datetime
import io
import json
import os
import platform
import random
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable, Iterator
from dataclasses import dataclass, field
from pathlib import Path

from live_e2e import ROOT, HERE, StepError, build, daemon_home, pump, serving, show_daemon_log, wait_for

EDITS = int(os.environ.get("LATENCY_EDITS", "10"))
KEYS = int(os.environ.get("LATENCY_KEYS", "20"))
BURSTS = int(os.environ.get("LATENCY_BURSTS", "3"))
ROUNDS = int(os.environ.get("LATENCY_ROUNDS", "1"))
REPLY_TIMEOUT = 60
COLUMNS = ["to-worker", "worker", "delivery", "wait", "apply", "total", "reply"]
# What an executable's name ends with where cargo writes it.
EXE = ".exe" if os.name == "nt" else ""

# A replay script: what the caret and the keys do, in order. ("at", s) puts the caret after
# `s`, which the text has once; ("cut", s) selects `s` and deletes it, one key; ("type", s)
# types `s` a key at a time (a newline and the indent after it are one key: the editor
# indents); ("back", n) is n backspaces.
Op = tuple[str, str | int]
Script = tuple[str, list[Op]]
# A session edit: its kind ("body" or "type"), what it does, and the replacements it makes.
Change = tuple[str, str, list[tuple[str, str]]]


def retype(anchor: str, old: str, new: str) -> list[Op]:
    """The caret after `anchor`, which ends in `old`; `old` backspaced away and `new` typed."""
    assert anchor.endswith(old), (anchor, old)
    return [("at", anchor), ("back", len(old)), ("type", new)]


def probe_scripts(r: int) -> list[Script]:
    weighed = f"total += (*item).into() * {r + 2}"
    bias = f"\n    let bias = {r + 1} as u64;"
    return [
        ("scale: retype the body", retype("fn scale(x: u64) -> u64 {\n    x * 2", "x * 2", f"x * {r + 3} + x / 2")),
        ("weigh<T>: a factor, a local and its use", [
            ("at", "total += (*item).into()"), ("type", f" * {r + 2}"),
            ("at", "let mut total = 0;"), ("type", bias),
            ("at", "\n    total"), ("type", " + bias"),
        ]),
        ("scale: back as it was", retype(f"fn scale(x: u64) -> u64 {{\n    x * {r + 3} + x / 2", f"x * {r + 3} + x / 2", "x * 2")),
        ("weigh<T>: back as it was", [
            ("at", "\n    total + bias"), ("back", len(" + bias")),
            ("at", f"let mut total = 0;{bias}"), ("cut", bias),
            ("at", weighed), ("back", len(f" * {r + 2}")),
        ]),
    ]


def life_scripts(r: int) -> list[Script]:
    rule = f" || n == {6 + r % 3}"
    hue = ["ff8800", "e0a030", "f25f5c", "c084fc"][r % 4]
    helper = (
        "\n/// How many of `xs` are above `floor`.\n#[inline(never)]\n"
        "fn count_above<T: PartialOrd + Copy>(xs: &[T], floor: T) -> usize {\n"
        "    xs.iter().filter(|x| **x > floor).count()\n}\n"
    )
    counted = "self.age.iter().filter(|a| **a > 0).count()"
    call = f"count_above(&self.age, {r % 2})"
    return [
        ("next_age: HighLife", [("at", "} else if n == 3"), ("type", rule)]),
        ("color: retype a colour", retype("age < 4 {\n        0xffd23f", "ffd23f", hue)),
        ("count_above<T>: typed, then called from step", [
            ("at", "        0x3a86ff\n    }\n}\n"), ("type", helper),
            ("cut", counted), ("type", call),
        ]),
        ("next_age: back to Conway", [("at", "} else if n == 3" + rule), ("back", len(rule))]),
        ("color: back as it was", retype(f"age < 4 {{\n        0x{hue}", hue, "ffd23f")),
        ("count_above<T>: step as it was, the helper deleted", [
            ("cut", call), ("type", counted), ("cut", helper),
        ]),
    ]


def bevy_scripts(r: int) -> list[Script]:
    pull = f"0.{r + 2}"
    guard = f"\n        if speed > {40 + r} as f32 {{\n            velocity.0 *= 0.98;\n        }}"
    return [
        ("drift<A>: retype the step of a generic system", retype(
            "transform.translation += velocity.0 * dt", "velocity.0 * dt", f"velocity.0 * dt * (1.0 + {pull})")),
        ("steer: a guard typed into a big system", [("at", "let speed = velocity.0.length();"), ("type", guard)]),
        ("score_of<K>: retype a generic body", retype("    K::POINTS * hits", "K::POINTS * hits", f"K::POINTS * hits + {r + 1}")),
        ("drift<A>: back as it was", retype(
            f"transform.translation += velocity.0 * dt * (1.0 + {pull})", f"velocity.0 * dt * (1.0 + {pull})", "velocity.0 * dt")),
        ("steer: the guard deleted", [("cut", guard)]),
        ("score_of<K>: back as it was", retype(f"    K::POINTS * hits + {r + 1}", f" + {r + 1}", "")),
    ]


def egui_scripts(r: int) -> list[Script]:
    label = f"\"Due in {r + 2} days\""
    width = f"{120 + 10 * r}.0"
    clamp = f"\n    let value = value.min({90 + r}.0);"
    return [
        ("labeled<T>: retype a generic widget's text", retype(
            "ui.label(format!(\"{name}: {value}\")", "format!(\"{name}: {value}\")", f"format!(\"{{name}} = {{value}} ({r})\")")),
        ("stat_bar<T>: a clamp typed into a generic body", [("at", "let value: f64 = value.into();"), ("type", clamp)]),
        ("invoice_row: retype a column width", retype("ui.set_min_width(140.0", "140.0", width)),
        ("footer: a new label", [("at", "ui.label(\"Totals\");"), ("type", f"\n            ui.label({label});")]),
        ("labeled<T>: back as it was", retype(
            f"ui.label(format!(\"{{name}} = {{value}} ({r})\")", f"format!(\"{{name}} = {{value}} ({r})\")", "format!(\"{name}: {value}\")")),
        ("stat_bar<T>: the clamp deleted", [("cut", clamp)]),
        ("invoice_row: back as it was", retype(f"ui.set_min_width({width}", width, "140.0")),
        ("footer: the label deleted", [("cut", f"\n            ui.label({label});")]),
    ]


PROBE_SESSION: list[Change] = [
    ("body", "scale: another factor", [("x * 2\n", "x * 3\n")]),
    ("body", "weigh<T>: start at one", [("let mut total = 0;", "let mut total = 1;")]),
    ("type", "Tally: a field added", [("    turns: u64,\n}", "    turns: u64,\n    last: u64,\n}"), ("Tally { turns: 0 }", "Tally { turns: 0, last: 0 }")]),
    ("body", "scale: back", [("x * 3\n", "x * 2\n")]),
    ("body", "weigh<T>: back", [("let mut total = 1;", "let mut total = 0;")]),
    ("body", "scale: a sum", [("x * 2\n", "x * 2 + 1\n")]),
    ("type", "Tally: the field's type changed", [("    last: u64,", "    last: u32,")]),
    ("body", "scale: back again", [("x * 2 + 1\n", "x * 2\n")]),
]

LIFE_SESSION: list[Change] = [
    ("body", "color: an old cell's blue", [("0x3a86ff", "0x3a6dff")]),
    ("body", "rand: another increment", [("wrapping_add(12345)", "wrapping_add(12347)")]),
    ("type", "Life: a field added", [("    generation: u64,\n    seed: u64,", "    generation: u64,\n    births: u64,\n    seed: u64,"),
                                     ("generation: 0, seed: 12345", "generation: 0, births: 0, seed: 12345")]),
    ("body", "step: counts births", [("        self.generation += 1;\n", "        self.generation += 1;\n        self.births += 1;\n")]),
    ("body", "reseed: denser", [("self.rand() < 30", "self.rand() < 32")]),
    ("body", "step: reseed sooner", [(".count() < 25", ".count() < 30")]),
    ("type", "Stats: a new struct", [("// ---- State that lives across edits.", "struct Stats {\n    peak: usize,\n}\n\n// ---- State that lives across edits.")]),
    ("body", "color: a younger threshold", [("age < 12", "age < 10")]),
    ("body", "neighbours: the same, written out", [("if dx != 1 || dy != 1 {", "if !(dx == 1 && dy == 1) {")]),
    ("type", "Life: two fields swapped", [("    births: u64,\n    seed: u64,", "    seed: u64,\n    births: u64,")]),
    ("body", "color: blue as it was", [("0x3a6dff", "0x3a86ff")]),
    ("body", "reseed: as it was", [("self.rand() < 32", "self.rand() < 30")]),
]

BEVY_SESSION: list[Change] = [
    ("body", "wrap: a wider field", [("const HALF: f32 = 400.0;", "const HALF: f32 = 420.0;")]),
    ("body", "drift<A>: damped", [("transform.translation += velocity.0 * dt", "transform.translation += velocity.0 * dt * 0.99")]),
    ("type", "Ship: a field added", [("pub struct Ship {\n    pub fuel: f32,", "pub struct Ship {\n    pub fuel: f32,\n    pub heat: f32,"),
                                     ("Ship { fuel: 100.0,", "Ship { fuel: 100.0, heat: 0.0,")]),
    ("body", "score_of<K>: a bonus", [("    K::POINTS * hits\n", "    K::POINTS * hits + 1\n")]),
    ("body", "spawn_rocks: more of them", [("for i in 0..ROCKS {", "for i in 0..ROCKS + 8 {")]),
    ("type", "Size: a variant added", [("pub enum Size {\n    Small,", "pub enum Size {\n    Tiny,\n    Small,"),
                                       ("Size::Small => 8.0,", "Size::Tiny => 4.0,\n        Size::Small => 8.0,"),
                                       ("Size::Small => 1,", "Size::Tiny => 1,\n        Size::Small => 1,"),
                                       ("Size::Small => None,", "Size::Small | Size::Tiny => None,"),
                                       ("Size::Small => score_of::<Pebble>(1),", "Size::Tiny | Size::Small => score_of::<Pebble>(1),")]),
    ("body", "drift<A>: as it was", [("transform.translation += velocity.0 * dt * 0.99", "transform.translation += velocity.0 * dt")]),
    ("body", "score_of<K>: as it was", [("    K::POINTS * hits + 1\n", "    K::POINTS * hits\n")]),
    ("type", "Velocity: derives Default", [("#[derive(Component, Clone, Copy)]\npub struct Velocity", "#[derive(Component, Clone, Copy, Default)]\npub struct Velocity")]),
    ("body", "wrap: as it was", [("const HALF: f32 = 420.0;", "const HALF: f32 = 400.0;")]),
]

EGUI_SESSION: list[Change] = [
    ("body", "invoice_row: wider", [("ui.set_min_width(140.0)", "ui.set_min_width(150.0)")]),
    ("body", "stat_bar<T>: a taller bar", [("egui::vec2(160.0, 12.0)", "egui::vec2(160.0, 14.0)")]),
    ("type", "Invoice: a field added", [("pub struct Invoice {\n    pub number: u32,", "pub struct Invoice {\n    pub number: u32,\n    pub note: String,"),
                                        ("Invoice {\n                    number,", "Invoice {\n                    number,\n                    note: String::new(),")]),
    ("body", "labeled<T>: a colon", [("format!(\"{name}: {value}\")", "format!(\"{name} : {value}\")")]),
    ("body", "footer: another heading", [("ui.label(\"Totals\");", "ui.label(\"Totals (all)\");")]),
    ("type", "Status: a variant added", [("pub enum Status {\n    Draft,", "pub enum Status {\n    Draft,\n    Held,"),
                                         ("Status::Draft => Color32::from_rgb(140, 140, 150),", "Status::Draft => Color32::from_rgb(140, 140, 150),\n            Status::Held => Color32::from_rgb(200, 160, 60),"),
                                         ("Status::Draft => \"draft\",", "Status::Draft => \"draft\",\n            Status::Held => \"held\",")]),
    ("body", "invoice_row: as it was", [("ui.set_min_width(150.0)", "ui.set_min_width(140.0)")]),
    ("body", "stat_bar<T>: as it was", [("egui::vec2(160.0, 14.0)", "egui::vec2(160.0, 12.0)")]),
    ("type", "Filter: a field added", [("pub struct Filter {\n    pub text: String,", "pub struct Filter {\n    pub text: String,\n    pub overdue: bool,"),
                                       ("Filter { text: String::new(),", "Filter { text: String::new(), overdue: false,")]),
    ("body", "labeled<T>: as it was", [("format!(\"{name} : {value}\")", "format!(\"{name}: {value}\")")]),
]


@dataclass
class Program:
    """A program to edit while it runs: where its crate is, and the function edited."""

    name: str
    project: Path
    binary: str
    function: str
    value: int
    env: dict[str, str]
    scripts: Callable[[int], list[Script]] = lambda r: []
    session: list[Change] = field(default_factory=list)

    def needle(self, value: int) -> str:
        return f"fn {self.function}() -> u64 {{\n    {value}\n}}"

    def edited(self, text: str, old: int, new: int) -> str:
        if self.needle(old) not in text:
            raise StepError(f"edit: `{self.needle(old)}` is not in {self.source}")
        return text.replace(self.needle(old), self.needle(new))

    @property
    def source(self) -> Path:
        return self.project / "src" / "main.rs"


def copy_program(name: str, work: Path) -> Program:
    shutil.copytree(ROOT / "crates" / "ra-jit-live", work / "crates" / "ra-jit-live")
    ignore = shutil.ignore_patterns("target")
    match name:
        case "probe":
            shutil.copytree(HERE / "live", work / "harness" / "live", ignore=ignore)
            return Program(name, work / "harness" / "live", "liveprobe", "answer", 1, {}, probe_scripts, PROBE_SESSION)
        case "life":
            shutil.copytree(ROOT / "demo" / "life", work / "demo" / "life", ignore=ignore)
            env = {"RA_JIT_LIFE_HEADLESS": "1"}
            return Program(name, work / "demo" / "life", "life", "step_ms", 80, env, life_scripts, LIFE_SESSION)
        case "bevy" | "egui":
            shutil.copytree(HERE / f"live-{name}", work / "harness" / f"live-{name}", ignore=ignore)
            scripts, session = {"bevy": (bevy_scripts, BEVY_SESSION), "egui": (egui_scripts, EGUI_SESSION)}[name]
            return Program(name, work / "harness" / f"live-{name}", f"live{name}", "answer", 1, {}, scripts, session)
    raise StepError(f"unknown program {name!r}: probe, life, bevy or egui")


def now_us() -> int:
    return time.time_ns() // 1000


def escape(text: str) -> str:
    return text.replace("\\", "\\\\").replace("\n", "\\n").replace("\t", "\\t").replace("\r", "\\r")


def ask_edit_winsock(socket: Path, line: bytes) -> str:
    """`ask_edit`'s exchange through Winsock's AF_UNIX, which Python's socket module has not on
    Windows."""
    import ctypes

    ws2 = ctypes.WinDLL("ws2_32")
    ws2.socket.restype = ctypes.c_size_t
    ws2.socket.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_int]
    ws2.connect.argtypes = [ctypes.c_size_t, ctypes.c_char_p, ctypes.c_int]
    ws2.send.argtypes = [ctypes.c_size_t, ctypes.c_char_p, ctypes.c_int, ctypes.c_int]
    ws2.recv.argtypes = [ctypes.c_size_t, ctypes.c_char_p, ctypes.c_int, ctypes.c_int]
    ws2.closesocket.argtypes = [ctypes.c_size_t]
    if ws2.WSAStartup(0x202, ctypes.create_string_buffer(512)) != 0:
        raise OSError("WSAStartup failed")
    fd = ws2.socket(1, 1, 0)  # AF_UNIX, SOCK_STREAM
    if fd == ctypes.c_size_t(-1).value:
        raise OSError(f"socket: WSA error {ws2.WSAGetLastError()}")
    try:
        name = str(socket).encode()
        address = (1).to_bytes(2, "little") + name + bytes(108 - len(name))
        if ws2.connect(fd, address, len(address)) != 0:
            raise OSError(f"connect {socket}: WSA error {ws2.WSAGetLastError()}")
        while line:
            sent = ws2.send(fd, line, len(line), 0)
            if sent < 0:
                raise OSError(f"send: WSA error {ws2.WSAGetLastError()}")
            line = line[sent:]
        reply, buffer = b"", ctypes.create_string_buffer(1 << 16)
        while not reply.endswith(b"\n"):
            got = ws2.recv(fd, buffer, len(buffer), 0)
            if got <= 0:
                break
            reply += buffer.raw[:got]
    finally:
        ws2.closesocket(fd)
    return reply.decode(errors="replace").strip()


async def ask_edit(path: Path, text: str) -> str:
    """What the demo's editor does on each edit: the whole text, on one line, to the daemon.
    The answer as it came: `ok ...`, or `error: ...` for text that does not build."""
    socket = daemon_home() / "daemon.sock"
    line = f"edit\t{escape(str(path))}\t{escape(text)}\n".encode()
    try:
        async with asyncio.timeout(REPLY_TIMEOUT):
            if os.name == "nt":
                reply = await asyncio.to_thread(ask_edit_winsock, socket, line)
            else:
                reader, writer = await asyncio.open_unix_connection(str(socket))
                writer.write(line)
                await writer.drain()
                reply = (await reader.readline()).decode(errors="replace").strip()
                writer.close()
    except TimeoutError:
        raise StepError(f"send edit: no reply from {socket} in {REPLY_TIMEOUT}s") from None
    return reply


async def send_edit(path: Path, text: str) -> str:
    reply = await ask_edit(path, text)
    if not reply.startswith("ok"):
        raise StepError(f"send edit: the daemon said {reply!r}")
    return reply


class Logs:
    """The worker logs, read from where the last edit left off."""

    def __init__(self) -> None:
        self.offsets: dict[Path, int] = {}
        self.skip()

    def files(self) -> list[Path]:
        # A worker that was a spare, started ahead of the checkout, keeps the spare's log.
        home = daemon_home()
        return sorted([*home.glob("workers/*/log"), *home.glob("spares/*/log")])

    def skip(self) -> None:
        for log in self.files():
            self.offsets[log] = log.stat().st_size

    def new(self) -> list[str]:
        lines = []
        for log in self.files():
            with log.open("rb") as f:
                f.seek(self.offsets.get(log, 0))
                data = f.read()
            self.offsets[log] = self.offsets.get(log, 0) + len(data)
            lines += data.decode(errors="replace").splitlines()
        return lines


def at_us(line: str) -> int | None:
    found = re.search(r"at_us=(\d+)", line)
    return int(found.group(1)) if found else None


def duration_ms(text: str) -> float:
    """A Rust `Duration` as Debug prints it: 1.2ms, 830.0µs, 2.1s."""
    found = re.match(r"([\d.]+)(ns|µs|us|ms|s)", text)
    if not found:
        return float("nan")
    scale = {"ns": 1e-6, "µs": 1e-3, "us": 1e-3, "ms": 1.0, "s": 1e3}[found.group(2)]
    return float(found.group(1)) * scale


def worker_steps(worker: list[str]) -> dict[str, list[float]]:
    """The steps the worker's log times, each with every value it had in `worker`."""
    steps: dict[str, list[float]] = {}
    for line in worker:
        if line.startswith(("timeline-us:", "apply-us:", "splice-us:")):
            prefix = {"t": "worker.", "a": "worker.apply.", "s": "worker.splice."}[line[0]]
            for step in line.split()[1:]:
                name, _, value = step.partition("=")
                if name != "bytes":
                    steps.setdefault(f"{prefix}{name}", []).append(int(value) / 1e3)
        elif line.startswith("live: edit") and (took_us := re.search(r"took_us=(\d+)", line)):
            # Laying the patch out for the program and leaving it there.
            steps.setdefault("worker.publish", []).append(int(took_us.group(1)) / 1e3)
        elif line.startswith("edit-to-binary:"):
            steps.setdefault("worker.edit-to-binary", []).append(duration_ms(line.split()[1]))
        elif line.startswith("link:") and "spliced" in line:
            steps.setdefault("worker.link", []).append(duration_ms(line.split()[1]))
    return steps


def breakdown(t0: int, reply: int, applied_line: str, worker: list[str]) -> dict[str, float]:
    requests = [it for line in worker if line.startswith("request:") and (it := at_us(line)) and it >= t0]
    published = [it for line in worker if line.startswith("live: edit") and (it := at_us(line)) and it >= t0]
    if not requests or not published:
        raise StepError(f"timing: no `request:` or `live: edit` stamp in the worker log since the edit:\n  " + "\n  ".join(worker[-30:]))
    found = re.search(r" in (\S+) after waiting (\S+) at_us=(\d+)", applied_line)
    if not found:
        raise StepError(f"timing: the program's outcome has no times: {applied_line}")
    took, waited, applied = duration_ms(found.group(1)), duration_ms(found.group(2)), int(found.group(3))
    request, publish = requests[0], published[-1]
    ms = lambda us: us / 1e3
    row = {
        "to-worker": ms(request - t0),
        "worker": ms(publish - request),
        "delivery": ms(applied - publish) - waited - took,
        "wait": waited,
        "apply": took,
        "total": ms(applied - t0),
        "reply": ms(reply - t0),
    }
    # One edit, one build: the last value of each step is this edit's.
    row.update({name: values[-1] for name, values in worker_steps(worker).items()})
    return row


def percentile(values: list[float], p: float) -> float:
    """The value `p` percent of the way up `values`, sorted (as benchhistory.py takes it)."""
    values = sorted(values)
    return values[min(len(values) - 1, int(p / 100 * len(values)))]


def summarize(title: str, rows: list[dict], columns: list[str] = COLUMNS) -> str:
    keys = columns + sorted({k for row in rows for k in row if k not in columns})
    out = [f"### {title}, ms", "", "| step | n | median | p90 | p95 | p99 | min | max |", "|---|---|---|---|---|---|---|---|"]
    for key in keys:
        values = sorted(row[key] for row in rows if isinstance(row.get(key), (int, float)))
        if not values:
            continue
        cells = [statistics.median(values), *(percentile(values, p) for p in (90, 95, 99)), values[0], values[-1]]
        out.append(f"| {key} | {len(values)} | " + " | ".join(f"{it:.2f}" for it in cells) + " |")
    return "\n".join(out)


def spread(values: list[float]) -> str:
    if not values:
        return "none"
    return " ".join(f"p{p} {percentile(values, p):.2f}" for p in (50, 90, 95, 99)) + f" max {max(values):.2f} ms"


class Running:
    """The program's output: each line in order, and when it ran each answer."""

    def __init__(self, lines: asyncio.Queue[str]) -> None:
        self.lines = lines
        self.log: list[str] = []

    async def wait_for(self, step: str, wanted: str) -> str:
        return await wait_for(step, self.lines, wanted, self.log)

    async def runs(self, step: str, value: int) -> int:
        """Waits for the program to run `value` (or newer) and says when it did."""
        pattern = re.compile(r"^answer (\d+) at_us=(\d+)")
        for line in self.log[self.seen :]:
            if (found := pattern.match(line)) and int(found.group(1)) >= value:
                return int(found.group(2))
        while True:
            line = await self.wait_for(step, "answer ")
            if (found := pattern.match(line)) and int(found.group(1)) >= value:
                return int(found.group(2))

    def drain(self) -> None:
        """Takes what the program said so far into the log, waiting for nothing."""
        while not self.lines.empty():
            self.log.append(self.lines.get_nowait())

    def mark(self) -> None:
        self.seen = len(self.log)

    seen = 0


class Launched:
    """A program started: the process, what it says, and when it first ran the answer."""

    def __init__(self, proc: asyncio.subprocess.Process, running: Running, pumper: asyncio.Task, started: int) -> None:
        self.proc, self.running, self.pumper, self.started = proc, running, pumper, started

    async def stop(self) -> None:
        self.pumper.cancel()
        if self.proc.returncode is None:
            self.proc.terminate()
            await self.proc.wait()


async def launch(program: Program, binary: Path, env: dict[str, str]) -> Launched:
    """Starts `binary` and waits for it to run the answer the source has now."""
    proc = await asyncio.create_subprocess_exec(
        str(binary), cwd=program.project, env=dict(env, **program.env),
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT,
    )
    lines: asyncio.Queue[str] = asyncio.Queue()
    pumper = asyncio.create_task(pump(proc.stdout, lines))
    running = Running(lines)
    try:
        await running.wait_for("program start", "ready")
        started = await running.runs("program runs the edited source", program.value)
    except BaseException:
        pumper.cancel()
        proc.terminate()
        await proc.wait()
        raise
    return Launched(proc, running, pumper, started)


async def one_by_one(mode: str, program: Program, running: Running, env: dict[str, str], logs: Logs) -> list[dict[str, float]]:
    rows = []
    # The first edit of a mode is a warm-up: not counted.
    for nth in range(EDITS + 1):
        new = program.edited(program.source.read_text(), program.value, program.value + 1)
        logs.skip()
        running.mark()
        t0 = now_us()
        if mode == "cargo":
            program.source.write_text(new)
            out = await build(f"{mode} edit {nth}", program.project, env)
            said = out.strip().splitlines()[-1] if out.strip() else ""
        else:
            said = await send_edit(program.source, new)
            program.source.write_text(new)
        reply = now_us()
        program.value += 1
        applied = await running.wait_for(f"{mode} edit {nth} applied", "outcome:")
        if "applied" not in applied:
            raise StepError(f"{mode} edit {nth}: the program did not take the patch: {applied}")
        # The new code can run before the apply thread says it was applied.
        await running.runs(f"{mode} edit {nth} runs", program.value)
        # Give the worker the moment it takes to log the rest of the build (the splice may
        # come after the answer).
        await asyncio.sleep(0.3)
        row = breakdown(t0, reply, applied, logs.new())
        print(f"{mode} {nth}: " + " ".join(f"{k}={row[k]:.2f}" for k in COLUMNS) + f"  ({said[-60:]})")
        if nth:
            rows.append(row)
    return rows


async def burst(program: Program, running: Running, logs: Logs, rng: random.Random, nth: int) -> tuple[list[dict[str, float]], dict[str, list[float]]]:
    """Types KEYS keystrokes and times each, as the demo's editor sends them."""
    typed: list[tuple[int, int]] = []  # (value, when typed)
    sent: list[tuple[int, int]] = []  # (value, when its edit was sent)
    replies: list[str] = []
    newest: list[tuple[str, int]] = []
    ready = asyncio.Event()
    done = False

    async def sender() -> None:
        # The demo's `send_as_typed`: one edit on the wire, then the newest text typed.
        while True:
            if not newest:
                if done:
                    return
                await ready.wait()
                ready.clear()
                continue
            text, value = newest.pop()
            newest.clear()
            sent.append((value, now_us()))
            replies.append(await send_edit(program.source, text))
            program.source.write_text(text)

    logs.skip()
    running.mark()
    task = asyncio.create_task(sender())
    text = program.source.read_text()
    for _ in range(KEYS):
        await asyncio.sleep(rng.uniform(0.03, 0.1))
        text = program.edited(text, program.value, program.value + 1)
        program.value += 1
        typed.append((program.value, now_us()))
        newest[:] = [(text, program.value)]
        ready.set()
    done = True
    ready.set()
    async with asyncio.timeout(REPLY_TIMEOUT):
        await task
    last = await running.runs(f"burst {nth} runs the last keystroke", program.value)
    await asyncio.sleep(0.3)
    rows = []
    answers = [
        (int(found.group(1)), int(found.group(2)))
        for line in running.log[running.seen :]
        if (found := re.match(r"^answer (\d+) at_us=(\d+)", line))
    ]
    for value, at in typed:
        ran = min((when for seen, when in answers if seen >= value), default=last)
        send = min((when for has, when in sent if has >= value), default=at)
        rows.append({"total": (ran - at) / 1e3, "queued": (send - at) / 1e3})
    superseded = sum("superseded" in it for it in replies)
    applied = sum(1 for line in running.log[running.seen :] if line.startswith("outcome:") and "applied" in line)
    totals = [row["total"] for row in rows]
    print(
        f"burst {nth}: {len(typed)} keystrokes, {len(sent)} edits sent ({superseded} superseded), {applied} patches applied; "
        f"keystroke to running {spread(totals)}"
    )
    return rows, worker_steps(logs.new())


WORD_ENDS = " \n;,(){}"
TYPOS = 0.03
NEIGHBOURS = {c: row[max(0, i - 1) : i] + row[i + 1 : i + 2] for row in ("qwertyuiop", "asdfghjkl", "zxcvbnm") for i, c in enumerate(row)}


def keystrokes(text: str, ops: list[Op], rng: random.Random) -> Iterator[tuple[str, float]]:
    """The keys of `ops` typed into `text`: for each, the pause before it in seconds and the
    text after it."""
    caret = 0
    last = ""

    def pause() -> float:
        wait = rng.uniform(0.03, 0.1)
        if last and last in WORD_ENDS and rng.random() < 0.3:
            wait += rng.uniform(0.15, 0.6)
        return wait

    def find(s: str) -> int:
        if text.count(s) != 1:
            raise StepError(f"replay: {s!r} is in the text {text.count(s)} times, not once")
        return text.index(s)

    for op, arg in ops:
        match op:
            case "at":
                caret = find(arg) + len(arg)
            case "cut":
                at = find(arg)
                wait = pause() + rng.uniform(0.2, 0.5)  # selecting it
                text, caret = text[:at] + text[at + len(arg) :], at
                yield wait, text
            case "type":
                for key in re.findall(r"\n *|.", arg, re.S):
                    if key.isalpha() and rng.random() < TYPOS:
                        wait = pause()
                        text = text[:caret] + rng.choice(NEIGHBOURS.get(key.lower(), "e")) + text[caret:]
                        yield wait, text
                        text = text[:caret] + text[caret + 1 :]
                        yield rng.uniform(0.1, 0.3), text
                    wait = pause()
                    text, caret = text[:caret] + key + text[caret:], caret + len(key)
                    last = key[-1]
                    yield wait, text
            case "back":
                for _ in range(arg):
                    wait = pause()
                    text, caret = text[: caret - 1] + text[caret:], caret - 1
                    last = ""
                    yield wait, text
            case _:
                raise StepError(f"replay: unknown op {op!r}")


def builds(answer: str) -> bool | None:
    """Whether the daemon's answer to an edit says its text compiles: `error: ` is an error in
    the source, as rustc writes it, and says it does not; one that is ra-jit's own (`cargo: `,
    a function it does not compile, such as `` `main` is not compiled by ra-jit: unsupported:
    ...``, or `failed: `) says nothing against the text, which rustc would build. `no program
    built has <file>` says nothing of the text either way (None): the daemon did not build
    it."""
    if answer.startswith("error: no program built has "):
        return None
    return not answer.startswith("error: ")


def outcome_lines(log: list[str]) -> tuple[list[tuple[int, int]], list[int]]:
    """The patches the program applied (sequence, when) and when it was told to restart."""
    applied, restarts = [], []
    for line in log:
        if found := re.match(r"^outcome: edit (\d+) applied.* at_us=(\d+)", line):
            applied.append((int(found.group(1)), int(found.group(2))))
        elif line.startswith("outcome: restart needed") and (it := at_us(line)):
            restarts.append(it)
    return applied, restarts


async def relaunch(program: Program, live: list[Launched], binary: Path, env: dict[str, str], answered: asyncio.Event | None = None, last: Callable[[], tuple[int, str] | None] = lambda: None) -> tuple[int, int]:
    """Starts the program again, as a launcher that restarts it when told to does: `cargo
    ra-jit build` of what is on disk, once that builds, then the executable to its answer.
    `last` says what was answered last (the key and the answer); a build is tried after an
    answer that is not an error, else after the next; with no answer in 30 s (the typing is
    over) the build's error is the round's. Says the key built and when the build started."""
    failed = None
    while True:
        said = last()
        if said is None or (said[1].startswith("ok") and "superseded" not in said[1]):
            key, started = said[0] if said else -1, now_us()
            try:
                await build("restart: cargo ra-jit build", program.project, env)
                break
            except StepError as error:
                failed = error
        if answered is None:
            raise failed or StepError("restart: the source on disk does not build")
        answered.clear()
        try:
            async with asyncio.timeout(30):
                await answered.wait()
        except TimeoutError:
            raise failed or StepError(f"restart: no answer in 30 s after {said and said[1][:200]!r}") from None
    await live[0].stop()
    live[0] = await launch(program, binary, env)
    return key, started


async def replay(program: Program, live: list[Launched], binary: Path, env: dict[str, str], logs: Logs, rng: random.Random, nth: int, answers: list[dict]) -> tuple[list[dict], collections.Counter, list[dict]]:
    """Types the program's scripts key by key, as the demo's editor sends them, and times each
    key whose text compiles. A program told to restart is started again at once. What the
    daemon answered each edit goes to `answers`."""
    typed: list[tuple[str, int, str]] = []  # (text, when typed, script)
    sent: list[tuple[int, int, int, str]] = []  # (key, when sent, when answered, answer)
    newest: list[tuple[str, int]] = []
    ready, wanted, answered = asyncio.Event(), asyncio.Event(), asyncio.Event()
    restarts: list[dict] = []
    asked_by: list[str] = []  # what asked for each restart: the daemon's answer or the program
    done = False
    restarting = False
    running = live[0].running
    running.drain()
    programs = [(live[0], len(running.log))]  # each program that ran, and where this round's lines start

    async def sender() -> None:
        while True:
            if not newest:
                if done:
                    return
                await ready.wait()
                ready.clear()
                continue
            text, key = newest.pop()
            newest.clear()
            at = now_us()
            reply = await ask_edit(program.source, text)
            sent.append((key, at, now_us(), reply))
            program.source.write_text(text)
            answered.set()
            # The program was linked again: the one running takes no more patches, and is
            # not always told (its executable in target/ is cargo's copy, which only a build
            # through cargo replaces).
            if reply.startswith("ok object"):
                asked_by.append("answer")
                wanted.set()

    async def watcher() -> None:
        # What the program says, as it says it: a restart is seen at once.
        while True:
            running = live[0].running
            try:
                line = await asyncio.wait_for(running.lines.get(), 0.05)
            except TimeoutError:
                continue
            running.log.append(line)
            if line.startswith("outcome: restart needed"):
                asked_by.append("program")
                wanted.set()

    async def restarter() -> None:
        nonlocal restarting
        while True:
            await wanted.wait()
            restarting = True
            wanted.clear()
            key, started = await relaunch(program, live, binary, env, answered, lambda: (sent[-1][0], sent[-1][3]) if sent else None)
            programs.append((live[0], 0))
            restarts.append({"key": key, "started": started, "running": live[0].started, "total": (live[0].started - started) / 1e3})
            print(f"replay {nth}: restarted after key {key} in {restarts[-1]['total']:.0f} ms")
            restarting = False

    start = program.source.read_text()
    logs.skip()
    tasks = [asyncio.create_task(it()) for it in (sender, watcher, restarter)]
    try:
        text = start
        for name, ops in program.scripts(nth):
            for wait, text in keystrokes(text, ops, rng):
                await asyncio.sleep(wait)
                typed.append((text, now_us(), name))
                newest[:] = [(text, len(typed) - 1)]
                ready.set()
        if text != start:
            raise StepError(f"replay {nth}: the scripts of {program.name} do not end where they started")
        done = True
        ready.set()
        async with asyncio.timeout(REPLY_TIMEOUT):
            await tasks[0]
        # The last text is the first, which builds: wait for its patch, or for the restart it
        # asked for (the program sees its executable linked again within 30 ms).
        worker: list[str] = []
        async with asyncio.timeout(900):
            for _ in range(50):
                await asyncio.sleep(0.1)
                worker += logs.new()
                while restarting or wanted.is_set():
                    if tasks[2].done():
                        tasks[2].result()
                    await asyncio.sleep(0.1)
                applied, _ = outcome_lines(live[0].running.log[programs[-1][1] :])
                published = [int(f.group(1)) for line in worker if (f := re.match(r"live: edit (\d+) ", line))]
                kind = sent[-1][3].split()[1].split("=")[0] if sent[-1][3].startswith("ok ") else "error"
                if kind == "unchanged" or (kind == "patch" and published and applied and applied[-1][0] >= published[-1]):
                    break
                if restarts and restarts[-1]["key"] >= sent[-1][0]:
                    break
        await asyncio.sleep(0.3)
    finally:
        for task in tasks:
            task.cancel()
    worker += logs.new()
    live[0].running.drain()
    applied, told = [], 0
    for launched, first in programs:
        got, restarted = outcome_lines(launched.running.log[first:])
        applied += got
        told += len(restarted)
    published = [
        (int(found.group(1)), int(found.group(3)), "restart needed" in found.group(2))
        for line in worker
        if (found := re.match(r"live: edit (\d+) for process \d+: (.*) at_us=(\d+)", line))
    ]

    # When a program first ran the code of each edit sent, or why none did.
    compiles: dict[str, bool] = {}
    refused: set[str] = set()  # texts that compile which the daemon would not build
    ran: dict[int, int] = {}  # sent edit -> when
    counts = collections.Counter(keys=len(typed), sent=len(sent), restarts=len(restarts))
    counts["restarts the program was told"] = told
    counts.update(f"restarts asked by the {it}" for it in asked_by)
    before = None  # when the code of the last edit that built ran
    for at, (key, sent_at, answered_at, reply) in enumerate(sent):
        kind = reply.split()[1].split("=")[0] if reply.startswith("ok ") else "error"
        counts[f"answer {kind}"] += 1
        if reply.startswith("error: no program built has "):
            counts["answer no program built"] += 1
        if kind != "superseded" and (judged := builds(reply)) is not None:
            compiles[typed[key][0]] = judged
            if kind == "error" and judged:
                refused.add(typed[key][0])
        if kind == "unchanged":
            if before is not None:
                ran[at] = max(answered_at, before)
            continue
        if kind == "patch":
            mine = [p for p in published if sent_at <= p[1] <= answered_at + 5000]
            if not mine:
                counts["patch for no program (the executable was linked again)"] += 1
            elif mine[-1][2]:
                counts["patch that asked for a restart"] += 1
            elif when := [w for s, w in applied if s >= mine[-1][0]]:
                ran[at] = min(when)
            else:
                counts["patch not applied"] += 1
        if kind != "error":
            before = ran.get(at)

    # Off the clock: whether each key typed over before it was sent compiles.
    asked = []
    for key, (text, _, _) in enumerate(typed):
        if text not in compiles:
            asked.append(await ask_edit(program.source, text))
            if (judged := builds(asked[-1])) is not None:
                compiles[text] = judged
                if asked[-1].startswith("error") and judged:
                    refused.add(text)
            answers.append({"round": nth, "key": key, "asked after": True, "answer": asked[-1][:200]})
    if asked:
        counts["asked after"] = len(asked)
        asked.append(await send_edit(program.source, start))
        program.source.write_text(start)
        await asyncio.sleep(0.5)
        live[0].running.drain()
        if any(it.startswith("ok object") for it in asked) or outcome_lines(live[0].running.log[programs[-1][1] :])[1][told:]:
            await relaunch(program, live, binary, env)
        logs.skip()

    rows = []
    for key, (text, at, name) in enumerate(typed):
        if text not in compiles:
            counts["keys never judged (no program built)"] += 1
            continue
        if not compiles[text]:
            counts["keys that do not compile"] += 1
            continue
        counts["keys that compile"] += 1
        if text in refused:
            counts["keys that compile and ra-jit refused"] += 1
        later = [(ran[i], sent[i][1], "patch") for i in range(len(sent)) if sent[i][0] >= key and i in ran]
        later += [(it["running"], it["started"], "restart") for it in restarts if it["key"] >= key]
        if not later:
            counts["keys never run"] += 1
            continue
        when, sent_at, how = min(later)
        counts[f"keys run by a {how}"] += 1
        rows.append({"total": (when - at) / 1e3, "queued": (sent_at - at) / 1e3, "by": how, "script": name, "refused": text in refused})
    answers.extend({"round": nth, "key": key, "ms": (answered_at - sent_at) / 1e3, "answer": reply[:200]} for key, sent_at, answered_at, reply in sent)
    print(f"replay {nth}: {len(typed)} keys, {len(sent)} edits sent, {counts['keys that compile']} keys compile, "
          f"{counts['keys that do not compile']} do not; key to running {spread([r['total'] for r in rows])}; "
          + ", ".join(f"{k} {v}" for k, v in sorted(counts.items()) if not k.startswith(("keys that", "keys ", "sent"))))
    return rows, counts, restarts


async def session(program: Program, live: list[Launched], env: dict[str, str], plain_env: dict[str, str], logs: Logs, binary: Path) -> tuple[list[dict], list[dict], list[dict], collections.Counter]:
    """The program's session, an edit at a time: how each was taken, and the restarts (and
    the refused edits, which a launcher restarts too) timed against cargo's build and start
    of the same source."""
    plain_binary = Path(plain_env["CARGO_TARGET_DIR"]) / "debug" / (program.binary + EXE)
    print("restart: plain cargo builds the program once, untimed")
    await plain_build("plain cargo, first build", program.project, plain_env)
    edits, restarts, cargo_runs = [], [], []
    counts = collections.Counter()
    for nth, (kind, label, changes) in enumerate(program.session):
        text = program.source.read_text()
        for old, new in changes:
            if text.count(old) != 1:
                raise StepError(f"restart {nth} ({label}): {old!r} is in {program.source} {text.count(old)} times, not once")
            text = text.replace(old, new)
        running = live[0].running
        logs.skip()
        running.drain()
        running.mark()
        t0 = now_us()
        reply = await ask_edit(program.source, text)
        answered = now_us()
        program.source.write_text(text)
        row = {"edit": label, "kind": kind, "reply": (answered - t0) / 1e3, "answer": reply[:60]}
        if "unchanged" in reply:
            row["outcome"] = "unchanged"
        elif not reply.startswith("ok"):
            row["outcome"] = "refused"
            row["why"] = reply
        else:
            outcome = await running.wait_for(f"restart {nth} ({label})", "outcome:")
            row["outcome"] = "restart" if "restart needed" in outcome else "applied" if "applied" in outcome else "declined"
            if row["outcome"] == "applied":
                row["total"] = (at_us(outcome) - t0) / 1e3
            elif row["outcome"] == "restart":
                row["why"] = outcome.split("restart needed: ", 1)[1].rsplit(" at_us=", 1)[0]
        counts[f"{kind} {row['outcome']}"] += 1
        if row["outcome"] in ("restart", "refused"):
            built = now_us()
            try:
                await build(f"restart {nth}: cargo ra-jit build", program.project, env)
            except StepError as error:
                # The daemon cannot build the source at all: what cargo takes for it is still
                # timed, and the session ends, since every edit after it builds on it.
                row["restart"] = f"failed: {error}"
            linked = now_us()
            if "restart" not in row:
                await live[0].stop()
                live[0] = await launch(program, binary, env)
                restart = {"edit": label, "total": (live[0].started - t0) / 1e3, "edit-reply": (answered - t0) / 1e3,
                           "build": (linked - built) / 1e3, "start": (live[0].started - linked) / 1e3}
                restarts.append(restart)
                row["restart"] = restart["total"]
            c0 = now_us()
            await plain_build(f"restart {nth}: plain cargo build", program.project, plain_env)
            c1 = now_us()
            plain = await launch(program, plain_binary, plain_env)
            await plain.stop()
            cargo_runs.append({"edit": label, "total": (plain.started - c0) / 1e3, "build": (c1 - c0) / 1e3, "start": (plain.started - c1) / 1e3})
            row["cargo run"] = cargo_runs[-1]["total"]
        edits.append(row)
        shown = {k: (f"{v:.1f}" if isinstance(v, float) else v) for k, v in row.items() if k != "answer"}
        print(f"restart {nth}: {shown}")
        if str(row.get("restart", "")).startswith("failed"):
            counts["the daemon could not build the source"] += 1
            break
    return edits, restarts, cargo_runs, counts


async def plain_build(step: str, project: Path, env: dict[str, str]) -> None:
    proc = await asyncio.create_subprocess_exec("cargo", "build", "--locked", cwd=project, env=env,
                                                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT)
    out, _ = await proc.communicate()
    if proc.returncode != 0:
        tail = "\n".join(out.decode(errors="replace").strip().splitlines()[-30:])
        raise StepError(f"{step}: exit code {proc.returncode}\n{tail}")


def run(*cmd: str) -> str:
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=30).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return ""


def host() -> dict:
    """What the numbers were measured on, and whether it ran on its battery."""
    info = {"name": os.environ.get("RA_JIT_BENCH_MACHINE"), "cpu": None, "cores": os.cpu_count(), "memory_gib": None,
            "os": f"{platform.system()} {platform.release()}", "battery": None}
    if sys.platform == "darwin":
        model = run("sysctl", "-n", "hw.model")
        named = re.search(r"Model Name: (.+)", run("system_profiler", "SPHardwareDataType"))
        info["name"] = info["name"] or (f"{named.group(1)} ({model})" if named else model)
        info["cpu"] = run("sysctl", "-n", "machdep.cpu.brand_string") or None
        memory = run("sysctl", "-n", "hw.memsize")
        info["memory_gib"] = round(int(memory) / 2**30) if memory.isdigit() else None
        info["os"] = f"macOS {run('sw_vers', '-productVersion')}"
        batt = run("pmset", "-g", "batt")
        if "AC Power" in batt:
            info["battery"] = "AC"
        elif found := re.search(r"(\d+)%", batt):
            info["battery"] = f"battery {found.group(1)}%"
    else:
        info["name"] = info["name"] or platform.node()
        info["cpu"] = platform.processor() or None
    return info


def metadata(program: str) -> dict:
    server, repo, run_id = (os.environ.get(k) for k in ("GITHUB_SERVER_URL", "GITHUB_REPOSITORY", "GITHUB_RUN_ID"))
    return {
        "program": program,
        "date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "commit": os.environ.get("GITHUB_SHA") or run("git", "-C", str(ROOT), "rev-parse", "HEAD") or None,
        "run_url": f"{server}/{repo}/actions/runs/{run_id}" if server and repo and run_id else None,
        "host": host(),
    }


async def main(program_name: str, modes: list[str]) -> None:
    # Lines as they happen, in order with the errors, also into a pipe.
    if isinstance(sys.stdout, io.TextIOWrapper):
        sys.stdout.reconfigure(line_buffering=True)
    bin_dir = Path(os.environ["RA_JIT_BIN"]).resolve().parent
    work = Path(tempfile.mkdtemp(prefix="ra-jit-live-latency-"))
    target = Path(os.environ.get("LATENCY_TARGET_DIR") or work / "target").resolve()
    meta = metadata(program_name)
    print(f"{meta['host']}")
    live: list[Launched] = []
    results: dict[str, list[dict]] = {}
    counted: dict[str, dict] = {}
    tables = []
    mode, failed = "first build", None
    try:
        program = copy_program(program_name, work)
        env = dict(
            os.environ,
            PATH=f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            CARGO_TARGET_DIR=str(target),
            RA_JIT_STATS="1",
        )
        env.pop("CARGO_INCREMENTAL", None)

        await build("first build", program.project, env)
        await serving(program.project, env)
        program.source.write_text(program.edited(program.source.read_text(), program.value, program.value + 1))
        program.value += 1
        await build("edit before start", program.project, env)
        binary = target / "debug" / (program.binary + EXE)
        print(f"{program.name}: {binary.stat().st_size / 1e6:.1f} MB executable, {len(program.source.read_text().splitlines())}-line {program.source.name}")
        live.append(await launch(program, binary, env))

        logs = Logs()
        rng = random.Random(1)
        for mode in modes:
            running = live[0].running
            match mode:
                case "cargo" | "socket":
                    rows = await one_by_one(mode, program, running, env, logs)
                    tables.append(summarize(f"{program.name}, {mode}: {len(rows)} edits one at a time", rows))
                case "burst":
                    rows, steps = [], {}
                    for nth in range(BURSTS):
                        got, worker = await burst(program, running, logs, rng, nth)
                        rows += got
                        for name, values in worker.items():
                            steps.setdefault(name, []).extend(values)
                        await asyncio.sleep(1)
                    title = f"{program.name}, burst: {len(rows)} keystrokes 30-100 ms apart, keystroke to running"
                    tables.append(summarize(title, rows, ["total", "queued"]))
                    # The worker's steps, per build of the bursts.
                    builds = [{name: values[i] for name, values in steps.items() if i < len(values)} for i in range(max(map(len, steps.values()), default=0))]
                    tables.append(summarize(f"{program.name}, burst: the worker, per build", builds, []))
                case "replay":
                    rows, counts, restarts, answers = [], collections.Counter(), [], []
                    for nth in range(ROUNDS):
                        try:
                            got, how, restarted = await replay(program, live, binary, env, logs, rng, nth, answers)
                        except StepError as error:
                            # The rounds before it are kept.
                            results[mode] = rows
                            counted["replay"] = dict(sorted(counts.items()), restarts=restarts, answers=answers, failed=f"round {nth}: {error}")
                            raise
                        rows += got
                        counts.update(how)
                        restarts += restarted
                        await asyncio.sleep(1)
                    counted["replay"] = dict(sorted(counts.items()), restarts=restarts, answers=answers)
                    title = (f"{program.name}, replay: {counts['keys']} keys typed, {counts['keys that compile']} compile, "
                             f"{counts['sent']} edits sent, {len(restarts)} restarts; key to running")
                    tables.append(summarize(title, rows, ["total", "queued"]))
                    for how in ("patch", "restart"):
                        if part := [row for row in rows if row["by"] == how]:
                            tables.append(summarize(f"{program.name}, replay: the keys run by a {how}", part, ["total", "queued"]))
                    if restarts:
                        tables.append(summarize(f"{program.name}, replay: each restart, `cargo ra-jit build` to the program running", [{"total": it["total"]} for it in restarts], ["total"]))
                    by_script = collections.defaultdict(list)
                    for row in rows:
                        by_script[row["script"]].append(row["total"])
                    tables.append(summarize(f"{program.name}, replay: key to running by script", [{k: v} for k, vs in by_script.items() for v in vs], []))
                case "restart":
                    plain_env = dict(env, CARGO_TARGET_DIR=str(target.with_name(target.name + "-cargo")))
                    edits, rows, cargo_runs, counts = await session(program, live, env, plain_env, logs, binary)
                    results["cargo-run"] = cargo_runs
                    counted["restart"] = dict(sorted(counts.items()), edits=len(edits), session=edits)
                    tables.append(summarize(f"{program.name}, restart: {len(edits)} edits, {len(rows)} asked for a restart; edit to the program started again", rows, ["total", "edit-reply", "build", "start"]))
                    tables.append(summarize(f"{program.name}, restart: the same source by plain cargo, build and start", cargo_runs, ["total", "build", "start"]))
                    applied = [e for e in edits if "total" in e and e["outcome"] == "applied"]
                    tables.append(summarize(f"{program.name}, restart: the session's edits applied live", applied, ["total", "reply"]))
                case _:
                    raise StepError(f"unknown mode {mode!r}: cargo, socket, burst, replay or restart")
            results[mode] = rows
    except StepError as error:
        # The modes before it keep their numbers; the JSON says which mode failed and how.
        failed = error
        counted["failed"] = {"mode": mode, "error": str(error)}
    finally:
        for it in live:
            await it.stop()
        shutil.rmtree(work, ignore_errors=True)

    text = "\n\n".join(tables)
    print(text)
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary, "a") as f:
            f.write(f"{program_name} on {meta['host']['name']} ({meta['host']['cpu']}, {meta['host']['battery'] or 'no battery'})\n\n{text}\n")
    if out := os.environ.get("LATENCY_JSON"):
        Path(out).write_text(json.dumps(dict(meta, modes=results, counts=counted), indent=1))
    if failed:
        raise failed


if __name__ == "__main__":
    args = sys.argv[1:]
    program = "probe"
    if args[:1] == ["--program"]:
        program, args = args[1], args[2:]
    try:
        asyncio.run(main(program, args or ["cargo"]))
    except StepError as error:
        print(f"error: {error}", file=sys.stderr)
        show_daemon_log()
        sys.exit(1)
