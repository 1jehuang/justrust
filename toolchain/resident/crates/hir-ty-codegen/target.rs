//! What ra-jit emits code for. The target is an input of the process, named by its triple
//! (`ra-jit --target <triple>`), and the host when nothing names one. Everything in the
//! backend that differs between platforms asks the [`Target`] for the property it depends
//! on (architecture, object format, calling convention, unwind tables) and never asks what
//! the daemon itself was built for.

use std::sync::OnceLock;

use cranelift_codegen::{
    ir::{Type, types},
    isa::{self, CallConv, OwnedTargetIsa},
    settings::{self, Configurable},
};

static SELECTED: OnceLock<Target> = OnceLock::new();

#[derive(Debug)]
pub struct Target {
    triple: String,
    pub arch: Arch,
    pub object_format: ObjectFormat,
    /// The convention of `extern "C"`, which the Rust conventions are laid over.
    pub call_conv: CallConv,
    pub unwind: UnwindFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
    /// WebAssembly with 32-bit addresses.
    Wasm32,
}

impl Arch {
    /// How long a function is at least, and the byte one that is shorter is filled up with:
    /// the length of a jump from its entry to anywhere a replacement of it may be. Nothing
    /// when every function is that long as it is.
    pub fn shortest_function(self) -> Option<(usize, u8)> {
        match self {
            // `jmp rel32`, filled up with `int3`.
            Arch::X86_64 => Some((5, 0xcc)),
            // `adrp x16`, `add x16` and `br x16`, as `b` reaches only 128 MB, filled up with
            // `udf #0`.
            Arch::Aarch64 => Some((12, 0)),
            // A function of a module is replaced by its slot in the table, not by a jump.
            Arch::Wasm32 => None,
        }
    }
}

/// Where the variable arguments of a C function are when it is entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariableArguments {
    /// In the registers the named parameters left, then on the stack: System V for x86-64
    /// and AArch64.
    Registers,
    /// In the integer registers the named parameters left, which the function writes to the
    /// room the caller keeps below the ones on the stack: Windows x64.
    Homed,
    /// On the stack, eight bytes each, whatever registers are left: Apple's AArch64.
    Stack,
    /// In memory the caller writes them to, each at its own alignment right after the one
    /// before, whose address is one more argument after the named ones: WebAssembly's C.
    Buffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectFormat {
    Elf,
    MachO,
    Coff,
    /// Relocatable WebAssembly objects.
    Wasm,
}

impl ObjectFormat {
    /// What a label starts with for the assembler to keep it out of the symbols.
    pub(super) fn local_label(self) -> &'static str {
        match self {
            ObjectFormat::MachO => "L",
            ObjectFormat::Elf | ObjectFormat::Coff | ObjectFormat::Wasm => ".L",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnwindFormat {
    /// DWARF call frame information in `.eh_frame`, landing pads in `.gcc_except_table`.
    EhFrame,
    /// The unwind codes of Windows in `.pdata` and `.xdata`, see `unwind::seh`.
    Seh,
    /// Nothing unwinds: a panic aborts.
    None,
}

/// How wasm code calls a function it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Calls {
    /// `call`, as rustc's code does.
    Direct,
    /// `call_indirect` through the slot the linker gives the function in the function
    /// table, so that a module loaded later replaces the function by writing its slot.
    Table,
}

impl Target {
    /// Names the target of this process. It is said once, before anything is compiled.
    pub fn select(triple: &str) -> Result<&'static Target, String> {
        let target = Target::parse(triple)?;
        let selected = SELECTED.get_or_init(|| {
            hir_ty::mir::target::select_word(target.word());
            target
        });
        if selected.triple == triple {
            Ok(selected)
        } else {
            Err(format!("the target is {} already, not {triple}", selected.triple))
        }
    }

    /// The target that was named, or the host.
    pub fn selected() -> Result<&'static Target, String> {
        match SELECTED.get() {
            Some(target) => Ok(target),
            None => Target::select(Target::host()),
        }
    }

    /// The triple of what the daemon itself runs on.
    pub fn host() -> &'static str {
        // The wasm backend writes its own code from Cranelift's IR and only borrows the
        // settings of an ISA Cranelift has, whatever the compiler itself runs on.
        if cfg!(feature = "wasm") {
            return "wasm32-unknown-unknown";
        }
        // FIXME: the triples of the hosts that have no code emitted for them yet are missing,
        // and `parse` turns those down by name.
        match (std::env::consts::ARCH, std::env::consts::OS) {
            ("x86_64", "linux") => "x86_64-unknown-linux-gnu",
            ("aarch64", "linux") => "aarch64-unknown-linux-gnu",
            ("aarch64", "macos") => "aarch64-apple-darwin",
            ("x86_64", "macos") => "x86_64-apple-darwin",
            ("x86_64", "windows") => "x86_64-pc-windows-msvc",
            ("aarch64", "windows") => "aarch64-pc-windows-msvc",
            _ => "unknown",
        }
    }

    fn parse(triple: &str) -> Result<Target, String> {
        let mut parts = triple.split('-');
        let arch = match parts.next() {
            Some("x86_64") => Arch::X86_64,
            Some("aarch64") => Arch::Aarch64,
            Some("wasm32") => Arch::Wasm32,
            _ => return Err(format!("blaze emits no code for the architecture of {triple}")),
        };
        let system = parts.find(|part| matches!(*part, "linux" | "darwin" | "windows"));
        // Cranelift's names for the C convention: `SystemV` is the psABI's on x86-64 and
        // AAPCS64 on AArch64, which Apple's differs from in what goes on the stack.
        let (object_format, unwind, call_conv) = match (system, arch) {
            // The calling convention is the one of the ISA whose settings are borrowed.
            (_, Arch::Wasm32) => (ObjectFormat::Wasm, UnwindFormat::None, CallConv::SystemV),
            (Some("linux"), _) => (ObjectFormat::Elf, UnwindFormat::EhFrame, CallConv::SystemV),
            (Some("darwin"), Arch::Aarch64) => {
                (ObjectFormat::MachO, UnwindFormat::EhFrame, CallConv::AppleAarch64)
            }
            (Some("windows"), Arch::X86_64) if triple.ends_with("-msvc") => {
                (ObjectFormat::Coff, UnwindFormat::Seh, CallConv::WindowsFastcall)
            }
            // FIXME: x86-64 Mach-O and AArch64 PE have no splice written for them, and the
            // GNU flavour of Windows unwinds another way than the tables of `unwind::seh`.
            _ => return Err(format!("blaze emits no code for the system of {triple}")),
        };
        Ok(Target { triple: triple.to_owned(), arch, object_format, call_conv, unwind })
    }

    /// The target of the process, once it is known that there is one: nothing is compiled
    /// before [`Target::selected`] has said so.
    pub fn current() -> &'static Target {
        Target::selected().unwrap_or_else(|error| panic!("{error}"))
    }

    pub fn is_wasm(&self) -> bool {
        self.arch == Arch::Wasm32
    }

    /// Whether an object is written as a relocatable wasm object, for a linker. A page links
    /// what the compiler emits itself, and reads the object Cranelift's writer made.
    pub fn writes_wasm_objects(&self) -> bool {
        self.is_wasm() && self.calls() == Calls::Direct
    }

    /// How wasm code calls: through the table where functions are replaced in a page.
    pub fn calls(&self) -> Calls {
        if cfg!(feature = "wasm") { Calls::Table } else { Calls::Direct }
    }

    /// The integer an address is.
    pub(super) fn pointer(&self) -> Type {
        match self.arch {
            Arch::X86_64 | Arch::Aarch64 => types::I64,
            Arch::Wasm32 => types::I32,
        }
    }

    /// How many bytes an address takes.
    pub(crate) fn word(&self) -> usize {
        self.pointer().bytes() as usize
    }

    pub fn triple(&self) -> &str {
        &self.triple
    }

    /// The symbol of the `main` the C runtime calls, and whether it takes the count and the
    /// arguments: rustc's `entry-name` and `main-needs-argc-argv` of the target. WASI's
    /// `_start` calls `__main_void`, and std reads the arguments from WASI itself; elsewhere
    /// on wasm a C `main(argc, argv)` is `__main_argc_argv`, as LLVM names the `main` rustc
    /// gives it.
    pub fn entry(&self) -> (&'static str, bool) {
        match self.arch {
            Arch::Wasm32 if self.triple.contains("-wasi") => ("__main_void", false),
            Arch::Wasm32 => ("__main_argc_argv", true),
            Arch::X86_64 | Arch::Aarch64 => ("main", true),
        }
    }

    /// Whether the program is linked with a C library, for `write` and `abort`: not on a wasm
    /// that is not WASI's, as wasm32-unknown-unknown has nothing to link but the program.
    pub fn has_libc(&self) -> bool {
        self.arch != Arch::Wasm32 || self.triple.contains("-wasi")
    }

    /// Whether a call can name its landing pad and the pad is entered with the exception in a
    /// register, as Cranelift has it for the calling conventions it knows the register of.
    pub fn exception_in_register(&self) -> bool {
        self.call_conv.supports_exceptions()
    }

    /// Where a function that takes `...` finds its variable arguments, for the conventions a
    /// definition of one is compiled for.
    pub fn variable_arguments(&self) -> Option<VariableArguments> {
        match (self.call_conv, self.arch) {
            (_, Arch::Wasm32) => Some(VariableArguments::Buffer),
            (CallConv::SystemV, Arch::X86_64 | Arch::Aarch64) => Some(VariableArguments::Registers),
            (CallConv::WindowsFastcall, Arch::X86_64) => Some(VariableArguments::Homed),
            (CallConv::AppleAarch64, Arch::Aarch64) => Some(VariableArguments::Stack),
            _ => None,
        }
    }

    /// Whether code reads the frame pointer, which every function then has to keep.
    fn reads_frame_pointer(&self) -> bool {
        // The pads of a convention Cranelift has no landing pads for take the exception
        // from the frame (see `unwind::seh`), which the frame pointer finds. Variable
        // arguments that are all on the stack are found from it too.
        !self.exception_in_register() || self.variable_arguments() == Some(VariableArguments::Stack)
    }

    /// The processor code is made for: the baseline of the target, as rustc compiles for.
    pub fn cpu(&self) -> &'static str {
        match (self.arch, self.object_format) {
            (Arch::X86_64, _) => "x86-64",
            (Arch::Wasm32, _) => "generic",
            (Arch::Aarch64, ObjectFormat::MachO) => "apple-m1",
            (Arch::Aarch64, _) => "generic",
        }
    }

    /// What LLVM is told of how a frame larger than a page reaches the stack it needs. A PE
    /// image has `__chkstk` for it, which LLVM calls unasked; elsewhere the code touches
    /// each page itself. Cranelift's code touches each page inline everywhere ([`Self::isa`]).
    pub fn stack_probe(&self) -> Option<&'static str> {
        (self.object_format != ObjectFormat::Coff).then_some("inline-asm")
    }

    /// The section a static with a name gets to itself, where a symbol does not say how
    /// large it is and the section does. Without one a static is as large as the next thing
    /// in its section is far, which counts what that is aligned by. The linker merges
    /// these into the section before the `$`, and the name is not one the writer of the
    /// object would put anything else in.
    pub fn section_of_static(&self, writable: bool) -> Option<&'static str> {
        match self.object_format {
            ObjectFormat::Coff => Some(if writable { ".data$static" } else { ".rdata$static" }),
            ObjectFormat::Elf | ObjectFormat::MachO | ObjectFormat::Wasm => None,
        }
    }

    /// The code generator for the target. `regalloc` names Cranelift's register allocator.
    /// With `frame_pointers` every function it compiles keeps one, which a function that
    /// asks for its frame pointer needs.
    pub(super) fn isa(
        &self,
        regalloc: &str,
        frame_pointers: bool,
    ) -> Result<OwnedTargetIsa, String> {
        let mut flags = settings::builder();
        // An image of PE reaches all of itself relative to the instruction, and what it
        // imports through the stubs the linker adds: there is no table of addresses to go
        // through.
        let (pic, tls_model) = match self.object_format {
            ObjectFormat::Elf => ("true", "elf_gd"),
            ObjectFormat::MachO => ("true", "macho"),
            ObjectFormat::Coff => ("false", "coff"),
            ObjectFormat::Wasm => ("true", "elf_gd"),
        };
        flags.set("is_pic", pic).map_err(|e| e.to_string())?;
        flags.set("tls_model", tls_model).map_err(|e| e.to_string())?;
        // A frame larger than a page touches each page on its way down, as rustc's code does
        // (`probe-stack=inline-asm`, [`Target::stack_probe`]). Windows commits a stack one
        // page at a time as the guard page below the last is touched; elsewhere a thread's
        // stack ends in a guard page that a frame larger than it would step over, into
        // whatever is mapped below, rather than fault where std says the stack overflowed.
        // WebAssembly's stack is the engine's.
        if self.arch != Arch::Wasm32 {
            flags.set("enable_probestack", "true").map_err(|e| e.to_string())?;
            flags.set("probestack_strategy", "inline").map_err(|e| e.to_string())?;
        }
        if frame_pointers || self.reads_frame_pointer() {
            // Cranelift gives a function its frame pointer only when all of them keep one.
            flags.set("preserve_frame_pointers", "true").map_err(|e| e.to_string())?;
        }
        flags.set("opt_level", "none").map_err(|e| e.to_string())?;
        // Cranelift checks the IR of every function it is given unless told not to.
        let verify = if cfg!(debug_assertions) { "true" } else { "false" };
        flags.set("enable_verifier", verify).map_err(|e| e.to_string())?;
        flags.set("enable_llvm_abi_extensions", "true").map_err(|e| e.to_string())?;
        flags.set("regalloc_algorithm", regalloc).map_err(|e| e.to_string())?;
        // Cranelift has no ISA for WebAssembly: its code is written from the IR, and the
        // settings of this one are what the IR is made under.
        let triple = match self.arch {
            Arch::Wasm32 => "x86_64-unknown-linux-gnu",
            Arch::X86_64 | Arch::Aarch64 => &self.triple,
        };
        let mut isa = isa::lookup_by_name(triple).map_err(|e| e.to_string())?;
        if self.arch == Arch::X86_64 {
            // Only a function of `#[target_feature(enable = "cmpxchg16b")]` compares and
            // swaps 128 bits, as `core::arch::x86_64::cmpxchg16b` does.
            isa.enable("has_cmpxchg16b").map_err(|e| e.to_string())?;
        }
        isa.finish(settings::Flags::new(flags)).map_err(|e| e.to_string())
    }
}
