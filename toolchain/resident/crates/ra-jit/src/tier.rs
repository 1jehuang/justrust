//! Tiering: a program that takes edits with `ra-jit-live` is sampled while it runs, and the
//! functions it spends its time in are compiled again with LLVM and swapped in as a patch,
//! without a restart (issue #6). See `hir_ty_codegen::tier` for the levels and
//! `docs/tiers.md` for the whole of it.
//!
//! A thread of the worker ([`watch`]) asks each program whose executable the worker linked to
//! sample itself (`sample` in its directory), reads what it sampled (`profile`) a few times a
//! second, puts each address in the function the executable or a patch has there, and asks
//! the worker to compile the hot ones at the next level ([`Ask`]). The worker compiles them
//! beside its builds, keeps the code with the program, and builds a patch, which is published
//! like an edit's. What happened is written for the program to show (`tiers`).
//!
//! `RA_JIT_TIER=off` starts none of it: no program is asked to sample, and every function
//! stays Cranelift's.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use hir_ty_codegen::tier::{Branches, Level};
use ra_jit_live::format::{self, Profile, Sample, TierEvent, TierFunction, Tiers};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::lock;

/// How often the sampler looks at the threads.
const INTERVAL_US: u64 = 2000;
/// How often the profiles are read, and how long the heat of a function is counted over.
const ROUND: Duration = Duration::from_millis(100);
const WINDOW: Duration = Duration::from_secs(1);
/// What makes a function hot: this share of the samples in the window, in thousandths, and
/// at least this many of them.
const HOT_SHARE: u64 = 20;
const HOT_SAMPLES: u64 = 10;
/// What makes a function warm enough to be built into a hot caller.
const WARM_SHARE: u64 = 3;
/// How long code of level 2 counts before level 3 is compiled from what it counted, and how
/// often it has to have been entered by then.
const COUNTING: Duration = Duration::from_secs(1);
const ENTERED: u64 = 50;
/// How many functions are asked for at once.
const AT_ONCE: usize = 4;
/// How many events the program is told of.
const EVENTS: usize = 12;

/// Whether tiering is on: it is unless `RA_JIT_TIER=off`, where ra-jit has LLVM and programs
/// take edits while they run (macOS on AArch64).
pub(crate) fn enabled() -> bool {
    cfg!(all(feature = "llvm", target_os = "macos", target_arch = "aarch64"))
        && std::env::var("RA_JIT_TIER").as_deref() != Ok("off")
}

/// The highest level a function is compiled at: 3, or 2 with `RA_JIT_TIER=2`, where level 2
/// counts nothing, as there is no level 3 to count for.
pub(crate) fn highest() -> Level {
    static HIGHEST: LazyLock<Level> = LazyLock::new(|| match std::env::var("RA_JIT_TIER") {
        Ok(level) if level == "2" => Level::Llvm,
        _ => Level::Pgo,
    });
    *HIGHEST
}

/// What the watcher asks the worker for one program.
pub(crate) struct Ask {
    /// The executable the program runs, as the worker splices it.
    pub(crate) binary: PathBuf,
    pub(crate) wants: Vec<Want>,
    /// Every function that has been hot or warm while the program ran, which may be built
    /// into what is compiled.
    pub(crate) hot: Vec<String>,
    /// The first counter that is free.
    pub(crate) counters: u32,
}

pub(crate) struct Want {
    /// As the object has it, without Mach-O's underscore.
    pub(crate) symbol: String,
    pub(crate) level: Level,
    pub(crate) profile: Option<Branches>,
}

/// What became of one function the worker was asked for.
pub(crate) struct Done {
    pub(crate) symbol: String,
    pub(crate) level: Level,
    pub(crate) outcome: Result<Compiled, String>,
}

pub(crate) struct Compiled {
    pub(crate) took: Duration,
    pub(crate) inlined: usize,
    pub(crate) branches: u32,
    /// The counters it counts in, when it does.
    pub(crate) counters: Option<(u32, u32)>,
}

/// One program the watcher knows.
struct Watched {
    binary: PathBuf,
    dir: PathBuf,
    pid: u32,
    started: u64,
    symbols: Symbols,
    /// Where each function a patch replaced starts in the room for patches, and where the
    /// executable has it; and where the code of the patches ends.
    placed: Vec<(u64, u64)>,
    placed_end: u64,
    last: Option<Profile>,
    /// What each round took, for the window: samples by function, all samples, the
    /// sampler's time and the time it ran.
    rounds: VecDeque<(Instant, FxHashMap<u64, u64>, u64, u64, u64)>,
    functions: FxHashMap<u64, Function>,
    /// The functions that have been hot or warm, by their address in the executable.
    ever_hot: FxHashSet<u64>,
    events: VecDeque<TierEvent>,
    counters: u32,
    /// Whether the worker has an [`Ask`] of this program it has not answered.
    asking: bool,
    /// Counters as the program last reported them.
    counted: FxHashMap<u32, u64>,
}

#[derive(Default)]
struct Function {
    level: Option<Level>,
    /// The level being compiled, if one is.
    compiling: Option<Level>,
    /// When it reached its level.
    since: Option<Instant>,
    counters: Option<(u32, u32)>,
    branches: u32,
    /// Why it stays where it is, until an edit: it can't be compiled higher, or it is not
    /// the worker's to compile.
    stuck: Option<String>,
    /// Not the worker's at all (rustc's code), or of the standard library.
    theirs: bool,
}

/// The functions of an executable, by where they start.
#[derive(Default)]
struct Symbols {
    starts: Vec<(u64, String)>,
    text: (u64, u64),
}

impl Symbols {
    fn read(binary: &Path) -> Symbols {
        use object::{Object as _, ObjectSection as _, ObjectSymbol as _};
        let Ok(data) = std::fs::read(binary) else { return Symbols::default() };
        let Ok(file) = object::File::parse(&*data) else { return Symbols::default() };
        let text = file
            .section_by_name("__text")
            .map(|it| (it.address(), it.address() + it.size()))
            .unwrap_or_default();
        let mut starts: Vec<(u64, String)> = file
            .symbols()
            .filter(|it| it.kind() == object::SymbolKind::Text && it.is_definition())
            .filter(|it| (text.0..text.1).contains(&it.address()))
            .filter_map(|it| Some((it.address(), it.name().ok()?.to_owned())))
            // The assembler's labels, which name no function.
            .filter(|(_, name)| {
                !(name.is_empty() || name.starts_with("ltmp") || name.starts_with("l_"))
            })
            .collect();
        starts.sort_unstable();
        starts.dedup_by_key(|it| it.0);
        Symbols { starts, text }
    }

    /// The start of the function at `pc` of the executable.
    fn at(&self, pc: u64) -> Option<u64> {
        if !(self.text.0..self.text.1).contains(&pc) {
            return None;
        }
        let after = self.starts.partition_point(|it| it.0 <= pc);
        Some(self.starts.get(after.checked_sub(1)?)?.0)
    }

    fn name(&self, start: u64) -> Option<&str> {
        let at = self.starts.binary_search_by_key(&start, |it| it.0).ok()?;
        Some(&self.starts[at].1)
    }
}

struct State {
    programs: FxHashMap<[u8; 16], Watched>,
    /// The UUID of each executable the worker links, as last read, by its time and length.
    uuids: FxHashMap<PathBuf, (SystemTime, u64, Option<[u8; 16]>)>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| {
    Mutex::new(State { programs: FxHashMap::default(), uuids: FxHashMap::default() })
});

/// Watches the programs whose executables the worker links, on a thread of its own, and
/// hands `ask` what to compile.
pub(crate) fn watch(ask: impl Fn(Ask) + Send + 'static) {
    eprintln!("tier: on, sampling the programs of this worker that take edits");
    std::thread::Builder::new()
        .name("ra-jit-tier".to_owned())
        .spawn(move || {
            loop {
                std::thread::sleep(ROUND);
                for asked in round() {
                    ask(asked);
                }
            }
        })
        .expect("spawning the thread that tiers");
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |it| it.as_millis() as u64)
}

/// The name a function is shown by: demangled, without the hash. An instance the daemon
/// named itself (`ra-jit::<name>::<number>`) is shown by its name, and a function of a crate
/// rustc built nothing of (`ra-jit.<crate id>.<path>`) by its path, the crate written without
/// its id.
fn shown(symbol: &str) -> String {
    let symbol = symbol.strip_prefix('_').unwrap_or(symbol);
    let hex = |it: &str| !it.is_empty() && it.bytes().all(|b| b.is_ascii_hexdigit());
    if let Some((id, path)) = symbol.strip_prefix("ra-jit.").and_then(|it| it.split_once('.'))
        && hex(id)
    {
        let path = path.rsplit_once('.').filter(|it| hex(it.1)).map_or(path, |it| it.0);
        return path.replace(&format!("[{id}]"), "");
    }
    let own =
        ["ra-jit::", "ra_jit::", "ra-jit.patch::"].iter().find_map(|it| symbol.strip_prefix(it));
    if let Some(own) = own {
        let numbered = |it: &(&str, &str)| {
            let number = it.1.strip_prefix('n').unwrap_or(it.1);
            !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
        };
        return own.rsplit_once("::").filter(numbered).map_or(own, |it| it.0).to_owned();
    }
    format!("{:#}", rustc_demangle::demangle(symbol))
}

/// The executables the worker links that a program runs, by their UUID.
fn owned(state: &mut State) -> Vec<([u8; 16], PathBuf)> {
    let binaries: Vec<PathBuf> = lock(&crate::LINKED).keys().cloned().collect();
    let mut found = Vec::new();
    for binary in binaries {
        let Ok(meta) = std::fs::metadata(&binary) else { continue };
        let stamp = (meta.modified().unwrap_or(UNIX_EPOCH), meta.len());
        let known = state.uuids.get(&binary).filter(|it| (it.0, it.1) == stamp);
        let uuid = match known {
            Some(known) => known.2,
            None => {
                let uuid = uuid_of(&binary);
                state.uuids.insert(binary.clone(), (stamp.0, stamp.1, uuid));
                uuid
            }
        };
        if let Some(uuid) = uuid {
            found.push((uuid, binary));
        }
    }
    found
}

fn uuid_of(binary: &Path) -> Option<[u8; 16]> {
    use object::Object as _;
    let data = std::fs::read(binary).ok()?;
    object::File::parse(&*data).ok()?.mach_uuid().ok()?
}

/// One look at every program: what it sampled since the last, and what to ask for.
fn round() -> Vec<Ask> {
    let mut state = lock(&STATE);
    let mut asks = Vec::new();
    let mut owned = owned(&mut state);
    // A splice writes the executable again, and a look while it does reads no UUID: what is
    // known of a program stays for as long as it runs.
    for (uuid, program) in &state.programs {
        if !owned.iter().any(|it| it.0 == *uuid) {
            owned.push((*uuid, program.binary.clone()));
        }
    }
    for (uuid, binary) in owned {
        let Some(process) = crate::live::running(&uuid) else {
            state.programs.remove(&uuid);
            continue;
        };
        let Some(dir) = format::dir(&uuid) else { continue };
        let program = state.programs.entry(uuid).or_insert_with(|| Watched::new(binary, dir));
        if program.pid != process.pid {
            *program = Watched::new(program.binary.clone(), program.dir.clone());
            program.pid = process.pid;
            program.started = process.started;
        }
        if program.symbols.starts.is_empty() {
            program.symbols = Symbols::read(&program.binary);
        }
        program.ask_to_sample();
        program.read();
        if let Some(ask) = program.next() {
            asks.push(ask);
        }
        program.publish();
    }
    asks
}

impl Watched {
    fn new(binary: PathBuf, dir: PathBuf) -> Watched {
        Watched {
            symbols: Symbols::read(&binary),
            binary,
            dir,
            pid: 0,
            started: 0,
            placed: Vec::new(),
            placed_end: 0,
            last: None,
            rounds: VecDeque::new(),
            functions: FxHashMap::default(),
            ever_hot: FxHashSet::default(),
            events: VecDeque::new(),
            counters: 0,
            asking: false,
            counted: FxHashMap::default(),
        }
    }

    fn ask_to_sample(&self) {
        let sample = Sample {
            interval_us: INTERVAL_US,
            counters: u64::from(self.counters),
            reader: std::process::id(),
        };
        let path = self.dir.join(format::SAMPLE);
        let text = sample.encode();
        if std::fs::read_to_string(&path).ok().as_deref() != Some(&*text) {
            let partial = self.dir.join(format!("{}.{}", format::SAMPLE, std::process::id()));
            _ = std::fs::write(&partial, text).and_then(|()| std::fs::rename(&partial, &path));
        }
    }

    /// The start, in the executable, of the function at `pc`: of the executable's code, or
    /// of the code of a patch that replaced it.
    fn function_at(&self, pc: u64) -> Option<u64> {
        if let Some(start) = self.symbols.at(pc) {
            return Some(start);
        }
        if pc >= self.placed_end {
            return None;
        }
        let after = self.placed.partition_point(|it| it.0 <= pc);
        Some(self.placed.get(after.checked_sub(1)?)?.1)
    }

    /// Takes in what the program sampled since the last look.
    fn read(&mut self) {
        let Ok(text) = std::fs::read_to_string(self.dir.join(format::PROFILE)) else { return };
        let Some(profile) = Profile::decode(&text) else { return };
        if profile.pid != self.pid {
            return;
        }
        let last = self.last.take().filter(|it| it.started == profile.started);
        if last.as_ref().is_some_and(|it| it.at == profile.at) {
            self.last = last;
            return;
        }
        let before: FxHashMap<u64, u64> =
            last.as_ref().map(|it| it.pcs.iter().copied().collect()).unwrap_or_default();
        let mut by_function: FxHashMap<u64, u64> = FxHashMap::default();
        for &(pc, count) in &profile.pcs {
            let new = count.saturating_sub(before.get(&pc).copied().unwrap_or(0));
            if new == 0 {
                continue;
            }
            if let Some(start) = self.function_at(pc) {
                *by_function.entry(start).or_default() += new;
            }
        }
        let (samples, sampler, wall) = match &last {
            Some(last) => (
                profile.samples.saturating_sub(last.samples),
                profile.sampler_ns.saturating_sub(last.sampler_ns),
                profile.at.saturating_sub(last.at) * 1_000_000,
            ),
            None => (
                profile.samples,
                profile.sampler_ns,
                profile.at.saturating_sub(profile.started) * 1_000_000,
            ),
        };
        let now = Instant::now();
        self.rounds.push_back((now, by_function, samples, sampler, wall));
        while self.rounds.front().is_some_and(|it| now - it.0 > WINDOW) {
            self.rounds.pop_front();
        }
        self.counted = profile.counters.iter().copied().collect();
        self.last = Some(profile);
    }

    /// Samples by function over the window, the hottest first, with all of them.
    fn heat(&self) -> (Vec<(u64, u64)>, u64) {
        let mut by_function: FxHashMap<u64, u64> = FxHashMap::default();
        let mut total = 0;
        for (_, functions, samples, _, _) in &self.rounds {
            total += samples;
            for (&start, &count) in functions {
                *by_function.entry(start).or_default() += count;
            }
        }
        let mut heat: Vec<(u64, u64)> = by_function.into_iter().collect();
        heat.sort_unstable_by_key(|it| (std::cmp::Reverse(it.1), it.0));
        (heat, total)
    }

    fn event(&mut self, text: String) {
        eprintln!("tier: {text}");
        self.events.push_back(TierEvent { at: now_ms(), text });
        while self.events.len() > EVENTS {
            self.events.pop_front();
        }
    }

    /// What to compile next, if anything.
    fn next(&mut self) -> Option<Ask> {
        if self.asking {
            return None;
        }
        let (heat, total) = self.heat();
        let share = |count: u64| (count * 1000).checked_div(total).unwrap_or(0);
        for &(start, count) in &heat {
            if share(count) >= WARM_SHARE {
                self.ever_hot.insert(start);
            }
        }
        let mut wants = Vec::new();
        for &(start, count) in &heat {
            if wants.len() == AT_ONCE {
                break;
            }
            let Some(symbol) = self.symbols.name(start).map(str::to_owned) else { continue };
            let function = self.functions.entry(start).or_default();
            // What hands out the counters is called by the code that counts, and is
            // ra-jit-live's.
            if symbol.ends_with(hir_ty_codegen::tier::COUNTERS) {
                function.theirs = true;
                function.stuck = Some("ra-jit-live's, which counts for level 2".to_owned());
            }
            if function.stuck.is_some() || function.theirs || function.compiling.is_some() {
                continue;
            }
            match function.level {
                None | Some(Level::Cranelift)
                    if share(count) >= HOT_SHARE && count >= HOT_SAMPLES =>
                {
                    function.compiling = Some(Level::Llvm);
                    wants.push(Want { symbol, level: Level::Llvm, profile: None });
                }
                _ => {}
            }
        }
        // Code that counts goes to level 3 once it has counted for long enough, hot or not.
        let counted = &self.counted;
        for (&start, function) in &mut self.functions {
            if wants.len() == AT_ONCE || highest() < Level::Pgo {
                break;
            }
            let (Some(Level::Llvm), None, Some((first, _)), Some(since)) =
                (function.level, function.compiling, function.counters, function.since)
            else {
                continue;
            };
            let count = |at: u32| counted.get(&at).copied().unwrap_or(0);
            if since.elapsed() < COUNTING || count(first) < ENTERED {
                continue;
            }
            let Some(symbol) = self.symbols.name(start).map(str::to_owned) else { continue };
            let taken = (0..function.branches)
                .map(|nth| [count(first + 1 + 2 * nth), count(first + 2 + 2 * nth)])
                .collect();
            let profile = Branches { entered: count(first), taken };
            function.compiling = Some(Level::Pgo);
            wants.push(Want { symbol, level: Level::Pgo, profile: Some(profile) });
        }
        if wants.is_empty() {
            return None;
        }
        for want in &mut wants {
            want.symbol = want.symbol.strip_prefix('_').unwrap_or(&want.symbol).to_owned();
        }
        self.asking = true;
        let hot = self
            .ever_hot
            .iter()
            .filter_map(|it| self.symbols.name(*it))
            .map(|it| it.strip_prefix('_').unwrap_or(it).to_owned())
            .collect();
        Some(Ask { binary: self.binary.clone(), wants, hot, counters: self.counters })
    }

    /// Writes what the program is told of its tiers.
    fn publish(&self) {
        let (heat, total) = self.heat();
        let share = |count: u64| (count * 1000).checked_div(total).unwrap_or(0);
        let (sampler, wall) = self.rounds.iter().fold((0, 0), |(s, w), it| (s + it.3, w + it.4));
        let mut functions: Vec<TierFunction> = heat
            .iter()
            .take(16)
            .map(|&(start, count)| {
                let function = self.functions.get(&start);
                let name = self.symbols.name(start).map(shown).unwrap_or_default();
                let level = match function {
                    Some(it) if it.theirs => 0,
                    Some(it) => it.level.unwrap_or(Level::Cranelift).number(),
                    None => 1,
                };
                let note = match function {
                    Some(Function { compiling: Some(level), .. }) => {
                        format!("compiling {}", format::level_name(level.number()))
                    }
                    Some(Function { stuck: Some(why), .. }) => why.clone(),
                    Some(Function { level: Some(Level::Llvm), counters: Some(_), .. }) => {
                        "counting branches".to_owned()
                    }
                    _ => String::new(),
                };
                TierFunction { level, samples: count, share: share(count), name, note }
            })
            .collect();
        // What is tiered stays in view once it has cooled down.
        for (start, function) in &self.functions {
            let tiered = function.level.is_some_and(|it| it > Level::Cranelift);
            if tiered && !heat.iter().any(|it| it.0 == *start) {
                functions.push(TierFunction {
                    level: function.level.unwrap_or(Level::Cranelift).number(),
                    samples: 0,
                    share: 0,
                    name: self.symbols.name(*start).map(shown).unwrap_or_default(),
                    note: "not run of late".to_owned(),
                });
            }
        }
        let tiers = Tiers {
            at: now_ms(),
            window_ms: WINDOW.as_millis() as u64,
            samples: total,
            sampler_permille: (sampler * 1000).checked_div(wall).unwrap_or(0),
            functions,
            events: self.events.iter().cloned().collect(),
        };
        let path = self.dir.join(format::TIERS);
        let partial = self.dir.join(format!("{}.{}", format::TIERS, std::process::id()));
        _ = std::fs::write(&partial, tiers.encode())
            .and_then(|()| std::fs::rename(&partial, &path));
    }

    fn by_symbol(&mut self, symbol: &str) -> Option<(u64, &mut Function)> {
        let start = self
            .symbols
            .starts
            .iter()
            .find(|it| it.1.strip_prefix('_').unwrap_or(&it.1) == symbol)?
            .0;
        Some((start, self.functions.entry(start).or_default()))
    }
}

fn with_binary(binary: &Path, work: impl FnOnce(&mut Watched)) {
    let mut state = lock(&STATE);
    if let Some(program) = state.programs.values_mut().find(|it| it.binary == binary) {
        work(program);
    }
}

/// Takes what the worker made of an [`Ask`].
pub(crate) fn done(binary: &Path, done: Vec<Done>, counters: u32) {
    with_binary(binary, |program| {
        program.asking = false;
        program.counters = program.counters.max(counters);
        for Done { symbol, level, outcome } in done {
            let name = shown(&symbol);
            let Some((_, function)) = program.by_symbol(&symbol) else { continue };
            function.compiling = None;
            let text = match outcome {
                Ok(compiled) => {
                    function.level = Some(level);
                    function.since = Some(Instant::now());
                    function.counters = compiled.counters;
                    function.branches = compiled.branches;
                    let mut text = format!(
                        "{name}: {} in {} ms",
                        format::level_name(level.number()),
                        compiled.took.as_millis()
                    );
                    if compiled.inlined != 0 {
                        text.push_str(&format!(", {} functions built in", compiled.inlined));
                    }
                    if level == Level::Pgo {
                        text.push_str(&format!(", {} branches weighed", compiled.branches));
                    }
                    text
                }
                Err(why) => {
                    function.stuck = Some(why.clone());
                    format!(
                        "{name}: stays at {}: {why}",
                        format::level_name(function.level.unwrap_or(Level::Cranelift).number())
                    )
                }
            };
            program.event(text);
        }
    });
}

/// Notes that the worker can't compile `symbol` higher, as it is not of the code it built
/// (rustc's), or of the standard library, which is built into its hot callers instead.
pub(crate) fn theirs(binary: &Path, symbol: &str, why: &str) {
    with_binary(binary, |program| {
        if let Some((_, function)) = program.by_symbol(symbol) {
            function.compiling = None;
            function.theirs = true;
            function.stuck = Some(why.to_owned());
        }
    });
}

/// Notes that an edit, or a full link, sent `symbol` back to level 1: it starts over, and is
/// compiled again once it is hot again.
pub(crate) fn demoted(binary: &Path, symbol: &str, why: &str) {
    with_binary(binary, |program| {
        let name = shown(symbol);
        let Some((start, function)) = program.by_symbol(symbol) else { return };
        *function = Function::default();
        program.ever_hot.remove(&start);
        program.rounds.iter_mut().for_each(|it| _ = it.1.remove(&start));
        program.event(format!("{name}: back to Cranelift: {why}"));
    });
}

/// Notes that no patch could hold the code `symbol` was compiled to: it is back at `level`
/// and stays there until an edit.
pub(crate) fn refused(binary: &Path, symbol: &str, level: Level, why: &str) {
    with_binary(binary, |program| {
        let name = shown(symbol);
        let Some((_, function)) = program.by_symbol(symbol) else { return };
        function.level = (level > Level::Cranelift).then_some(level);
        function.stuck = Some(why.to_owned());
        program.event(format!("{name}: stays at {}: {why}", format::level_name(level.number())));
    });
}

/// Notes that an [`Ask`] could not be taken up at all, and why.
pub(crate) fn declined(binary: &Path, why: &str) {
    with_binary(binary, |program| {
        program.asking = false;
        for function in program.functions.values_mut() {
            if function.compiling.take().is_some() {
                function.stuck = Some(why.to_owned());
            }
        }
    });
}

/// Notes where a patch laid out for the program with `pid` put the functions it replaced:
/// `moved` has where each starts in the patch and where the executable has it.
pub(crate) fn placed(uuid: [u8; 16], pid: u32, mut moved: Vec<(u64, u64)>, end: u64) {
    let mut state = lock(&STATE);
    let Some(program) = state.programs.get_mut(&uuid).filter(|it| it.pid == pid) else { return };
    if std::env::var_os("RA_JIT_DUMP_TIER").is_some() {
        for (new, old) in &moved {
            let name = program.symbols.name(*old).unwrap_or("?");
            eprintln!("tier: placed {new:#x} for {old:#x} {name}");
        }
    }
    program.placed.append(&mut moved);
    program.placed.sort_unstable();
    program.placed_end = program.placed_end.max(end);
}

#[cfg(test)]
mod tests {
    use super::shown;

    #[test]
    fn a_function_is_shown_by_its_path_without_ids_or_hashes() {
        let own = "_ra-jit.607a16ec818c5cec.lifebench[607a16ec818c5cec]::next_age";
        assert_eq!(shown(own), "lifebench::next_age");
        let method = "ra-jit.607a16ec818c5cec.<lifebench[607a16ec818c5cec]::Life>::step";
        assert_eq!(shown(method), "<lifebench::Life>::step");
        assert_eq!(shown("ra-jit.1f.app[1f]::main::helper.9c0e"), "app::main::helper");
        assert_eq!(shown("ra-jit.1f.helper.9c0e"), "helper");
        assert_eq!(shown("_ra-jit::next_age::12"), "next_age");
        assert_eq!(shown("ra-jit.patch::next_age::3"), "next_age");
    }
}
