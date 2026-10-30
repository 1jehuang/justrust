//! Unwind tables of Windows (`.pdata` and `.xdata`) and what a landing pad needs there.
//!
//! Windows unwinds in two passes and expects the code that runs on the way to be functions
//! of their own, which the unwinder calls and which return to it. Our landing pads are
//! blocks of the function instead, entered once and never returned from, and Cranelift has
//! no call that names its pad for the calling convention of Windows. So a call with a pad
//! is a plain call followed by a look at a slot of the frame, and the function names a
//! personality that is ours: when an exception is looked for a handler and the call it
//! passes is such a call, the personality claims it, has the frames below unwound, and has
//! the call return with the thrown object, moved to the heap, in the slot. A pad that only
//! cleans up throws the object again when it is done.
//!
//! LLVM has calls that name their pad, and its code keeps nothing where Cranelift's has
//! the slot. A call of its code goes on at the pad instead of where it returns to, with
//! the object in the register a function returns in ([`Pads`]).
//!
//! Only C++ exceptions are claimed, which is what a Rust panic is for MSVC. The object is
//! moved as bytes, which is all a panic needs.

use cranelift_codegen::{
    Context,
    ir::{
        self, AbiParam, ExtFuncData, ExternalName, InstBuilder, MemFlagsData, Signature,
        StackSlotData, StackSlotKind, TrapCode, UserExternalName, Value,
        condcodes::IntCC,
        types::{I8, I32, I64},
    },
    isa::unwind::winx64,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_object::{
    ObjectProduct,
    object::{
        RelocationEncoding, RelocationFlags, RelocationKind, SectionKind,
        write::{Relocation, SectionId, SymbolId},
    },
};

use super::FunctionUnwind;

/// Throws again what a landing pad was entered with. Takes the pad's exception.
pub(in super::super) const RESUME: &str = "ra-jit.seh.resume";
/// Frees what a landing pad that stops the unwind was entered with.
pub(in super::super) const RELEASE: &str = "ra-jit.seh.release";
pub(in super::super) const PERSONALITY: &str = "ra-jit.seh.personality";

/// The bit of a source location that marks a call that has a landing pad, for [`function`]
/// to find them. The rest is the statement the call is of, as without it.
const GUARDED: u32 = 1 << 31;

/// The source location of a call of the statement at `loc` that has a landing pad.
pub(in super::super) fn guarded(loc: ir::SourceLoc) -> ir::SourceLoc {
    ir::SourceLoc::new(unguarded(loc).bits() | GUARDED)
}

pub(in super::super) fn is_guarded(loc: ir::SourceLoc) -> bool {
    !loc.is_default() && loc.bits() & GUARDED != 0
}

/// The source location without the marks, which says the statement alone.
pub(in super::super) fn unguarded(loc: ir::SourceLoc) -> ir::SourceLoc {
    if loc.is_default() {
        ir::SourceLoc::new(0)
    } else {
        ir::SourceLoc::new(loc.bits() & !(GUARDED | IN_BOUNDS))
    }
}

/// The mark of an `iadd` that offsets a pointer within what it points into, as the `offset`
/// intrinsic does: LLVM builds it as `getelementptr inbounds`.
const IN_BOUNDS: u32 = 1 << 30;

pub(in super::super) fn in_bounds(loc: ir::SourceLoc) -> ir::SourceLoc {
    let bits = if loc.is_default() { 0 } else { loc.bits() };
    ir::SourceLoc::new(bits | IN_BOUNDS)
}

pub(in super::super) fn is_in_bounds(loc: ir::SourceLoc) -> bool {
    !loc.is_default() && loc.bits() & IN_BOUNDS != 0
}
/// Where a call that unwound finds its exception, from the frame pointer: the first of
/// the four slots a caller leaves above the return address for the function to use as it
/// likes, which Cranelift's code does not use otherwise.
pub(in super::super) const ARRIVAL: i32 = 16;
/// Where the thrown object is in the exception a landing pad is entered with. Before it
/// are what throwing it again takes: its `ThrowInfo`, the image that is relative to, and
/// the size of the object.
pub(in super::super) const OBJECT: i64 = 24;

/// The exception code of the C++ runtime of MSVC, and the first of its four parameters.
/// The other three are the object, its `ThrowInfo`, and the image the latter is in.
const CXX_EXCEPTION: u32 = 0xE06D_7363;
const CXX_MAGIC: i64 = 0x1993_0520;
const EXCEPTION_NONCONTINUABLE: i64 = 1;
/// The flags of an exception record that say the frames are being unwound, not searched.
const EXCEPTION_UNWIND: i64 = 0x66;
const CONTINUE_SEARCH: i64 = 1;
const UNW_FLAG_EHANDLER: u8 = 1;
/// The largest object [`RESUME`] throws from its own frame. A panic's is two pointers.
const INLINE_OBJECT: u32 = 256;
const MEM: MemFlagsData = MemFlagsData::new().with_notrap();

/// A function's `UNWIND_INFO` without a handler, and how long its code is.
#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Serialize, serde_derive::Deserialize)]
pub(super) struct Frame {
    pub(super) len: u32,
    pub(super) info: Vec<u8>,
}

/// The calls of a function that have a landing pad, as the personality reads them after
/// their number: three `u32` each, all from the start of the function. A call that returns
/// to an address after the first and no later than the second has a pad, which is at the
/// third; or, where that is zero, the call returns with the exception in [`ARRIVAL`].
#[derive(Default)]
pub(in super::super) struct Pads(Vec<u8>);

impl Pads {
    const ENTRY: usize = 12;

    /// A call that returns to `returns_to` and looks at [`ARRIVAL`] when it has.
    fn returning(&mut self, returns_to: u32) {
        self.entered(returns_to - 1..returns_to, 0);
    }

    /// Calls that return to an address after `calls.start` and no later than `calls.end`,
    /// and go on at `pad` when they unwind.
    pub(in super::super) fn entered(&mut self, calls: std::ops::Range<u32>, pad: u32) {
        for word in [calls.start, calls.end, pad] {
            self.0.extend_from_slice(&word.to_le_bytes());
        }
    }

    pub(in super::super) fn finish(self) -> Option<Vec<u8>> {
        (!self.0.is_empty()).then_some(self.0)
    }
}

/// The frame of compiled code and the calls in it that have a landing pad.
pub(super) fn function(
    code: &cranelift_codegen::CompiledCode,
    info: &winx64::UnwindInfo,
) -> Result<(Frame, Option<Vec<u8>>), String> {
    let mut bytes = vec![0; info.emit_size()];
    info.emit(&mut bytes);
    let bytes = with_saves(&bytes)?;
    let guarded: Vec<_> = code
        .buffer
        .get_srclocs_sorted()
        .iter()
        .filter(|it| is_guarded(it.loc))
        .map(|it| it.start..it.end)
        .collect();
    let mut calls = Pads::default();
    for call_site in code.buffer.call_sites() {
        // The call ends where it returns to, which may be where the next thing starts.
        if guarded.iter().any(|it| it.contains(&(call_site.ret_addr - 1))) {
            calls.returning(call_site.ret_addr);
        }
    }
    let frame = Frame { len: code.buffer.data().len() as u32, info: bytes };
    Ok((frame, calls.finish()))
}

const UWOP_PUSH_NONVOL: u8 = 0;
const UWOP_ALLOC_LARGE: u8 = 1;
const UWOP_ALLOC_SMALL: u8 = 2;
const UWOP_SET_FPREG: u8 = 3;
const UWOP_SAVE_NONVOL: u8 = 4;
const UWOP_SAVE_XMM128: u8 = 8;

/// Cranelift's `UNWIND_INFO` with where the registers the function saves are.
///
/// Cranelift's code saves them below the frame pointer, in the order it names them: those
/// of eight bytes and then those of sixteen. Its `UNWIND_INFO` means to say so by having
/// the frame start that far below the frame pointer, but what it writes loses the
/// distance (it shifts it out of the byte) and halves where each register of eight bytes
/// is. That finds the frames, which is what Wasmtime needs of it, and gives the registers
/// values they never had, which is not enough to run a frame again. So this writes the
/// distance, the places, and an allocation of the distance that the code does not have
/// by itself, for the unwinder to find the frame pointer's own slot from where the frame
/// starts.
fn with_saves(info: &[u8]) -> Result<Vec<u8>, String> {
    let unknown = |op: u8| format!("an unwind code {op} of Cranelift's");
    let nodes = &info[4..4 + usize::from(info[2]) * 2];
    // (offset in the prologue, operation, register, how many words follow)
    let mut codes = Vec::new();
    let mut at = 0;
    while at < nodes.len() {
        let (offset, op, register) = (nodes[at], nodes[at + 1] & 0xf, nodes[at + 1] >> 4);
        let following = match op {
            UWOP_PUSH_NONVOL | UWOP_SET_FPREG => 0,
            UWOP_SAVE_NONVOL | UWOP_SAVE_XMM128 => 1,
            _ => return Err(unknown(op)),
        };
        let word = u16::from_le_bytes([nodes[at + 2 * following], nodes[at + 2 * following + 1]]);
        codes.push((offset, op, register, word));
        at += 2 + 2 * following;
    }
    // The codes are last first.
    let words = codes.iter().filter(|code| code.1 == UWOP_SAVE_NONVOL).count();
    let vectors = codes.iter().filter(|code| code.1 == UWOP_SAVE_XMM128).map(|code| code.3 + 1);
    let saved = vectors.max().map_or(words.next_multiple_of(2) * 8, |end| usize::from(end) * 16);
    if saved == 0 {
        return Ok(info.to_vec());
    }
    if saved > 240 {
        return Err(format!("{saved} bytes of saved registers"));
    }
    let mut out = Vec::new();
    let mut word = words;
    for (offset, op, register, following) in codes {
        out.extend([offset, register << 4 | op]);
        match op {
            UWOP_SAVE_NONVOL => {
                word -= 1;
                out.extend((word as u16).to_le_bytes());
            }
            UWOP_SAVE_XMM128 => out.extend(following.to_le_bytes()),
            UWOP_SET_FPREG if saved <= 128 => {
                out.extend([offset, (((saved - 8) / 8) as u8 * 16) | UWOP_ALLOC_SMALL]);
            }
            UWOP_SET_FPREG => {
                out.extend([offset, UWOP_ALLOC_LARGE]);
                out.extend((saved as u16 / 8).to_le_bytes());
            }
            _ => (),
        }
    }
    let count = out.len() / 2;
    out.resize(out.len().next_multiple_of(4), 0);
    let header = [info[0], info[1], count as u8, ((saved / 16) as u8 * 16) | (info[3] & 0xf)];
    Ok(header.into_iter().chain(out).collect())
}

struct Entry {
    func: FuncId,
    frame: Frame,
    pads: Option<Vec<u8>>,
}

#[derive(Default)]
pub(super) struct Tables {
    entries: Vec<Entry>,
    personality: Option<FuncId>,
    resume: Option<FuncId>,
    release: Option<FuncId>,
}

impl Tables {
    pub(super) fn add_function(
        &mut self,
        module: &mut dyn Module,
        func: FuncId,
        frame: &Frame,
        pads: Option<&Vec<u8>>,
    ) -> Result<(), String> {
        if pads.is_some() {
            self.personality(module)?;
        }
        self.entries.push(Entry { func, frame: frame.clone(), pads: pads.cloned() });
        Ok(())
    }

    /// The function of this object that `symbol` names, if it is one of ours.
    pub(super) fn runtime(
        &mut self,
        module: &mut dyn Module,
        symbol: &str,
    ) -> Result<Option<FuncId>, String> {
        let (slot, build): (_, fn(&mut Shim<'_>)) = match symbol {
            RESUME => (&mut self.resume, resume),
            RELEASE => (&mut self.release, release),
            _ => return Ok(None),
        };
        if let Some(id) = *slot {
            return Ok(Some(id));
        }
        let id = self.define(module, symbol, &[I64], &[], build)?;
        match symbol {
            RESUME => self.resume = Some(id),
            _ => self.release = Some(id),
        }
        Ok(Some(id))
    }

    fn personality(&mut self, module: &mut dyn Module) -> Result<FuncId, String> {
        if let Some(id) = self.personality {
            return Ok(id);
        }
        let id = self.define(module, PERSONALITY, &[I64; 4], &[I32], personality)?;
        self.personality = Some(id);
        Ok(id)
    }

    fn define(
        &mut self,
        module: &mut dyn Module,
        name: &str,
        params: &[ir::Type],
        returns: &[ir::Type],
        build: fn(&mut Shim<'_>),
    ) -> Result<FuncId, String> {
        let sig = signature(module, params, returns);
        let id = module.declare_function(name, Linkage::Local, &sig).map_err(|e| e.to_string())?;
        let mut ctx = Context::new();
        ctx.func.signature = sig;
        let mut builder_ctx = FunctionBuilderContext::new();
        let mut error = None;
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut builder_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            let mut shim = Shim { builder, module, entry, error: &mut error };
            build(&mut shim);
            shim.builder.seal_all_blocks();
            let config = shim.module.target_config();
            shim.builder.finalize(config);
        }
        if let Some(error) = error {
            return Err(error);
        }
        module.define_function(id, &mut ctx).map_err(|e| e.to_string())?;
        // The unwinder passes these frames too: each calls what unwinds or throws.
        if let Some(FunctionUnwind { frame: super::Frame::Windows(frame), .. }) =
            FunctionUnwind::of(module.isa(), &ctx)?
        {
            self.entries.push(Entry { func: id, frame, pads: None });
        }
        Ok(id)
    }

    /// Writes an `UNWIND_INFO` for each function to `.xdata`, and to `.pdata` where each
    /// function is and where its `UNWIND_INFO` is.
    pub(super) fn emit(self, product: &mut ObjectProduct) -> Result<(), String> {
        if self.entries.is_empty() {
            return Ok(());
        }
        let personality = self.personality.map(|id| product.function_symbol(id));
        let mut xdata = Vec::new();
        let mut pdata = Vec::new();
        // (offset in `.xdata` of the address of the personality)
        let mut handlers = Vec::new();
        // (the function, how long it is, where its `UNWIND_INFO` is in `.xdata`)
        let mut functions = Vec::new();
        // The linker takes the table in the order of the addresses, which is not the order
        // the functions came in: what a function needs of ours is defined after it.
        let mut entries = self.entries;
        entries.sort_by_key(|entry| {
            let symbol = product.object.symbol(product.function_symbol(entry.func));
            symbol.value
        });
        for entry in &entries {
            functions.push((product.function_symbol(entry.func), entry.frame.len, xdata.len()));
            let start = xdata.len();
            xdata.extend_from_slice(&entry.frame.info);
            if let Some(pads) = &entry.pads {
                xdata[start] |= UNW_FLAG_EHANDLER << 3;
                handlers.push(xdata.len());
                xdata.extend_from_slice(&[0; 4]);
                xdata.extend_from_slice(&((pads.len() / Pads::ENTRY) as u32).to_le_bytes());
                xdata.extend_from_slice(pads);
            }
            pdata.extend_from_slice(&[0; 12]);
        }
        let object = &mut product.object;
        let xdata_section =
            object.add_section(Vec::new(), b".xdata".to_vec(), SectionKind::ReadOnlyData);
        object.section_mut(xdata_section).set_data(xdata, 4);
        let pdata_section =
            object.add_section(Vec::new(), b".pdata".to_vec(), SectionKind::ReadOnlyData);
        object.section_mut(pdata_section).set_data(pdata, 4);
        let xdata_symbol = object.section_symbol(xdata_section);

        let mut relocate = |section: SectionId, offset: usize, symbol: SymbolId, addend: i64| {
            // What is local is addressed through its section.
            let (symbol, within) = object.symbol_section_and_offset(symbol).unwrap_or((symbol, 0));
            object
                .add_relocation(
                    section,
                    Relocation {
                        offset: offset as u64,
                        symbol,
                        flags: RelocationFlags::Generic {
                            kind: RelocationKind::ImageOffset,
                            encoding: RelocationEncoding::Generic,
                            size: 32,
                        },
                        addend: within as i64 + addend,
                    },
                )
                .map_err(|e| e.to_string())
        };
        for offset in handlers {
            let Some(personality) = personality else {
                return Err("landing pads without a personality".to_owned());
            };
            relocate(xdata_section, offset, personality, 0)?;
        }
        for (index, (symbol, len, info)) in functions.into_iter().enumerate() {
            let at = index * 12;
            relocate(pdata_section, at, symbol, 0)?;
            relocate(pdata_section, at + 4, symbol, i64::from(len))?;
            relocate(pdata_section, at + 8, xdata_symbol, info as i64)?;
        }
        Ok(())
    }
}

fn signature(module: &dyn Module, params: &[ir::Type], returns: &[ir::Type]) -> Signature {
    let mut sig = module.make_signature();
    sig.params.extend(params.iter().copied().map(AbiParam::new));
    sig.returns.extend(returns.iter().copied().map(AbiParam::new));
    sig
}

/// A function of ours being built.
struct Shim<'a> {
    builder: FunctionBuilder<'a>,
    module: &'a mut dyn Module,
    entry: ir::Block,
    error: &'a mut Option<String>,
}

impl Shim<'_> {
    fn param(&self, index: usize) -> Value {
        self.builder.block_params(self.entry)[index]
    }

    /// Calls a function of the system.
    fn system(
        &mut self,
        name: &str,
        params: &[ir::Type],
        returns: &[ir::Type],
        args: &[Value],
    ) -> Option<Value> {
        let sig = signature(self.module, params, returns);
        let signature = self.builder.func.import_signature(sig.clone());
        let id = match self.module.declare_function(name, Linkage::Import, &sig) {
            Ok(id) => id,
            Err(error) => {
                // Reported when the function is built: the call is to nowhere meanwhile.
                self.error.get_or_insert(error.to_string());
                let null = self.builder.ins().iconst(I64, 0);
                let call = self.builder.ins().call_indirect(signature, null, args);
                return self.builder.inst_results(call).first().copied();
            }
        };
        let name = self
            .builder
            .func
            .declare_imported_user_function(UserExternalName { namespace: 0, index: id.as_u32() });
        // The linker puts a stub where what a DLL exports is called.
        let callee = self.builder.func.import_function(ExtFuncData {
            name: ExternalName::user(name),
            signature,
            colocated: true,
            patchable: false,
        });
        let call = self.builder.ins().call(callee, args);
        self.builder.inst_results(call).first().copied()
    }

    fn load(&mut self, ty: ir::Type, base: Value, offset: i32) -> Value {
        self.builder.ins().load(ty, MEM, base, offset)
    }

    /// A `u32` as the 64 bits addresses are computed in.
    fn load_u32(&mut self, base: Value, offset: i32) -> Value {
        self.builder.ins().uload32(MEM, base, offset)
    }

    fn store(&mut self, value: Value, base: Value, offset: i32) {
        self.builder.ins().store(MEM, value, base, offset);
    }

    /// Copies `len` bytes, which may be none.
    fn copy(&mut self, to: Value, from: Value, len: Value) {
        let [head, body, done] = std::array::from_fn(|_| self.builder.create_block());
        self.builder.append_block_param(head, I64);
        let zero = self.builder.ins().iconst(I64, 0);
        self.builder.ins().jump(head, &[zero.into()]);
        self.builder.switch_to_block(head);
        let at = self.builder.block_params(head)[0];
        let more = self.builder.ins().icmp(IntCC::UnsignedLessThan, at, len);
        self.builder.ins().brif(more, body, &[], done, &[]);
        self.builder.switch_to_block(body);
        let source = self.builder.ins().iadd(from, at);
        let byte = self.load(I8, source, 0);
        let target = self.builder.ins().iadd(to, at);
        self.store(byte, target, 0);
        let next = self.builder.ins().iadd_imm_s(at, 1);
        self.builder.ins().jump(head, &[next.into()]);
        self.builder.switch_to_block(done);
    }

    fn heap(&mut self) -> Value {
        self.system("GetProcessHeap", &[], &[I64], &[]).expect("a result")
    }

    fn free(&mut self, block: Value) {
        let heap = self.heap();
        let flags = self.builder.ins().iconst(I32, 0);
        self.system("HeapFree", &[I64, I32, I64], &[I32], &[heap, flags, block]);
    }
}

/// `EXCEPTION_DISPOSITION personality(EXCEPTION_RECORD *, void *frame, CONTEXT *,
/// DISPATCHER_CONTEXT *)`: what the unwinder calls for a frame of a function with landing
/// pads. The data that follows the handler in the function's `UNWIND_INFO` is the number
/// of entries of [`Pads`] and then the entries.
fn personality(shim: &mut Shim<'_>) {
    let (record, frame, dispatch) = (shim.param(0), shim.param(1), shim.param(3));
    let [search, scan, check, next, found, sized, claim, arrive, enter, unwind, pass] =
        std::array::from_fn(|_| shim.builder.create_block());
    shim.builder.append_block_param(scan, I64);
    shim.builder.append_block_param(unwind, I64);

    // Frames being unwound have nothing to do: a pad is entered when a handler is looked
    // for, so a frame of ours is never unwound past with a pad left to run.
    let flags = shim.load_u32(record, 4);
    let unwinding = shim.builder.ins().band_imm_u(flags, EXCEPTION_UNWIND);
    shim.builder.ins().brif(unwinding, pass, &[], search, &[]);

    shim.builder.switch_to_block(search);
    let code = shim.load_u32(record, 0);
    let parameters = shim.load_u32(record, 24);
    let cxx = shim.builder.ins().icmp_imm_u(IntCC::Equal, code, i64::from(CXX_EXCEPTION));
    let four = shim.builder.ins().icmp_imm_u(IntCC::Equal, parameters, 4);
    let ours = shim.builder.ins().band(cxx, four);
    let control = shim.load(I64, dispatch, 0);
    let image = shim.load(I64, dispatch, 8);
    let entry = shim.load(I64, dispatch, 16);
    let pads = shim.load(I64, dispatch, 56);
    let begin = shim.load_u32(entry, 0);
    let start = shim.builder.ins().iadd(image, begin);
    let returned = shim.builder.ins().isub(control, start);
    let count = shim.load_u32(pads, 0);
    let zero = shim.builder.ins().iconst(I64, 0);
    shim.builder.ins().brif(ours, scan, &[zero.into()], pass, &[]);

    shim.builder.switch_to_block(scan);
    let index = shim.builder.block_params(scan)[0];
    let more = shim.builder.ins().icmp(IntCC::UnsignedLessThan, index, count);
    shim.builder.ins().brif(more, check, &[], pass, &[]);

    shim.builder.switch_to_block(check);
    let offset = shim.builder.ins().imul_imm_s(index, Pads::ENTRY as i64);
    let call = shim.builder.ins().iadd(pads, offset);
    let after = shim.load_u32(call, 4);
    let until = shim.load_u32(call, 8);
    let pad = shim.load_u32(call, 12);
    let after = shim.builder.ins().icmp(IntCC::UnsignedLessThan, after, returned);
    let until = shim.builder.ins().icmp(IntCC::UnsignedLessThanOrEqual, returned, until);
    let hit = shim.builder.ins().band(after, until);
    shim.builder.ins().brif(hit, found, &[], next, &[]);

    shim.builder.switch_to_block(next);
    let following = shim.builder.ins().iadd_imm_s(index, 1);
    shim.builder.ins().jump(scan, &[following.into()]);

    shim.builder.switch_to_block(found);
    let object = shim.load(I64, record, 40);
    let throw_info = shim.load(I64, record, 48);
    let thrown_from = shim.load(I64, record, 56);
    // `throw;` has no `ThrowInfo`: it throws again what a C++ handler is in the middle of.
    shim.builder.ins().brif(throw_info, sized, &[], pass, &[]);

    // The size is with the first of the types the object can be caught as.
    shim.builder.switch_to_block(sized);
    let types = shim.load_u32(throw_info, 12);
    let types = shim.builder.ins().iadd(thrown_from, types);
    let first = shim.load_u32(types, 4);
    let first = shim.builder.ins().iadd(thrown_from, first);
    let size = shim.load_u32(first, 20);
    let heap = shim.heap();
    let no_flags = shim.builder.ins().iconst(I32, 0);
    let whole = shim.builder.ins().iadd_imm_s(size, OBJECT);
    let block = shim
        .system("HeapAlloc", &[I64, I32, I64], &[I64], &[heap, no_flags, whole])
        .expect("a result");
    shim.builder.ins().brif(block, claim, &[], pass, &[]);

    shim.builder.switch_to_block(claim);
    shim.store(throw_info, block, 0);
    shim.store(thrown_from, block, 8);
    shim.store(size, block, 16);
    let copy = shim.builder.ins().iadd_imm_s(block, OBJECT);
    shim.copy(copy, object, size);
    shim.builder.ins().brif(pad, enter, &[], arrive, &[]);

    shim.builder.switch_to_block(enter);
    let pad = shim.builder.ins().iadd(start, pad);
    shim.builder.ins().jump(unwind, &[pad.into()]);

    // The frame the unwinder names is the frame pointer less what the `UNWIND_INFO` says.
    shim.builder.switch_to_block(arrive);
    let info = shim.load_u32(entry, 8);
    let info = shim.builder.ins().iadd(image, info);
    let register = shim.builder.ins().uload8(I64, MEM, info, 3);
    let above = shim.builder.ins().ushr_imm_u(register, 4);
    let above = shim.builder.ins().ishl_imm_u(above, 4);
    let frame_pointer = shim.builder.ins().iadd(frame, above);
    shim.store(block, frame_pointer, ARRIVAL);
    shim.builder.ins().jump(unwind, &[control.into()]);

    // The unwinder puts `block` in the register a function returns in.
    shim.builder.switch_to_block(unwind);
    let target = shim.builder.block_params(unwind)[0];
    let context = shim.load(I64, dispatch, 40);
    let history = shim.load(I64, dispatch, 64);
    shim.system("RtlUnwindEx", &[I64; 6], &[], &[frame, target, record, block, context, history]);
    shim.builder.ins().trap(TrapCode::user(1).expect("a nonzero trap code"));

    shim.builder.switch_to_block(pass);
    let disposition = shim.builder.ins().iconst(I32, CONTINUE_SEARCH);
    shim.builder.ins().return_(&[disposition]);
}

/// `resume(exception) -> !`: throws the object again, from this frame when it fits, so
/// that what catches it has nothing of ours to free.
fn resume(shim: &mut Shim<'_>) {
    let exception = shim.param(0);
    let [inline, raise] = std::array::from_fn(|_| shim.builder.create_block());
    shim.builder.append_block_param(raise, I64);
    let throw_info = shim.load(I64, exception, 0);
    let thrown_from = shim.load(I64, exception, 8);
    let size = shim.load(I64, exception, 16);
    let object = shim.builder.ins().iadd_imm_s(exception, OBJECT);
    let fits = shim.builder.ins().icmp_imm_u(
        IntCC::UnsignedLessThanOrEqual,
        size,
        i64::from(INLINE_OBJECT),
    );
    shim.builder.ins().brif(fits, inline, &[], raise, &[object.into()]);

    shim.builder.switch_to_block(inline);
    let slot = shim.builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        INLINE_OBJECT,
        4,
    ));
    let own = shim.builder.ins().stack_addr(I64, slot, 0);
    shim.copy(own, object, size);
    shim.free(exception);
    shim.builder.ins().jump(raise, &[own.into()]);

    shim.builder.switch_to_block(raise);
    let object = shim.builder.block_params(raise)[0];
    let slot = shim.builder.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        32,
        3,
    ));
    let parameters = shim.builder.ins().stack_addr(I64, slot, 0);
    let magic = shim.builder.ins().iconst(I64, CXX_MAGIC);
    for (index, value) in [magic, object, throw_info, thrown_from].into_iter().enumerate() {
        shim.store(value, parameters, index as i32 * 8);
    }
    let code = shim.builder.ins().iconst(I32, i64::from(CXX_EXCEPTION));
    let flags = shim.builder.ins().iconst(I32, EXCEPTION_NONCONTINUABLE);
    let count = shim.builder.ins().iconst(I32, 4);
    shim.system("RaiseException", &[I32, I32, I32, I64], &[], &[code, flags, count, parameters]);
    shim.builder.ins().trap(TrapCode::user(1).expect("a nonzero trap code"));
}

/// `release(exception)`: frees what a pad that stops the unwind was entered with.
fn release(shim: &mut Shim<'_>) {
    let exception = shim.param(0);
    shim.free(exception);
    shim.builder.ins().return_(&[]);
}
