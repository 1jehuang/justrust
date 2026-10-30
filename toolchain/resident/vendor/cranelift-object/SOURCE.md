# cranelift-object

- Upstream: https://github.com/bytecodealliance/wasmtime (`cranelift/object`), crates.io
  `cranelift-object` 0.136.2 (sha256 7e1d0b304a1a852087669b13a26f341c1984c15ba0f4f2d9b3182e67e69782c4,
  git 3c8a3e79aa).
- Licence: Apache-2.0 WITH LLVM-exception (LICENSE).
- Left out: tests/ (and the dev-dependencies and `[[test]]` they need), Cargo.lock.

Changes, each marked `blaze:` in src/backend.rs:

1. Data objects that name the same custom section (segment, name, kind and Mach-O type and
   attributes) share one section, as in LLVM's objects. Upstream adds a section for each, and a
   Mach-O object has room for 255: blaze writes a program as one object, so 255 statics with a
   `#[link_section]` across all its crates (`inventory::submit!` and `#[ctor]` each put one in
   `__DATA,__mod_init_func`) failed with `Too many sections` (#724). On ELF and COFF a data
   object keeps a section of its own where `per_data_object_section` or `#[used]` asks for one,
   as `add_subsection` does for the standard sections, since there the section is what the
   linker keeps or drops and what `SHF_GNU_RETAIN` is on. Mach-O objects go by symbols
   (`MH_SUBSECTIONS_VIA_SYMBOLS`, `N_NO_DEAD_STRIP`), so there they always share.
2. A zero-initialized data object in a custom section is written as zeros: the section is not
   one of uninitialized data, and another data object may follow it there.
3. `used` marks a Mach-O symbol `N_NO_DEAD_STRIP` after the symbol is defined. Upstream did it
   before, which fixed the symbol's flags as those of an undefined one: the object had each
   `used` data object as an undefined external (`n_type` `N_UNDF | N_EXT`) with a section.
