//! The splice of a wasm module: the module of the last full link, written again with the
//! functions of the patch in it.
//!
//! Wasm code names what it calls by index, not by address, so nothing has to jump: the body
//! of a function the patch replaces goes where the old body was, under the same index, and
//! every caller, table slot and export is as it was. A function the module does not have
//! gets the index after the last, and a slot behind the last of the table when its address
//! is taken. A static with another value is initialised again where it is by a data segment
//! after the linker's, when it is no larger; new data goes into pages added to the end of
//! the memory the module starts with, which nothing of the program has claimed: its heap is
//! what the linker left of the first pages and what `memory.grow` adds behind these.
//!
//! The linker has to be told to keep its symbol table (`--emit-relocs`): without it a module
//! names its functions for a debugger at most, and its data not at all. The module that is
//! written has neither that table nor the relocations, which the splice does not keep right.

use std::ops::Range;

use anyhow::{Context, bail, ensure};
use rustc_hash::{FxHashMap, FxHashSet};
use wasmparser::{
    BinaryReader, CodeSectionReader, DataKind, DataSectionReader, ElementItems, ElementKind,
    ElementSectionReader, FuncType, FunctionSectionReader, ImportSectionReader, Linking,
    LinkingSectionReader, MemorySectionReader, RelocSectionReader, RelocationType, SymbolFlags,
    SymbolInfo, TableSectionReader, TypeRef, TypeSectionReader, ValType,
};

use wasm_encoder::Encode;

use super::Spliced;

const PAGE: u32 = 1 << 16;
/// What the linker leaves room for where an index or an address goes.
const PADDED: usize = 5;
const FUNCREF: u8 = 0x70;

/// The ids of the sections that are read or written again.
mod id {
    pub(super) const CUSTOM: u8 = 0;
    pub(super) const TYPE: u8 = 1;
    pub(super) const IMPORT: u8 = 2;
    pub(super) const FUNCTION: u8 = 3;
    pub(super) const TABLE: u8 = 4;
    pub(super) const MEMORY: u8 = 5;
    pub(super) const ELEMENT: u8 = 9;
    pub(super) const CODE: u8 = 10;
    pub(super) const DATA: u8 = 11;
    pub(super) const DATA_COUNT: u8 = 12;
}

struct Section {
    id: u8,
    /// Of the whole section, and of what follows its id and length.
    whole: Range<usize>,
    contents: Range<usize>,
}

/// What a wasm file is made of, as far as both a linked module and an object have it.
struct Parts<'a> {
    sections: Vec<Section>,
    types: Vec<FuncType>,
    /// The names of the functions and the globals that are imported, in the order of their
    /// indices, and the types of the functions.
    imported_functions: Vec<&'a str>,
    imported_types: Vec<u32>,
    imported_globals: Vec<&'a str>,
    /// The type of each function that has a body.
    functions: Vec<u32>,
    /// Where each body is, without its length.
    bodies: Vec<Range<usize>>,
    data: Vec<Data>,
    symbols: Vec<SymbolInfo<'a>>,
    /// The functions the `name` section names, for a debugger.
    names: Vec<wasmparser::Naming<'a>>,
    /// The alignment of each data segment of an object.
    alignments: Vec<u32>,
    code_relocations: Vec<wasmparser::RelocationEntry>,
    data_relocations: Vec<wasmparser::RelocationEntry>,
}

struct Data {
    /// `None` for a segment the program copies itself.
    address: Option<u32>,
    bytes: Range<usize>,
}

impl<'a> Parts<'a> {
    fn read(bytes: &'a [u8]) -> anyhow::Result<Parts<'a>> {
        ensure!(bytes.starts_with(b"\0asm\x01\0\0\0"), "not a wasm module");
        let mut parts = Parts {
            sections: Vec::new(),
            types: Vec::new(),
            imported_functions: Vec::new(),
            imported_types: Vec::new(),
            names: Vec::new(),
            imported_globals: Vec::new(),
            functions: Vec::new(),
            bodies: Vec::new(),
            data: Vec::new(),
            symbols: Vec::new(),
            alignments: Vec::new(),
            code_relocations: Vec::new(),
            data_relocations: Vec::new(),
        };
        let mut reader = BinaryReader::new(bytes, 0);
        reader.read_bytes(8)?;
        while !reader.eof() {
            let start = reader.original_position();
            let id = reader.read_u8()?;
            let len = reader.read_var_u32()? as usize;
            let contents = reader.original_position()..reader.original_position() + len;
            reader.read_bytes(len)?;
            parts.sections.push(Section { id, whole: start..contents.end, contents });
        }
        for at in 0..parts.sections.len() {
            let Section { id, ref contents, .. } = parts.sections[at];
            let reader = BinaryReader::new(&bytes[contents.clone()], contents.start);
            match id {
                id::TYPE => {
                    for ty in TypeSectionReader::new(reader)?.into_iter_err_on_gc_types() {
                        parts.types.push(ty?);
                    }
                }
                id::IMPORT => {
                    for import in ImportSectionReader::new(reader)? {
                        let import = import?;
                        match import.ty {
                            TypeRef::Func(ty) => {
                                parts.imported_functions.push(import.name);
                                parts.imported_types.push(ty);
                            }
                            TypeRef::Global(_) => parts.imported_globals.push(import.name),
                            _ => {}
                        }
                    }
                }
                id::FUNCTION => {
                    parts.functions =
                        FunctionSectionReader::new(reader)?.into_iter().collect::<Result<_, _>>()?
                }
                id::CODE => {
                    for body in CodeSectionReader::new(reader)? {
                        parts.bodies.push(body?.range());
                    }
                }
                id::DATA => {
                    for data in DataSectionReader::new(reader)? {
                        let data = data?;
                        let address = match data.kind {
                            DataKind::Active { memory_index: 0, offset_expr } => {
                                Some(constant(&offset_expr)?)
                            }
                            _ => None,
                        };
                        let end = data.range.end;
                        parts.data.push(Data { address, bytes: end - data.data.len()..end });
                    }
                }
                id::CUSTOM => parts.custom(reader)?,
                _ => {}
            }
        }
        Ok(parts)
    }

    fn custom(&mut self, mut reader: BinaryReader<'a>) -> anyhow::Result<()> {
        let name = reader.read_string()?;
        let at = reader.original_position();
        let reader = BinaryReader::new(reader.read_bytes(reader.bytes_remaining())?, at);
        match name {
            "linking" => {
                for subsection in LinkingSectionReader::new(reader)?.subsections() {
                    match subsection? {
                        Linking::SymbolTable(symbols) => {
                            self.symbols = symbols.into_iter().collect::<Result<_, _>>()?
                        }
                        Linking::SegmentInfo(segments) => {
                            for segment in segments {
                                self.alignments.push(segment?.alignment);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "name" => {
                for names in wasmparser::NameSectionReader::new(reader) {
                    if let wasmparser::Name::Function(names) = names? {
                        self.names = names.into_iter().collect::<Result<_, _>>()?;
                    }
                }
            }
            "reloc.CODE" | "reloc.DATA" => {
                let entries = RelocSectionReader::new(reader)?
                    .entries()
                    .into_iter()
                    .collect::<Result<_, _>>()?;
                match name {
                    "reloc.CODE" => self.code_relocations = entries,
                    _ => self.data_relocations = entries,
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn section(&self, id: u8) -> Option<&Section> {
        self.sections.iter().find(|it| it.id == id)
    }

    /// The name of a function or a global a symbol stands for: an import has its own.
    fn name(name: Option<&'a str>, index: u32, imported: &[&'a str]) -> anyhow::Result<&'a str> {
        name.or_else(|| imported.get(index as usize).copied()).context("a symbol without a name")
    }
}

fn constant(expression: &wasmparser::ConstExpr<'_>) -> anyhow::Result<u32> {
    let mut reader = expression.get_binary_reader();
    ensure!(reader.read_u8()? == 0x41, "an address that is not a constant");
    Ok(reader.read_var_i32()? as u32)
}

/// What a name stands for in the module.
#[derive(Clone, Copy)]
enum Named {
    Function(u32),
    Data {
        address: u32,
        size: u32,
    },
    Global(u32),
    /// Symbols private to their objects that share the name: which one is meant is lost.
    Many,
}

/// The one table of functions and the segment that fills it.
struct Table {
    /// The slot of the first function of the segment.
    first: u32,
    functions: Vec<u32>,
    max: Option<u32>,
}

/// The module as the linker wrote it.
pub(super) struct Module {
    bytes: Vec<u8>,
    sections: Vec<Section>,
    types: Vec<FuncType>,
    /// The type of each function: of those that are imported, and of those with a body.
    imported: Vec<u32>,
    functions: Vec<u32>,
    bodies: Vec<Range<usize>>,
    symbols: FxHashMap<Box<str>, Named>,
    /// The functions the symbol table leaves out and a debugger is told the names of, in
    /// the order the linker read them. Of definitions that differ in their signature the
    /// linker keeps each, lists none, and has every caller reach the one of its own
    /// signature; of those that do not differ, the first is the one it kept.
    unlisted: FxHashMap<Box<str>, Vec<u32>>,
    /// `None` when the module has none of that shape, and nothing is added to it then.
    table: Option<Table>,
    slots: FxHashMap<u32, u32>,
    /// The pages the memory starts with, and the most it may have.
    pages: Option<(u32, Option<u32>)>,
    segments: u32,
}

impl Module {
    pub(super) fn read(bytes: Vec<u8>) -> anyhow::Result<Module> {
        let parts = Parts::read(&bytes)?;
        ensure!(!parts.symbols.is_empty(), "a module without a symbol table (--emit-relocs)");
        let mut symbols = FxHashMap::<Box<str>, Named>::default();
        for symbol in &parts.symbols {
            let (name, named) = match *symbol {
                SymbolInfo::Func { index, name, .. } => {
                    (Parts::name(name, index, &parts.imported_functions)?, Named::Function(index))
                }
                SymbolInfo::Global { index, name, .. } => {
                    (Parts::name(name, index, &parts.imported_globals)?, Named::Global(index))
                }
                SymbolInfo::Data { flags, name, symbol: Some(data) } => {
                    let address = if flags.contains(SymbolFlags::ABSOLUTE) {
                        data.offset
                    } else {
                        let segment = parts.data.get(data.index as usize);
                        let start = segment.and_then(|it| it.address);
                        start.context("data in a segment without an address")? + data.offset
                    };
                    (name, Named::Data { address, size: data.size })
                }
                _ => continue,
            };
            symbols.entry(name.into()).and_modify(|it| *it = Named::Many).or_insert(named);
        }
        let listed: FxHashSet<u32> = parts
            .symbols
            .iter()
            .filter_map(|it| match *it {
                SymbolInfo::Func { index, .. } => Some(index),
                _ => None,
            })
            .collect();
        let mut unlisted = FxHashMap::<Box<str>, Vec<u32>>::default();
        for name in parts.names.iter().filter(|it| !listed.contains(&it.index)) {
            unlisted.entry(name.name.into()).or_default().push(name.index);
        }
        unlisted.values_mut().for_each(|it| it.sort_unstable());
        let table = Module::table(&parts, &bytes)?;
        let slots = table
            .iter()
            .flat_map(|table| (table.first..).zip(&table.functions))
            .map(|(slot, &function)| (function, slot))
            .collect();
        let pages = match parts.section(id::MEMORY) {
            Some(section) => {
                let reader = BinaryReader::new(&bytes[section.contents.clone()], 0);
                let mut memories = MemorySectionReader::new(reader)?.into_iter();
                match (memories.next().transpose()?, memories.next()) {
                    (Some(memory), None) if !memory.memory64 && !memory.shared => {
                        Some((memory.initial as u32, memory.maximum.map(|it| it as u32)))
                    }
                    _ => None,
                }
            }
            None => None,
        };
        let Parts { sections, types, imported_types, functions, bodies, data, .. } = parts;
        Ok(Module {
            imported: imported_types,
            segments: data.len() as u32,
            sections,
            types,
            functions,
            bodies,
            symbols,
            unlisted,
            table,
            slots,
            pages,
            bytes,
        })
    }

    /// The function with a name and a signature, when the module has one.
    fn function(&self, name: &str, ty: &FuncType) -> anyhow::Result<Option<u32>> {
        let is = |&function: &u32| {
            self.ty(function).and_then(|it| self.types.get(it as usize)) == Some(ty)
        };
        match self.symbols.get(name) {
            Some(Named::Function(function)) => {
                ensure!(is(function), "{name} has another signature");
                Ok(Some(*function))
            }
            Some(_) => bail!("{name} is not one function of the executable"),
            None => Ok(self.unlisted.get(name).and_then(|it| it.iter().copied().find(|it| is(it)))),
        }
    }

    /// The type of the function with an index.
    fn ty(&self, function: u32) -> Option<u32> {
        self.imported.iter().chain(&self.functions).nth(function as usize).copied()
    }

    fn table(parts: &Parts<'_>, bytes: &[u8]) -> anyhow::Result<Option<Table>> {
        let (Some(tables), Some(elements)) = (parts.section(id::TABLE), parts.section(id::ELEMENT))
        else {
            return Ok(None);
        };
        let contents = &bytes[tables.contents.clone()];
        let mut tables = TableSectionReader::new(BinaryReader::new(contents, 0))?.into_iter();
        let (Some(table), None) = (tables.next().transpose()?, tables.next()) else {
            return Ok(None);
        };
        let elements = BinaryReader::new(&bytes[elements.contents.clone()], 0);
        let mut elements = ElementSectionReader::new(elements)?.into_iter();
        let (Some(element), None) = (elements.next().transpose()?, elements.next()) else {
            return Ok(None);
        };
        let ElementKind::Active { table_index: None | Some(0), offset_expr } = element.kind else {
            return Ok(None);
        };
        let ElementItems::Functions(functions) = element.items else { return Ok(None) };
        let functions: Vec<u32> = functions.into_iter().collect::<Result<_, _>>()?;
        let first = constant(&offset_expr)?;
        let plain = table.ty.initial == u64::from(first) + functions.len() as u64
            && !table.ty.table64
            && !table.ty.shared
            && matches!(table.init, wasmparser::TableInit::RefNull)
            && contents.get(1) == Some(&FUNCREF);
        Ok(plain.then(|| Table { first, functions, max: table.ty.maximum.map(|it| it as u32) }))
    }

    /// The module with `patch` in place of what it defines, and what that took.
    pub(super) fn splice(&self, patch: &[u8]) -> anyhow::Result<(Vec<u8>, Spliced)> {
        let parts = Parts::read(patch)?;
        let mut placed = Placed::new(self, &parts, patch)?;
        for (relocations, section) in
            [(&parts.code_relocations, id::CODE), (&parts.data_relocations, id::DATA)]
        {
            for relocation in relocations {
                placed.relocate(relocation, section)?;
            }
        }
        Ok(placed.write())
    }
}

/// The names the patch defines for the rest of the program.
pub(super) fn defined(patch: &[u8]) -> anyhow::Result<Vec<Box<[u8]>>> {
    let parts = Parts::read(patch)?;
    let names = parts.symbols.iter().filter_map(|symbol| match *symbol {
        SymbolInfo::Func { flags, name: Some(name), .. }
        | SymbolInfo::Data { flags, name, symbol: Some(_) }
            if !flags.intersects(SymbolFlags::UNDEFINED | SymbolFlags::BINDING_LOCAL) =>
        {
            Some(name.as_bytes().into())
        }
        _ => None,
    });
    Ok(names.collect())
}

/// What a symbol of the patch is in the module that is written.
#[derive(Clone, Copy)]
enum Target {
    Function(u32),
    Address(u32),
    Global(u32),
}

/// The patch laid out in the module: every function and segment of it has its place, and
/// its bytes are a copy the relocations are applied to.
struct Placed<'a> {
    module: &'a Module,
    parts: &'a Parts<'a>,
    patch: Vec<u8>,
    types: Vec<FuncType>,
    /// The index each function the patch defines has in the module.
    function_indices: Vec<u32>,
    /// The functions behind the module's, as indices into the patch's.
    added: Vec<usize>,
    /// Those that take the place of one of the module's, by the index of that.
    replaced: FxHashMap<u32, usize>,
    /// The address of each segment of the patch, and where its bytes are in the patch.
    data_addresses: Vec<u32>,
    data_ranges: Vec<Range<usize>>,
    /// The pages the patch's new data takes.
    pages: u32,
    /// The functions that get a slot behind the table's last.
    slotted: Vec<u32>,
    slots: FxHashMap<u32, u32>,
}

impl<'a> Placed<'a> {
    fn new(module: &'a Module, parts: &'a Parts<'a>, patch: &[u8]) -> anyhow::Result<Self> {
        let mut placed = Placed {
            module,
            parts,
            patch: patch.to_vec(),
            types: module.types.clone(),
            function_indices: vec![u32::MAX; parts.bodies.len()],
            added: Vec::new(),
            replaced: FxHashMap::default(),
            data_addresses: vec![u32::MAX; parts.data.len()],
            data_ranges: parts.data.iter().map(|it| it.bytes.clone()).collect(),
            pages: 0,
            slotted: Vec::new(),
            slots: FxHashMap::default(),
        };
        let imported = parts.imported_functions.len() as u32;
        let total = (module.imported.len() + module.functions.len()) as u32;
        for symbol in &parts.symbols {
            match *symbol {
                SymbolInfo::Func { flags, index, name: Some(name) }
                    if !flags.contains(SymbolFlags::UNDEFINED) =>
                {
                    let body = (index - imported) as usize;
                    let ty =
                        placed.ty(*parts.functions.get(body).context("no such function")?)?;
                    let old = match flags.contains(SymbolFlags::BINDING_LOCAL) {
                        true => None,
                        false => module.function(name, &placed.types[ty as usize])?,
                    };
                    let Some(old) = old else {
                        ensure!(
                            !flags.intersects(SymbolFlags::EXPORTED | SymbolFlags::NO_STRIP),
                            "{name} is new and exported"
                        );
                        continue;
                    };
                    ensure!(
                        old as usize >= module.imported.len(),
                        "the patch replaces {name}, which is imported"
                    );
                    placed.function_indices[body] = old;
                    placed.replaced.insert(old, body);
                }
                SymbolInfo::Data { flags, name, symbol: Some(data) } => {
                    let segment = data.index as usize;
                    let len = parts.data.get(segment).context("no such segment")?.bytes.len();
                    if flags.contains(SymbolFlags::BINDING_LOCAL) {
                        continue;
                    }
                    let (address, size) = match module.symbols.get(name) {
                        None => continue,
                        Some(&Named::Data { address, size }) => (address, size),
                        Some(_) => bail!("the patch replaces {name}, which is not one static"),
                    };
                    let align = 1u32 << parts.alignments.get(segment).copied().unwrap_or(0);
                    ensure!(
                        data.offset == 0 && data.size as usize == len,
                        "the static {name} shares its segment"
                    );
                    ensure!(
                        data.size <= size && address.is_multiple_of(align),
                        "the static {name} has another value, which is larger or aligned to \
                         more: {} bytes after {size}",
                        data.size
                    );
                    placed.data_addresses[segment] = address;
                }
                _ => {}
            }
        }
        for (body, index) in placed.function_indices.iter_mut().enumerate() {
            if *index == u32::MAX {
                *index = total + placed.added.len() as u32;
                placed.added.push(body);
                // Its type is one of the module's from here on.
                let ty = parts.functions[body];
                let ty = parts.types.get(ty as usize).context("no such type")?;
                if !placed.types.contains(ty) {
                    placed.types.push(ty.clone());
                }
            }
        }
        let new: Vec<usize> =
            (0..parts.data.len()).filter(|&it| placed.data_addresses[it] == u32::MAX).collect();
        if !new.is_empty() {
            let (pages, _) = module.pages.context("a memory that is not the module's own")?;
            let start = pages.checked_mul(PAGE).context("a memory that is full")?;
            let mut end = start;
            for segment in new {
                let align = 1u32 << parts.alignments.get(segment).copied().unwrap_or(0);
                end = end.next_multiple_of(align);
                placed.data_addresses[segment] = end;
                end = end
                    .checked_add(parts.data[segment].bytes.len() as u32)
                    .context("a memory that is full")?;
            }
            placed.pages = (end - start).div_ceil(PAGE);
        }
        Ok(placed)
    }

    /// The index in the module of a type of the patch.
    fn ty(&mut self, ty: u32) -> anyhow::Result<u32> {
        let ty = self.parts.types.get(ty as usize).context("no such type")?;
        let index = match self.types.iter().position(|it| it == ty) {
            Some(index) => index,
            None => {
                self.types.push(ty.clone());
                self.types.len() - 1
            }
        };
        Ok(index as u32)
    }

    fn target(&self, symbol: u32) -> anyhow::Result<Target> {
        let (module, parts) = (self.module, self.parts);
        let missing = |name: &str| anyhow::anyhow!("{name} is not in the executable");
        Ok(match *parts.symbols.get(symbol as usize).context("no such symbol")? {
            SymbolInfo::Func { flags, index, name } if flags.contains(SymbolFlags::UNDEFINED) => {
                let name = Parts::name(name, index, &parts.imported_functions)?;
                let ty = parts.imported_types.get(index as usize).context("no such import")?;
                let ty = parts.types.get(*ty as usize).context("no such type")?;
                Target::Function(module.function(name, ty)?.ok_or_else(|| missing(name))?)
            }
            SymbolInfo::Func { index, .. } => {
                let body = index as usize - parts.imported_functions.len();
                Target::Function(self.function_indices[body])
            }
            SymbolInfo::Data { symbol: Some(data), .. } => {
                Target::Address(self.data_addresses[data.index as usize] + data.offset)
            }
            SymbolInfo::Data { name, symbol: None, .. } => match module.symbols.get(name) {
                Some(&Named::Data { address, .. }) => Target::Address(address),
                Some(Named::Many) => bail!("{name} names several things in the executable"),
                _ => return Err(missing(name)),
            },
            SymbolInfo::Global { index, name, .. } => {
                let name = Parts::name(name, index, &parts.imported_globals)?;
                match module.symbols.get(name) {
                    Some(&Named::Global(index)) => Target::Global(index),
                    _ => return Err(missing(name)),
                }
            }
            _ => bail!("a relocation against a symbol that is no function, data or global"),
        })
    }

    /// The slot of the table that holds a function, which is a new one when it has none.
    fn slot(&mut self, function: u32) -> anyhow::Result<u32> {
        if let Some(&slot) = self.module.slots.get(&function).or_else(|| self.slots.get(&function))
        {
            return Ok(slot);
        }
        let table = self.module.table.as_ref().context("no table of functions to add to")?;
        let slot = table.first + (table.functions.len() + self.slotted.len()) as u32;
        self.slotted.push(function);
        self.slots.insert(function, slot);
        Ok(slot)
    }

    /// Writes what `relocation` asks for into the copy of the patch, whose offset is from
    /// the start of the section with the id `section`.
    fn relocate(
        &mut self,
        relocation: &wasmparser::RelocationEntry,
        section: u8,
    ) -> anyhow::Result<()> {
        use RelocationType::*;
        let section = self.parts.section(section).context("a relocation of no section")?;
        let at = section.contents.start + relocation.offset as usize;
        let value = match relocation.ty {
            TypeIndexLeb => i64::from(self.ty(relocation.index)?),
            _ => match (relocation.ty, self.target(relocation.index)?) {
                (FunctionIndexLeb, Target::Function(index)) => i64::from(index),
                (TableIndexSleb | TableIndexI32, Target::Function(index)) => {
                    i64::from(self.slot(index)?)
                }
                (MemoryAddrLeb | MemoryAddrSleb | MemoryAddrI32, Target::Address(address)) => {
                    i64::from(address) + relocation.addend
                }
                (GlobalIndexLeb, Target::Global(index)) => i64::from(index),
                (ty, _) => bail!("a relocation of type {ty:?}, which only the linker handles"),
            },
        };
        let place = self.patch.get_mut(at..).context("a relocation outside of the patch")?;
        match relocation.ty {
            TableIndexI32 | MemoryAddrI32 => {
                let place = place.first_chunk_mut().context("a relocation at the end")?;
                *place = (value as u32).to_le_bytes();
            }
            TableIndexSleb | MemoryAddrSleb => {
                let place = place.first_chunk_mut().context("a relocation at the end")?;
                *place = padded(value as i32 as i64, true);
            }
            _ => {
                let place = place.first_chunk_mut().context("a relocation at the end")?;
                *place = padded(value, false);
            }
        }
        Ok(())
    }

    fn write(self) -> (Vec<u8>, Spliced) {
        let Placed { module, parts, patch, .. } = &self;
        let mut out = Vec::with_capacity(module.bytes.len() + patch.len());
        out.extend(&module.bytes[..8]);
        let body = |out: &mut Vec<u8>, index: usize| {
            let bytes = &patch[parts.bodies[index].clone()];
            (bytes.len() as u32).encode(out);
            out.extend(bytes);
        };
        // Those that stay where they were are written there once more, after the linker's.
        let segments = parts.data.len();
        for section in &module.sections {
            let old = &module.bytes[section.contents.clone()];
            let mut contents = Vec::new();
            match section.id {
                id::TYPE if self.types.len() > module.types.len() => {
                    (self.types.len() as u32).encode(&mut contents);
                    for ty in &self.types {
                        contents.push(0x60);
                        for values in [ty.params(), ty.results()] {
                            (values.len() as u32).encode(&mut contents);
                            contents.extend(values.iter().map(|&it| value_type(it)));
                        }
                    }
                }
                id::FUNCTION if !self.added.is_empty() => {
                    ((module.functions.len() + self.added.len()) as u32).encode(&mut contents);
                    for &ty in &module.functions {
                        ty.encode(&mut contents);
                    }
                    for &added in &self.added {
                        let ty = &parts.types[parts.functions[added] as usize];
                        let index = self.types.iter().position(|it| it == ty);
                        (index.expect("a type that was added") as u32).encode(&mut contents);
                    }
                }
                id::TABLE if !self.slotted.is_empty() => {
                    let table = module.table.as_ref().expect("a slot of no table");
                    let len = table.first + (table.functions.len() + self.slotted.len()) as u32;
                    contents.push(1);
                    contents.push(FUNCREF);
                    contents.push(u8::from(table.max.is_some()));
                    len.encode(&mut contents);
                    if let Some(max) = table.max {
                        max.max(len).encode(&mut contents);
                    }
                }
                id::MEMORY if self.pages > 0 => {
                    let (pages, max) = module.pages.expect("data in no memory");
                    let pages = pages + self.pages;
                    contents.push(1);
                    contents.push(u8::from(max.is_some()));
                    pages.encode(&mut contents);
                    if let Some(max) = max {
                        max.max(pages).encode(&mut contents);
                    }
                }
                id::ELEMENT if !self.slotted.is_empty() => {
                    let table = module.table.as_ref().expect("a slot of no table");
                    contents.extend([1, 0, 0x41]);
                    (table.first as i32).encode(&mut contents);
                    contents.push(0x0b);
                    ((table.functions.len() + self.slotted.len()) as u32).encode(&mut contents);
                    for &function in table.functions.iter().chain(&self.slotted) {
                        function.encode(&mut contents);
                    }
                }
                id::DATA_COUNT if segments > 0 => {
                    (module.segments + segments as u32).encode(&mut contents);
                }
                id::CODE => {
                    ((module.bodies.len() + self.added.len()) as u32).encode(&mut contents);
                    for (index, range) in (module.imported.len() as u32..).zip(&module.bodies) {
                        match self.replaced.get(&index) {
                            Some(&new) => body(&mut contents, new),
                            None => {
                                (range.len() as u32).encode(&mut contents);
                                contents.extend(&module.bytes[range.clone()]);
                            }
                        }
                    }
                    for &added in &self.added {
                        body(&mut contents, added);
                    }
                }
                id::DATA if segments > 0 => {
                    let mut reader = BinaryReader::new(old, 0);
                    reader.read_var_u32().expect("a data section that was read");
                    (module.segments + segments as u32).encode(&mut contents);
                    let rest = reader.read_bytes(reader.bytes_remaining());
                    contents.extend(rest.expect("the rest of a section"));
                    for segment in 0..segments {
                        let bytes = &patch[self.data_ranges[segment].clone()];
                        contents.extend([0, 0x41]);
                        (self.data_addresses[segment] as i32).encode(&mut contents);
                        contents.push(0x0b);
                        (bytes.len() as u32).encode(&mut contents);
                        contents.extend(bytes);
                    }
                }
                id::CUSTOM if stale(old) => continue,
                _ => {
                    out.extend(&module.bytes[section.whole.clone()]);
                    continue;
                }
            }
            out.push(section.id);
            (contents.len() as u32).encode(&mut out);
            out.extend(contents);
        }
        let appended = self.added.iter().map(|&it| parts.bodies[it].len()).sum::<usize>()
            + parts.data.iter().map(|it| it.bytes.len()).sum::<usize>();
        (out, Spliced { redirected: self.replaced.len(), appended, copied: 0 })
    }
}

/// Whether a custom section is one the splice leaves behind.
fn stale(contents: &[u8]) -> bool {
    let name = BinaryReader::new(contents, 0).read_string().unwrap_or_default();
    name == "linking" || name.starts_with("reloc.")
}

fn value_type(ty: ValType) -> u8 {
    match ty {
        ValType::I32 => 0x7f,
        ValType::I64 => 0x7e,
        ValType::F32 => 0x7d,
        ValType::F64 => 0x7c,
        ValType::V128 => 0x7b,
        ValType::Ref(ty) if ty.is_extern_ref() => 0x6f,
        ValType::Ref(_) => 0x70,
    }
}

/// A number in the five bytes the linker leaves for it.
fn padded(mut value: i64, signed: bool) -> [u8; PADDED] {
    let mut bytes = [0x80; PADDED];
    for byte in &mut bytes {
        *byte |= (value & 0x7f) as u8;
        value >>= 7;
    }
    bytes[PADDED - 1] &= if signed { 0x7f } else { 0x0f };
    bytes
}
