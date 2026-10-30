//! The package of what a debugger reads of a program that keeps it out of the executable
//! (`split-debuginfo = "packed"`): the `.dwp` next to it. rustc wrote one for the program it
//! linked; the daemon's program has the daemon's code as well, described in its object.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use thorin::{DwarfPackage, MissingReferencedObjectBehaviour, Session};
use typed_arena::Arena;

/// Where a build keeps the package of a program.
pub(crate) fn beside(executable: &Path) -> PathBuf {
    executable.with_extension("dwp")
}

/// Writes at `package` what a debugger reads of `executable`: the entries of the objects the
/// executable names, and those of `rustcs`, the package rustc wrote for the program it linked.
/// The objects rustc kept the entries of may be gone once it has packed them, so its package
/// is read first and an object is read for what is not in it.
pub(crate) fn pack(executable: &Path, rustcs: &Path, package: &Path) -> anyhow::Result<()> {
    let files = Files { data: Arena::new(), relocations: Arena::new() };
    let mut packed = DwarfPackage::new(&files);
    if rustcs.exists() {
        packed.add_input_object(rustcs).with_context(|| format!("reading {}", rustcs.display()))?;
    }
    packed
        .add_executable(executable, MissingReferencedObjectBehaviour::Skip)
        .with_context(|| format!("reading what {} names", executable.display()))?;
    let object = packed.finish().context("a part of the program has its entries nowhere")?;
    let bytes = object.write().context("writing the package")?;
    let new = package.with_extension(format!("{}.new", std::process::id()));
    std::fs::write(&new, bytes)?;
    std::fs::rename(&new, package)?;
    Ok(())
}

/// Whether the package at `package` was written after everything it is made from.
pub(crate) fn is_current(package: &Path, from: &[&Path]) -> bool {
    let written = |path: &Path| std::fs::metadata(path).and_then(|it| it.modified()).ok();
    let Some(packed) = written(package) else { return false };
    from.iter().all(|it| written(it).is_none_or(|it| it <= packed))
}

/// The files a package is made from, kept for as long as it is being made.
struct Files<Relocations> {
    data: Arena<Vec<u8>>,
    relocations: Arena<Relocations>,
}

impl<Relocations> Session<Relocations> for Files<Relocations> {
    fn alloc_data(&self, data: Vec<u8>) -> &[u8] {
        self.data.alloc(data)
    }

    fn alloc_relocation(&self, data: Relocations) -> &Relocations {
        self.relocations.alloc(data)
    }

    fn read_input(&self, path: &Path) -> std::io::Result<&[u8]> {
        Ok(self.data.alloc(std::fs::read(path)?))
    }
}
