//! `ra-jit mir <project> --crate <name> [--level N] [--time]`: the MIR the code generator
//! compiles, of every function of a crate and of the closures in them, written as rustc's
//! `-Zunpretty=mir` writes its own. `harness/mirparity.py` compares the two.
//!
//! `--level` is the MIR optimization level (`RA_JIT_MIR_OPT`): 0, the default, writes the
//! runtime body, 1 and 2 that body after rustc's passes at its `-Zmir-opt-level` 1 and 2.
//! `--time` says on stderr how long the bodies took to build and to optimize.

use std::{
    fmt::Write as _,
    path::Path,
    time::{Duration, Instant},
};

use anyhow::Context;
use hir_def::{FunctionId, ItemContainerId, Lookup};
use hir_ty::{
    InferBodyId,
    db::{HirDatabase, InternedClosureId},
    mir::{MirBody, MirOptLevel, closure_path_suffix, impl_written_at, print_mir_body},
};
use ide_db::RootDatabase;
use rustc_hash::FxHashSet;

use crate::{
    Args, Parts, cargo_config, crates_named, declared_in, load, load_config, process_wide,
};

pub(crate) fn command(mut args: impl Iterator<Item = String>) -> anyhow::Result<()> {
    let mut project = None;
    let mut krate = None;
    let mut level = MirOptLevel::Off;
    let mut time = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--crate" => krate = Some(args.next().context("--crate needs a name")?),
            "--level" => {
                let number = args.next().context("--level needs a number")?;
                level = number
                    .parse()
                    .ok()
                    .and_then(MirOptLevel::from_number)
                    .context("--level is 0, 1 or 2")?;
            }
            "--time" => time = true,
            _ if project.is_none() => project = Some(arg),
            _ => anyhow::bail!("unexpected argument {arg}"),
        }
    }
    let project = project.context("usage: ra-jit mir <project> --crate <name> [--level N]")?;
    let krate = krate.context("--crate is needed")?;
    let asked = [project.clone(), "--crate".into(), krate.clone(), "--out".into(), "none".into()];
    let mut args = Args::parse(asked.into_iter())?;
    process_wide();
    let (db, vfs, checkouts) = load(
        &[Path::new(&project)],
        &cargo_config(&args),
        load_config(),
        args.selection.clone(),
        None,
        None,
    )?;
    let Parts { root, base, members, debug, .. } = checkouts.parts(&args.project)?;
    (args.checkout, args.base, args.members, args.debug) = (root, base, members, debug);
    hir::attach_db(&db, || {
        let mut dump =
            Dump { db: &db, vfs: &vfs, level, out: String::new(), timing: Timing::default() };
        for krate in crates_named(&db, &krate, &args)? {
            let mut functions = declared_in(&db, krate).functions;
            functions.sort_by_key(|&it| dump.position(it));
            for func in functions {
                dump.function(func);
            }
        }
        print!("{}", dump.out);
        if time {
            let Timing { bodies, built, optimized } = dump.timing;
            eprintln!(
                "mir: {bodies} bodies, built in {built:.1?}, optimized at level {} in {optimized:.1?}",
                level.number(),
            );
        }
        Ok(())
    })
}

#[derive(Default)]
struct Timing {
    bodies: usize,
    built: Duration,
    optimized: Duration,
}

struct Dump<'a> {
    db: &'a RootDatabase,
    vfs: &'a vfs::Vfs,
    level: MirOptLevel,
    out: String,
    timing: Timing,
}

impl<'a> Dump<'a> {
    /// Where a function is written: its file and offset, for the order rustc writes them in.
    fn position(&self, func: FunctionId) -> (String, u32) {
        let Some(source) = hir::HasSource::source(hir::Function::from(func), self.db) else {
            return (String::new(), 0);
        };
        let range = source.syntax().original_file_range_rooted(self.db);
        let file = range.file_id.file_id(self.db);
        (self.vfs.file_path(file).to_string(), range.range.start().into())
    }

    /// `file:line` of where an `impl` is written, as rustc's `<impl at file:line:col: ..>`.
    fn impl_place(&self, id: hir_def::ImplId) -> String {
        let (file, at) = impl_written_at(self.db, id);
        let line = ide_db::line_index(self.db, file).line_col(at).line + 1;
        let path = self.vfs.file_path(file).to_string();
        let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        format!("{name}:{line}")
    }

    fn path(&self, func: FunctionId) -> String {
        let function = hir::Function::from(func);
        let mut path: Vec<String> = function
            .module(self.db)
            .path_to_root(self.db)
            .into_iter()
            .rev()
            .filter_map(|it| it.name(self.db).map(|it| it.as_str().to_owned()))
            .collect();
        match func.lookup(self.db).container {
            ItemContainerId::ImplId(id) => path.push(format!("<impl at {}>", self.impl_place(id))),
            ItemContainerId::TraitId(id) => {
                path.push(hir::Trait::from(id).name(self.db).as_str().to_owned())
            }
            _ => (),
        }
        path.push(function.name(self.db).as_str().to_owned());
        path.join("::")
    }

    fn body(&mut self, def: InferBodyId<'a>) -> Option<&'a MirBody<'a>> {
        let db: &'a dyn HirDatabase = self.db;
        let start = Instant::now();
        let runtime = db.runtime_mir_body(def);
        self.timing.built += start.elapsed();
        if let Err(error) = &runtime {
            let _ = writeln!(self.out, "// blaze: no MIR: {error:?}");
            return None;
        }
        self.timing.bodies += 1;
        if self.level == MirOptLevel::Off {
            return runtime.ok();
        }
        let start = Instant::now();
        let optimized = db.optimized_mir_body(def, self.level);
        self.timing.optimized += start.elapsed();
        optimized.ok()
    }

    fn closure_body(&mut self, closure: InternedClosureId<'a>) -> Option<&'a MirBody<'a>> {
        let db: &'a dyn HirDatabase = self.db;
        let start = Instant::now();
        let runtime = db.runtime_mir_body_for_closure(closure);
        self.timing.built += start.elapsed();
        if let Err(error) = &runtime {
            let _ = writeln!(self.out, "// blaze: no MIR: {error:?}");
            return None;
        }
        self.timing.bodies += 1;
        if self.level == MirOptLevel::Off {
            return runtime.ok();
        }
        let start = Instant::now();
        let optimized = db.optimized_mir_body_for_closure(closure, self.level);
        self.timing.optimized += start.elapsed();
        optimized.ok()
    }

    fn function(&mut self, func: FunctionId) {
        let path = self.path(func);
        // A trait method without a default has no body.
        if let ItemContainerId::TraitId(_) = func.lookup(self.db).container
            && !hir::Function::from(func).has_body(self.db)
        {
            return;
        }
        let def = InferBodyId::DefWithBodyId(func.into());
        let Some(body) = self.body(def) else {
            let _ = writeln!(self.out, "// blaze: of fn {path}\n");
            return;
        };
        let text = print_mir_body(self.db, body, &path);
        self.out.push_str(&text);
        self.out.push('\n');
        let mut seen = FxHashSet::default();
        self.closures(&path, &body.closures, &mut seen);
    }

    fn closures(
        &mut self,
        path: &str,
        closures: &[InternedClosureId<'a>],
        seen: &mut FxHashSet<InternedClosureId<'a>>,
    ) {
        for &closure in closures {
            if !seen.insert(closure) {
                continue;
            }
            let db: &dyn HirDatabase = self.db;
            let suffix = closure_path_suffix(db, closure).unwrap_or_else(|| "::{closure}".into());
            let name = format!("{path}{suffix}");
            let Some(body) = self.closure_body(closure) else {
                let _ = writeln!(self.out, "// blaze: of fn {name}\n");
                continue;
            };
            let text = print_mir_body(self.db, body, &name);
            self.out.push_str(&text);
            self.out.push('\n');
            self.closures(path, &body.closures, seen);
        }
    }
}
