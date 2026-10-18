//! Direct Linux x86-64 backend. No external compiler, assembler, or linker.
//!
//! Private ABI: arguments are evaluated left-to-right and pushed as eight-byte
//! stack words. The callee reads them in reverse order above rbp. eax carries
//! bool results (unit is zero), while rax carries normalized <=64-bit integers.
//! The i128/u128 private carrier is an indirect 16-byte scalar snapshot, low
//! limb at offset 0 and high limb at offset 8, with eight-byte alignment.
//! Wide parameters use one snapshot pointer word and wide results use sret.
//! Signed integers are sign-extended and unsigned integers zero-extended from
//! their declared width. References remain a separate thin-pointer class,
//! including borrowed receiver argument0 and references to aggregate referents.
//! All registers are caller-saved. Frames use
//! stable rbp-relative slots, independent of temporary pushes. At most 4096
//! parameters/arguments are accepted. This is not a platform interoperability ABI.
//! Aggregate layout is private: ordinary scalar/unit leaves occupy eight bytes,
//! wide integer leaves occupy sixteen bytes,
//! tuples and nominal structs concatenate their fields. Payload enums contain
//! an eight-byte tag followed by the largest variant's concatenated fields.
//! Any non-unit variant shape makes the entire enum use aggregate transport,
//! including unit variants of that enum and zero-field tuple/struct variants.
//! Arrays are always indirect and concatenate element storage. Empty arrays
//! own one initialized eight-byte dummy word, including when nested as elements.
//! Aggregate arguments carry snapshot pointers. Aggregate results use a
//! caller-owned destination, passed last at rbp+16, even for a one-field aggregate.
use super::cfg;
use super::frontend::{BinaryOp, Program, UnaryOp};
use super::ir::{self, LocalId};
use super::ownership;
use std::collections::HashMap;

mod wide_float;
mod wide_int;

/// Physical storage only. Wide integers remain scalar Copy values in analysis.
fn wide_integer(ty: &ir::Ty) -> bool {
    matches!(ty, ir::Ty::Int(integer) if integer.bits() == 128)
}

const WIDE_SCRATCH_BYTES: i32 = 256;

const MAX_ARGS: usize = 4096;
/// Expanded stack-word limit per call: every logical argument may occupy at
/// most two words (shared slice: data, len) plus one sret word.
const MAX_ARG_WORDS: usize = 2 * MAX_ARGS + 1;
const MAX_FRAME_BYTES: i32 = 1024 * 1024;
const BASE: u64 = 0x400000;
const PAGE: usize = 4096;

/// Shape-only physical descriptor of a shared slice carrier. It deliberately
/// has no source root, origin, lifetime, local, array identity or static
/// extent: the length is a runtime word copied from validated construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SliceLayout {
    pub data_offset: u8,
    pub len_offset: u8,
    pub word_bytes: u8,
    /// Exact private array element size, not Rust/FFI layout. This is compile
    /// time metadata only, never a third runtime descriptor word.
    pub element_stride: u32,
}

/// `&[u8]` on this private x86-64 backend: 16-byte carrier storage, true
/// two-word argument transport (data pushed first, then len).
pub const SHARED_BYTE_SLICE_LAYOUT: SliceLayout = SliceLayout {
    data_offset: 0,
    len_offset: 8,
    word_bytes: 8,
    element_stride: 8,
};

fn shared_slice_layout(element: &ir::Ty) -> Result<SliceLayout, String> {
    Ok(SliceLayout {
        element_stride: u32::try_from(value_size(element)?)
            .map_err(|_| "slice element exceeds private layout limit")?,
        ..SHARED_BYTE_SLICE_LAYOUT
    })
}

/// Expanded stack words a parameter of `ty` occupies in the private call ABI.
/// Every currently admitted shape is one word (scalars, thin references and
/// aggregate snapshot pointers). Unsupported shapes error exactly as their
/// transport classification does.
/// Exactly shared `&[T]` is true two-word transport (data, len); it is
/// checked BEFORE `indirect`. Bare `Slice` has no by-value ABI and errors.
fn argument_words(ty: &ir::Ty) -> Result<usize, String> {
    if let Some(element) = ty.shared_slice_element() {
        shared_slice_layout(element)?;
        return Ok(2);
    }
    indirect(ty)?;
    Ok(1)
}

/// Expanded words of a signature's parameters, checked against both the
/// logical MAX_ARGS limit and the expanded MAX_ARG_WORDS limit.
fn parameter_words(params: &[ir::Ty]) -> Result<Vec<usize>, String> {
    if params.len() > MAX_ARGS {
        return Err(format!("at most {MAX_ARGS} parameters are supported"));
    }
    params.iter().map(argument_words).collect()
}

fn total_argument_words(words: &[usize], sret: bool) -> Result<usize, String> {
    words
        .iter()
        .try_fold(usize::from(sret), |total, words| total.checked_add(*words))
        .filter(|total| *total <= MAX_ARG_WORDS)
        .ok_or_else(|| format!("at most {MAX_ARG_WORDS} argument words are supported"))
}

/// Caller cleanup: 8 * (sum of expanded argument words + sret word).
fn call_cleanup_bytes(words: &[usize], sret: bool) -> Result<i32, String> {
    total_argument_words(words, sret)?
        .checked_mul(8)
        .and_then(|bytes| i32::try_from(bytes).ok())
        .ok_or_else(|| "call cleanup exceeds private stack limit".into())
}

/// Callee rbp offsets of each parameter's lowest pushed word:
/// `16 + sret*8 + 8 * sum(words of later params)`. For a two-word slice this
/// addresses len, with data at offset+8.
fn argument_slot_offsets(words: &[usize], sret: bool) -> Result<Vec<i32>, String> {
    total_argument_words(words, sret)?;
    let mut offsets = vec![0; words.len()];
    let mut later = 0_usize;
    for (index, words) in words.iter().enumerate().rev() {
        offsets[index] = later
            .checked_add(usize::from(sret))
            .and_then(|slots| slots.checked_mul(8))
            .and_then(|bytes| bytes.checked_add(16))
            .and_then(|bytes| i32::try_from(bytes).ok())
            .ok_or("argument offset exceeds private stack limit")?;
        later = later.checked_add(*words).ok_or("argument words overflow")?;
    }
    Ok(offsets)
}

/// Only exactly shared `&[T]` has a native carrier. Bare/mutable/nested slices
/// remain unsupported, as do elements rejected by concrete admission.
const SLICE_UNSUPPORTED: &str =
    "unsupported: only shared &[T] parameters and locals have a native slice layout";
const SLICE_RESULT_UNSUPPORTED: &str = "unsupported: shared slice returns have no native ABI";

/// Transport class of a function or call result. Slice results are rejected
/// here, before `indirect` would classify the carrier as snapshot storage.
fn result_indirect(ty: &ir::Ty) -> Result<bool, String> {
    if ty.contains_slice() {
        return Err(SLICE_RESULT_UNSUPPORTED.into());
    }
    indirect(ty)
}

/// Storage transport class. Exactly shared `&[T]` is a 16-byte frame carrier
/// whose operand convention is its snapshot ADDRESS in rax, exactly like
/// aggregates, so local copies move both words. It never crosses a call
/// boundary as a pointer: calls use `argument_words` == 2 and push both words.
fn indirect(ty: &ir::Ty) -> Result<bool, String> {
    ir::reject_associated_types(ty, None)?;
    if let Some(element) = ty.shared_slice_element() {
        shared_slice_layout(element)?;
        return Ok(true);
    }
    if ty.contains_slice() {
        return Err(SLICE_UNSUPPORTED.into());
    }
    if wide_integer(ty) {
        return Ok(true);
    }
    match ty {
        ir::Ty::Tuple(_) | ir::Ty::Struct(_) | ir::Ty::Array { .. } => Ok(true),
        ir::Ty::Enum(ty) => Ok(ty.has_payload()),
        ir::Ty::Int(_)
        | ir::Ty::Float(_)
        | ir::Ty::Bool
        | ir::Ty::Unit
        | ir::Ty::Never
        | ir::Ty::Ref { .. } => Ok(false),
        ir::Ty::Param(_) => Err("type parameter has no native ABI".into()),
        ir::Ty::AssociatedType(_) => Err("unresolved associated type has no native ABI".into()),
        ir::Ty::Slice { .. } => Err(SLICE_UNSUPPORTED.into()),
    }
}

fn value_size(ty: &ir::Ty) -> Result<i32, String> {
    ir::reject_associated_types(ty, None)?;
    if wide_integer(ty) {
        // PRIVATE layout: low limb at 0, high limb at 8, eight-byte alignment.
        return Ok(16);
    }
    if matches!(ty, ir::Ty::Param(_) | ir::Ty::AssociatedType(_)) {
        return Err("unresolved type has no native layout".into());
    }
    if let Some(element) = ty.shared_slice_element() {
        shared_slice_layout(element)?;
        return Ok(2 * i32::from(SHARED_BYTE_SLICE_LAYOUT.word_bytes));
    }
    if ty.contains_slice() {
        return Err(SLICE_UNSUPPORTED.into());
    }
    if let ir::Ty::Struct(structure) = ty
        && structure.def().fields.is_empty()
    {
        // A source zero-sized nominal keeps its nominal type and move rules.
        // The PRIVATE ABI deliberately gives it an initialized dummy word,
        // just like an empty array, so snapshots and references always address
        // owned frame storage. This is not a Rust/FFI size or layout claim.
        return Ok(8);
    }
    if let ir::Ty::Array { element, len } = ty {
        if *len > ir::MAX_ARRAY_LEN {
            return Err("array length exceeds private backend limit".into());
        }
        if *len == 0 {
            // ConcreteCfg's exhaustive metadata walk validated the element,
            // even though no element storage or stride is needed at runtime.
            return Ok(8);
        }
        // Empty arrays still own an initialized dummy word. This rule also
        // determines the stride of arrays whose elements are empty arrays.
        let stride = value_size(element)?;
        let len = i32::try_from(*len).map_err(|_| "array length exceeds private frame limit")?;
        return stride
            .checked_mul(len)
            .filter(|size| *size <= MAX_FRAME_BYTES)
            .map(|size| size.max(8))
            .ok_or_else(|| "array exceeds private frame limit".into());
    }
    if let ir::Ty::Enum(ty) = ty {
        let payload = ty
            .def()
            .variants
            .iter()
            .try_fold(0_i32, |largest, variant| {
                let size = variant.fields.iter().try_fold(0_i32, |size, field| {
                    size.checked_add(value_size(&field.ty)?)
                        .filter(|size| *size <= MAX_FRAME_BYTES - 8)
                        .ok_or_else(|| "enum payload exceeds private frame limit".to_string())
                })?;
                Ok::<_, String>(largest.max(size))
            })?;
        return Ok(8 + payload);
    }
    match ty.field_count() {
        Some(fields) => (0..fields).try_fold(0_i32, |size, index| {
            let field = ty
                .field_ty(index)
                .ok_or("invalid checked aggregate field")?;
            size.checked_add(value_size(field)?)
                .filter(|size| *size <= MAX_FRAME_BYTES)
                .ok_or_else(|| "aggregate exceeds private frame limit".to_string())
        }),
        None => match ty {
            ir::Ty::Int(_)
            | ir::Ty::Float(_)
            | ir::Ty::Bool
            | ir::Ty::Unit
            | ir::Ty::Never
            | ir::Ty::Ref { .. } => Ok(8),
            _ => Err("unknown concrete native layout shape".into()),
        },
    }
}

fn field_offset(ty: &ir::Ty, index: usize) -> Result<(i32, &ir::Ty), String> {
    let field = ty
        .field_ty(index)
        .ok_or("invalid checked aggregate field")?;
    let offset = (0..index).try_fold(0_i32, |offset, index| {
        let ty = ty
            .field_ty(index)
            .ok_or("invalid checked aggregate field")?;
        offset
            .checked_add(value_size(ty)?)
            .ok_or_else(|| "aggregate layout too large".to_string())
    })?;
    Ok((offset, field))
}

fn array_offset(ty: &ir::Ty, index: usize) -> Result<(i32, &ir::Ty), String> {
    let ir::Ty::Array { element, len } = ty else {
        return Err("constant index requires array storage".into());
    };
    if index >= *len {
        return Err("constant array index out of bounds".into());
    }
    let offset = value_size(element)?
        .checked_mul(i32::try_from(index).map_err(|_| "array index exceeds private frame limit")?)
        .filter(|offset| *offset < MAX_FRAME_BYTES)
        .ok_or("array index offset exceeds private frame limit")?;
    Ok((offset, element))
}

fn allocate_value(frame: &mut i32, ty: &ir::Ty) -> Result<i32, String> {
    *frame = frame
        .checked_add(value_size(ty)?)
        .filter(|size| *size <= MAX_FRAME_BYTES)
        .ok_or("function exceeds private frame limit")?;
    Ok(-*frame)
}

/// Relocations refer to byte offsets within `code`: calls address a signed
/// rel32 displacement field, strings address an absolute u64 immediate field.
#[derive(Clone, Debug)]
pub struct Object {
    pub code: Vec<u8>,
    pub calls: Vec<(usize, String)>,
    pub strings: Vec<(usize, Vec<u8>)>,
}

struct Emitter<'a> {
    object: Object,
    concrete: &'a ownership::ConcreteCfg,
    resolve: &'a dyn Fn(&ir::CallTarget) -> Result<String, String>,
    locals: Vec<i32>,
    aggregate_scratch: i32,
    /// 0=LHS, 16=RHS, 32=result, 48..256=wide arithmetic workspace.
    /// Independent of aggregate scratch so constructors cannot destroy inputs.
    wide_scratch: i32,
    returns: Vec<usize>,
    traps: Vec<usize>,
    edges: Vec<(usize, cfg::BlockId)>,
}
impl<'a> Emitter<'a> {
    fn function(&self) -> &'a cfg::Function {
        self.concrete.function().as_ref()
    }
    fn bytes(&mut self, bytes: &[u8]) {
        self.object.code.extend_from_slice(bytes);
    }
    fn i32(&mut self, value: i32) {
        self.bytes(&value.to_le_bytes());
    }
    fn position(&self) -> usize {
        self.object.code.len()
    }
    fn immediate(&mut self, value: i32) {
        self.bytes(&[0xb8]);
        self.i32(value);
    }
    fn word(&mut self, value: u64) {
        self.bytes(&[0x48, 0xb8]);
        self.bytes(&value.to_le_bytes());
    }
    /// Capture an already evaluated numeric value into dedicated two-limb
    /// storage. Wide values arrive by address, other numbers in rax. Only rax
    /// and rdx are clobbered. Source signedness determines the extension.
    fn snapshot_number(&mut self, ty: ir::NumTy, slot: i32) {
        if matches!(ty, ir::NumTy::Int(integer) if integer.bits() == 128) {
            self.bytes(&[0x48, 0x8b, 0x50, 0x08, 0x48, 0x8b, 0x00]);
        } else if matches!(ty, ir::NumTy::Int(integer) if integer.is_signed()) {
            self.bytes(&[0x48, 0x99]); // cqo: source is already normalized
        } else {
            self.bytes(&[0x31, 0xd2]);
        }
        self.bytes(&[0x48, 0x89, 0x85]);
        self.i32(self.wide_scratch + slot);
        self.bytes(&[0x48, 0x89, 0x95]);
        self.i32(self.wide_scratch + slot + 8);
    }
    fn wide_constant(&mut self, value: ir::IntValue) {
        self.word(value.bits as u64);
        self.bytes(&[0x48, 0x89, 0x85]);
        self.i32(self.wide_scratch + 32);
        self.word((value.bits >> 64) as u64);
        self.bytes(&[0x48, 0x89, 0x85]);
        self.i32(self.wide_scratch + 40);
        self.address(self.wide_scratch + 32);
    }
    fn integer_cast(&mut self, value: &cfg::Operand, to: ir::IntTy) -> Result<(), String> {
        let ir::Ty::Int(from) = self.function().operand_ty(value)? else {
            return Err("integer cast requires an integer source".into());
        };
        self.operand(value)?;
        if to.bits() == 128 {
            self.snapshot_number(ir::NumTy::Int(from), 32);
            self.address(self.wide_scratch + 32);
        } else {
            if from.bits() == 128 {
                self.bytes(&[0x48, 0x8b, 0x00]);
            }
            self.normalize_integer(to);
        }
        Ok(())
    }
    fn normalize_integer(&mut self, ty: ir::IntTy) {
        assert!(ty.bits() <= 64, "wide integer reached word normalization");
        if ty.bits() < 64 {
            let shift = (64 - ty.bits()) as u8;
            self.bytes(&[0x48, 0xc1, 0xe0, shift]);
            self.bytes(&[0x48, 0xc1, if ty.is_signed() { 0xf8 } else { 0xe8 }, shift]);
        }
    }
    fn check_integer_range(&mut self, ty: ir::IntTy) {
        assert!(ty.bits() <= 64, "wide integer reached word range check");
        if ty.bits() < 64 {
            self.bytes(&[0x48, 0x89, 0xc2]); // rdx = untruncated result
            self.normalize_integer(ty);
            self.bytes(&[0x48, 0x39, 0xd0]);
            self.trap_if(0x85);
        }
    }
    fn jump(&mut self, opcode: &[u8]) -> usize {
        self.bytes(opcode);
        let at = self.position();
        self.i32(0);
        at
    }
    fn patch(&mut self, at: usize, target: usize) -> Result<(), String> {
        let displacement = i32::try_from(target as i128 - at as i128 - 4)
            .map_err(|_| "function exceeds rel32 range".to_string())?;
        self.object.code[at..at + 4].copy_from_slice(&displacement.to_le_bytes());
        Ok(())
    }
    fn edge(&mut self, opcode: &[u8], target: cfg::BlockId) {
        let at = self.jump(opcode);
        self.edges.push((at, target));
    }
    fn trap_if(&mut self, condition: u8) {
        let at = self.jump(&[0x0f, condition]);
        self.traps.push(at);
    }
    fn local_offset(&self, local: LocalId) -> Result<i32, String> {
        self.locals
            .get(local.0)
            .copied()
            .ok_or_else(|| "invalid CFG local".to_string())
    }
    // Materialize the current address, never substitute a loan's original
    // referent for a carrier that may have been reassigned at runtime.
    // Only rax is clobbered, allowing stores to preserve their RHS in r9.
    fn place_address(&mut self, place: &ir::Place) -> Result<&'a ir::Ty, String> {
        let mut offset = self.local_offset(place.local)?;
        let mut ty: &'a ir::Ty = &self.function().locals[place.local.0].ty;
        let mut indirect_base = false;
        let mut projections = place.projection.iter();
        while let Some(projection) = projections.next() {
            let (field, next) = match projection {
                ir::Projection::Deref => {
                    let ir::Ty::Ref { referent, .. } = ty else {
                        return Err("dereference requires reference storage".into());
                    };
                    if indirect_base {
                        if offset != 0 {
                            self.bytes(&[0x48, 0x05]);
                            self.i32(offset);
                        }
                    } else {
                        self.address(offset);
                    }
                    // Physical capability transport, not a semantic Copy of
                    // the potentially non-Copy mutable-reference carrier.
                    self.bytes(&[0x48, 0x8b, 0x00]);
                    indirect_base = true;
                    offset = 0;
                    ty = referent;
                    continue;
                }
                ir::Projection::Field(index) => field_offset(ty, *index)?,
                ir::Projection::ConstantIndex(index) => array_offset(ty, *index)?,
                ir::Projection::Downcast(variant) => {
                    let ir::Ty::Enum(enum_ty) = ty else {
                        return Err("downcast requires enum storage".into());
                    };
                    if variant.owner != enum_ty.def().id {
                        return Err("downcast enum owner mismatch".into());
                    }
                    let definition = enum_ty
                        .def()
                        .variants
                        .get(variant.index)
                        .ok_or("invalid downcast variant")?;
                    let Some(ir::Projection::Field(index)) = projections.next() else {
                        return Err("downcast must be followed by payload field".into());
                    };
                    let field = definition
                        .fields
                        .get(*index)
                        .ok_or("invalid downcast payload field")?;
                    let offset =
                        definition.fields[..*index]
                            .iter()
                            .try_fold(8_i32, |offset, field| {
                                offset
                                    .checked_add(value_size(&field.ty)?)
                                    .ok_or_else(|| "enum projection offset too large".to_string())
                            })?;
                    (offset, &field.ty)
                }
            };
            offset = offset
                .checked_add(field)
                .ok_or("projection offset too large")?;
            ty = next;
        }
        if indirect_base {
            if offset != 0 {
                self.bytes(&[0x48, 0x05]);
                self.i32(offset);
            }
        } else {
            self.address(offset);
        }
        Ok(ty)
    }
    fn address(&mut self, offset: i32) {
        self.bytes(&[0x48, 0x8d, 0x85]);
        self.i32(offset);
    }
    fn copy_pointer_value(&mut self, size: i32) {
        if size > 64 {
            // Large array transport must not expand generated code per element.
            // Preserve both pointers, especially rdi used by return epilogues.
            self.bytes(&[0x49, 0x89, 0xfb, 0x49, 0x89, 0xf2, 0xb9]);
            self.i32(size / 8);
            self.bytes(&[0xfc, 0xf3, 0x48, 0xa5]);
            self.bytes(&[0x4c, 0x89, 0xdf, 0x4c, 0x89, 0xd6]);
            return;
        }
        // All private scalar/unit leaves are initialized eight-byte words.
        for offset in (0..size).step_by(8) {
            self.bytes(&[0x48, 0x8b, 0x8e]);
            self.i32(offset);
            self.bytes(&[0x48, 0x89, 0x8f]);
            self.i32(offset);
        }
    }
    fn store_value(&mut self, offset: i32, ty: &ir::Ty) -> Result<(), String> {
        if indirect(ty)? {
            self.bytes(&[0x48, 0x89, 0xc6, 0x48, 0x8d, 0xbd]);
            self.i32(offset);
            self.copy_pointer_value(value_size(ty)?);
            self.address(offset);
        } else {
            if let ir::Ty::Int(integer) = ty {
                self.normalize_integer(*integer);
            }
            self.bytes(&[0x48, 0x89, 0x85]);
            self.i32(offset);
        }
        Ok(())
    }
    // rax holds the value/snapshot pointer and rdi the destination address.
    fn store_address_value(&mut self, ty: &ir::Ty) -> Result<(), String> {
        if indirect(ty)? {
            self.bytes(&[0x48, 0x89, 0xc6]);
            self.copy_pointer_value(value_size(ty)?);
            self.bytes(&[0x48, 0x89, 0xf8]);
        } else {
            if let ir::Ty::Int(integer) = ty {
                self.normalize_integer(*integer);
            }
            self.bytes(&[0x48, 0x89, 0x07]);
        }
        Ok(())
    }
    // Shared slice carrier helpers. Convention: a slice operand leaves
    // the ADDRESS of a two-word carrier snapshot in rax (like aggregates), but
    // unlike aggregates that address never crosses a call boundary: calls push
    // both words. Every store/load/call site must test the exact shared-slice
    // shape BEFORE the existing `indirect`/scalar branches.
    fn load_word(&mut self, register: u8, base: u8, displacement: u8) {
        // mov reg64, [base + disp8]
        self.bytes(&[0x48, 0x8b, 0x40 | (register << 3) | base, displacement]);
    }
    /// ArrayToSlice: rax holds the array reference's data pointer (the
    /// existing dummy address for empty arrays). Write data and the validated
    /// constant length into the frame carrier, leaving its address in rax.
    fn array_to_slice(
        &mut self,
        layout: SliceLayout,
        offset: i32,
        len: usize,
    ) -> Result<(), String> {
        let data = offset
            .checked_add(i32::from(layout.data_offset))
            .ok_or("slice carrier offset too large")?;
        let len_at = offset
            .checked_add(i32::from(layout.len_offset))
            .ok_or("slice carrier offset too large")?;
        self.bytes(&[0x48, 0x89, 0x85]);
        self.i32(data);
        self.word(u64::try_from(len).map_err(|_| "slice length exceeds usize word")?);
        self.bytes(&[0x48, 0x89, 0x85]);
        self.i32(len_at);
        self.address(offset);
        Ok(())
    }
    /// rax = source carrier address. Copy both words into the frame carrier
    /// at `offset` and leave its address in rax. Clobbers rcx.
    #[cfg_attr(not(test), allow(dead_code))]
    fn copy_slice_to_frame(&mut self, layout: SliceLayout, offset: i32) -> Result<(), String> {
        for field in [layout.data_offset, layout.len_offset] {
            self.load_word(1, 0, field);
            self.bytes(&[0x48, 0x89, 0x8d]);
            self.i32(
                offset
                    .checked_add(i32::from(field))
                    .ok_or("slice carrier offset too large")?,
            );
        }
        self.address(offset);
        Ok(())
    }
    /// rax = source carrier address, rdi = destination carrier address.
    /// Copy both words and leave the destination in rax. Clobbers rcx.
    #[cfg_attr(not(test), allow(dead_code))]
    fn copy_slice_to_address(&mut self, layout: SliceLayout) {
        for field in [layout.data_offset, layout.len_offset] {
            self.load_word(1, 0, field);
            self.bytes(&[0x48, 0x89, 0x4f, field]); // mov [rdi+disp8], rcx
        }
        self.bytes(&[0x48, 0x89, 0xf8]);
    }
    /// rax = carrier address. Push data then len, so the callee slot offset
    /// from `argument_slot_offsets` addresses len and data is at offset+8.
    fn push_slice_argument(&mut self, layout: SliceLayout) {
        self.bytes(&[0xff, 0x70, layout.data_offset]);
        self.bytes(&[0xff, 0x70, layout.len_offset]);
    }
    /// Callee prologue: copy a two-word argument at rbp+`argument` (len) and
    /// rbp+`argument`+8 (data) into the local carrier at rbp+`local`.
    fn store_slice_parameter(
        &mut self,
        layout: SliceLayout,
        argument: i32,
        local: i32,
    ) -> Result<(), String> {
        let word = i32::from(layout.word_bytes);
        let pushed_data = argument
            .checked_add(word)
            .ok_or("argument offset too large")?;
        for (from, field) in [
            (pushed_data, layout.data_offset),
            (argument, layout.len_offset),
        ] {
            self.bytes(&[0x48, 0x8b, 0x8d]);
            self.i32(from);
            self.bytes(&[0x48, 0x89, 0x8d]);
            self.i32(
                local
                    .checked_add(i32::from(field))
                    .ok_or("slice carrier offset too large")?,
            );
        }
        Ok(())
    }
    /// SliceLen: rax = carrier address, result rax = runtime len word. Never
    /// derived from the data pointer or an origin table.
    fn slice_len(&mut self, layout: SliceLayout) {
        self.load_word(0, 0, layout.len_offset);
    }
    /// SliceIndex step 1: rax = carrier address. Snapshot rsi = data and
    /// rdx = len before the index is evaluated. The index evaluation must
    /// preserve rsi and rdx (current operand/place loads clobber only rax).
    fn snapshot_slice(&mut self, layout: SliceLayout) {
        self.load_word(6, 0, layout.data_offset);
        self.load_word(2, 0, layout.len_offset);
    }
    /// SliceIndex step 2: rax = full 64-bit unsigned index, rsi = data,
    /// rdx = len. Trap (exit 101) on index >= len BEFORE scaling, address
    /// formation or load. Check full descriptor extent and element offset
    /// using unsigned multiplication, then load the exact element value.
    fn checked_slice_index_load(
        &mut self,
        layout: SliceLayout,
        element: &ir::Ty,
    ) -> Result<(), String> {
        let stride = i32::try_from(layout.element_stride)
            .map_err(|_| "slice stride exceeds private layout limit")?;
        if stride != value_size(element)? {
            return Err("slice stride disagrees with exact array element layout".into());
        }
        self.bytes(&[0x48, 0x39, 0xd0]); // cmp rax, rdx
        self.trap_if(0x83); // jae: unsigned, so high-bit indices trap
        self.bytes(&[0x49, 0x89, 0xc0]); // r8 = index snapshot
        self.bytes(&[0x48, 0x89, 0xd0, 0xb9]); // rax = len, ecx = stride
        self.i32(stride);
        self.bytes(&[0x48, 0xf7, 0xe1]); // unsigned rdx:rax = len * stride
        self.trap_if(0x82); // jc: multiplication overflow
        self.bytes(&[0x48, 0x01, 0xf0]); // rax = data + extent
        self.trap_if(0x82); // jc: pointer extent overflow
        self.bytes(&[0x4c, 0x89, 0xc0]); // rax = original index snapshot
        self.bytes(&[0x48, 0xf7, 0xe1]); // unsigned rdx:rax = index * stride
        self.trap_if(0x82);
        self.bytes(&[0x48, 0x01, 0xf0]); // rax += data
        self.trap_if(0x82); // jc: pointer element overflow
        if indirect(element)? {
            // No borrowed element address escapes as a value: materialize an
            // independent full wide/aggregate snapshot before assignment.
            self.store_value(self.aggregate_scratch, element)?;
        } else {
            self.bytes(&[0x48, 0x8b, 0x00]);
            if let ir::Ty::Int(integer) = element {
                self.normalize_integer(*integer);
            }
        }
        Ok(())
    }
    fn trap_block(&mut self) -> Result<(), String> {
        let trap = self.position();
        self.bytes(&[
            0xbf, 101, 0, 0, 0, 0xb8, 60, 0, 0, 0, 0x0f, 0x05, 0x0f, 0x0b,
        ]);
        for at in std::mem::take(&mut self.traps) {
            self.patch(at, trap)?;
        }
        Ok(())
    }
    /// rax = address of an exactly shared `&[T]` whole-local carrier.
    fn slice_operand(&mut self, operand: &cfg::Operand) -> Result<(), String> {
        let (cfg::Operand::Copy(place) | cfg::Operand::Move(place)) = operand else {
            return Err("slice operand requires a local carrier".into());
        };
        if !place.projection.is_empty()
            || self
                .function()
                .place_ty(place)?
                .shared_slice_element()
                .is_none()
        {
            return Err("slice operand requires a whole shared &[T] local".into());
        }
        self.load_place(place)
    }
    fn store_place(&mut self, place: &ir::Place) -> Result<(), String> {
        // Rust assignment evaluates the complete owning RHS first. Resolve a
        // dereference's CURRENT carrier only after that evaluation/snapshot.
        self.bytes(&[0x49, 0x89, 0xc1]); // r9 = RHS
        let ty = self.place_address(place)?;
        self.bytes(&[0x48, 0x89, 0xc7, 0x4c, 0x89, 0xc8]);
        self.store_address_value(ty)
    }
    fn operand(&mut self, operand: &cfg::Operand) -> Result<(), String> {
        match operand {
            cfg::Operand::Constant(cfg::Constant::Int(value)) => {
                value.validate()?;
                if value.ty.bits() == 128 {
                    self.wide_constant(*value);
                } else {
                    self.word(value.bits as u64);
                    self.normalize_integer(value.ty);
                }
            }
            cfg::Operand::Constant(cfg::Constant::Float(value)) => {
                value.validate()?;
                self.word(value.bits);
            }
            cfg::Operand::Constant(value) => self.immediate(match value {
                cfg::Constant::Int(_) | cfg::Constant::Float(_) => {
                    unreachable!("numeric constants use typed full-width immediates")
                }
                cfg::Constant::Bool(value) => i32::from(*value),
                cfg::Constant::Unit => 0,
                cfg::Constant::Enum { ty, variant } => {
                    if ty.has_payload() {
                        return Err("payload enum constants require constructor storage".into());
                    }
                    i32::try_from(*variant)
                        .map_err(|_| "enum tag exceeds private scalar representation")?
                }
            }),
            cfg::Operand::Copy(place) | cfg::Operand::Move(place) => {
                self.load_place(place)?;
            }
        }
        Ok(())
    }
    // Physical transport only. Ownership permissions were established by 109,
    // so returning a non-Copy aggregate must not invent a Copy operand.
    fn load_place(&mut self, place: &ir::Place) -> Result<(), String> {
        let ty = self.place_address(place)?;
        if !indirect(ty)? {
            self.bytes(&[0x48, 0x8b, 0x00]);
            if let ir::Ty::Int(integer) = ty {
                self.normalize_integer(*integer);
            }
        }
        Ok(())
    }
    fn index_address(
        &mut self,
        array: &ir::Place,
        index: &cfg::Operand,
    ) -> Result<&'a ir::Ty, String> {
        let ty = self.function().place_ty(array)?;
        let ir::Ty::Array { element, len } = ty else {
            return Err("runtime index requires array storage".into());
        };
        if self.function().operand_ty(index)? != ir::Ty::Int(ir::IntTy::Usize) {
            return Err("runtime index requires genuine usize operand".into());
        }
        self.operand(index)?;
        // Read the full unsigned index and reject it before scaling or forming
        // any element address. The array length cap is not an index range cap.
        self.bytes(&[0x48, 0x3d]);
        self.i32(i32::try_from(*len).map_err(|_| "array length exceeds backend limit")?);
        self.trap_if(0x83); // jae
        if *len != 0 {
            self.bytes(&[0x48, 0x69, 0xc0]); // rax *= physical element stride
            self.i32(value_size(element)?);
            self.bytes(&[0x49, 0x89, 0xc0]); // r8 = checked byte offset
            self.place_address(array)?;
            self.bytes(&[0x4c, 0x01, 0xc0]);
        }
        Ok(element)
    }
    fn enum_variant(
        &mut self,
        ty: &ir::EnumTy,
        variant: usize,
        fields: &[cfg::Operand],
    ) -> Result<(), String> {
        let definition = ty
            .def()
            .variants
            .get(variant)
            .ok_or("invalid enum variant")?;
        if fields.len() != definition.fields.len() {
            return Err("enum constructor field count mismatch".into());
        }
        let tag = i32::try_from(variant).map_err(|_| "enum tag exceeds private representation")?;
        if !ty.has_payload() {
            self.immediate(tag);
            return Ok(());
        }
        // Inactive payload storage is still physically transported by this
        // private ABI. Initialize the entire backing, including tail padding,
        // before any operand is stored. CFG lowering snapshots operands before
        // construction, so this scratch cannot alias a constructor input.
        let scratch = self.aggregate_scratch;
        let size = value_size(&ir::Ty::Enum(ty.clone()))?;
        self.immediate(0);
        for field in (0..size).step_by(8) {
            self.bytes(&[0x48, 0x89, 0x85]);
            self.i32(
                scratch
                    .checked_add(field)
                    .ok_or("enum scratch offset too large")?,
            );
        }
        self.immediate(tag);
        self.store_value(scratch, &ir::Ty::Int(ir::IntTy::I32))?;
        let mut offset = scratch
            .checked_add(8)
            .ok_or("enum payload offset too large")?;
        for (operand, field) in fields.iter().zip(definition.fields.iter()) {
            self.operand(operand)?;
            self.store_value(offset, &field.ty)?;
            offset = offset
                .checked_add(value_size(&field.ty)?)
                .ok_or("enum payload offset too large")?;
        }
        self.address(scratch);
        Ok(())
    }
    fn repeat(&mut self, ty: &ir::Ty, value: &cfg::Operand) -> Result<(), String> {
        let ir::Ty::Array { element, len } = ty else {
            return Err("repeat requires array storage".into());
        };
        value_size(ty)?;
        // The CFG operand is a stable source snapshot. Transport it once even
        // for a zero-length repeat, without inventing another semantic Copy.
        self.operand(value)?;
        if *len == 0 {
            self.immediate(0);
            self.store_value(self.aggregate_scratch, &ir::Ty::Unit)?;
        } else {
            self.bytes(&[0x48, 0x8d, 0xbd]); // rdi = backing destination
            self.i32(self.aggregate_scratch);
            if indirect(element)? {
                self.bytes(&[0x48, 0x89, 0xc2, 0x41, 0xb8]); // rdx = source, r8d = len
                self.i32(i32::try_from(*len).map_err(|_| "repeat length exceeds limit")?);
                let again = self.position();
                self.bytes(&[0x48, 0x89, 0xd6, 0xb9]); // rsi = source, ecx = stride words
                self.i32(value_size(element)? / 8);
                self.bytes(&[0xfc, 0xf3, 0x48, 0xa5]); // cld; rep movsq
                self.bytes(&[0x49, 0xff, 0xc8]); // dec r8
                let more = self.jump(&[0x0f, 0x85]);
                self.patch(more, again)?;
            } else {
                self.bytes(&[0xb9]); // ecx = len; rax is the scalar snapshot
                self.i32(i32::try_from(*len).map_err(|_| "repeat length exceeds limit")?);
                self.bytes(&[0xfc, 0xf3, 0x48, 0xab]); // cld; rep stosq
            }
        }
        self.address(self.aggregate_scratch);
        Ok(())
    }
    fn binary_integer(&mut self, op: BinaryOp, ty: ir::IntTy) -> Result<(), String> {
        if ty.bits() > 64 {
            return Err("wide integer cannot use the word arithmetic kernel".into());
        }
        match op {
            BinaryOp::Shl | BinaryOp::Shr => {
                // RCX is the independently typed, normalized FULL count.
                // Unsigned comparison also rejects sign-extended negatives.
                self.bytes(&[0x48, 0x83, 0xf9, ty.bits() as u8]);
                self.trap_if(0x83); // jae, before hardware masks CL
                self.bytes(&[
                    0x48,
                    0xd3,
                    match op {
                        BinaryOp::Shl => 0xe0,
                        _ if ty.is_signed() => 0xf8,
                        _ => 0xe8,
                    },
                ]);
                // Lost left-shift value bits are truncation, not overflow.
                self.normalize_integer(ty);
            }
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                self.bytes(&[
                    0x48,
                    match op {
                        BinaryOp::BitAnd => 0x21,
                        BinaryOp::BitOr => 0x09,
                        _ => 0x31,
                    },
                    0xc8,
                ]);
                self.normalize_integer(ty);
            }
            BinaryOp::Add | BinaryOp::Sub => {
                self.bytes(&[
                    0x48,
                    if matches!(op, BinaryOp::Add) {
                        0x01
                    } else {
                        0x29
                    },
                    0xc8,
                ]);
                self.trap_if(if ty.is_signed() { 0x80 } else { 0x82 });
                self.check_integer_range(ty);
            }
            BinaryOp::Mul => {
                if ty.is_signed() {
                    self.bytes(&[0x48, 0x0f, 0xaf, 0xc1]);
                    self.trap_if(0x80);
                } else {
                    self.bytes(&[0x48, 0xf7, 0xe1]); // rdx:rax = unsigned product
                    self.trap_if(0x82);
                }
                self.check_integer_range(ty);
            }
            BinaryOp::Div | BinaryOp::Rem => {
                self.bytes(&[0x48, 0x85, 0xc9]);
                self.trap_if(0x84);
                if ty.is_signed() {
                    // Guard MIN/-1 at the declared width for BOTH operations,
                    // before hardware division can fault or rem can hide overflow.
                    let minimum = u64::MAX << (ty.bits() - 1);
                    self.bytes(&[0x48, 0xba]);
                    self.bytes(&minimum.to_le_bytes());
                    self.bytes(&[0x48, 0x39, 0xd0]);
                    let ordinary = self.jump(&[0x0f, 0x85]);
                    self.bytes(&[0x48, 0x83, 0xf9, 0xff]);
                    self.trap_if(0x84);
                    self.patch(ordinary, self.position())?;
                    self.bytes(&[0x48, 0x99, 0x48, 0xf7, 0xf9]); // cqo; idiv rcx
                } else {
                    self.bytes(&[0x31, 0xd2, 0x48, 0xf7, 0xf1]);
                }
                if matches!(op, BinaryOp::Rem) {
                    self.bytes(&[0x48, 0x89, 0xd0]);
                }
                self.check_integer_range(ty);
            }
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => {
                let cc = match op {
                    BinaryOp::Eq => 0x94,
                    BinaryOp::Ne => 0x95,
                    BinaryOp::Lt if ty.is_signed() => 0x9c,
                    BinaryOp::Le if ty.is_signed() => 0x9e,
                    BinaryOp::Gt if ty.is_signed() => 0x9f,
                    BinaryOp::Ge if ty.is_signed() => 0x9d,
                    BinaryOp::Lt => 0x92,
                    BinaryOp::Le => 0x96,
                    BinaryOp::Gt => 0x97,
                    _ => 0x93,
                };
                self.bytes(&[0x48, 0x39, 0xc8, 0x0f, cc, 0xc0, 0x0f, 0xb6, 0xc0]);
            }
            BinaryOp::And | BinaryOp::Or => {
                return Err("integer logical operator is unsupported".into());
            }
        }
        Ok(())
    }
    // RAX = left raw IEEE bits, RCX = right. F32 lives in the low 32 bits with
    // zero upper bits; every float producer below preserves that invariant.
    fn float_operands(&mut self, ty: ir::FloatTy) {
        match ty {
            ir::FloatTy::F64 => self.bytes(&[
                0x66, 0x48, 0x0f, 0x6e, 0xc0, // movq xmm0, rax
                0x66, 0x48, 0x0f, 0x6e, 0xc9, // movq xmm1, rcx
            ]),
            ir::FloatTy::F32 => self.bytes(&[
                0x66, 0x0f, 0x6e, 0xc0, // movd xmm0, eax
                0x66, 0x0f, 0x6e, 0xc9, // movd xmm1, ecx
            ]),
        }
    }
    fn float_result(&mut self, ty: ir::FloatTy) {
        match ty {
            ir::FloatTy::F64 => self.bytes(&[0x66, 0x48, 0x0f, 0x7e, 0xc0]), // movq rax, xmm0
            ir::FloatTy::F32 => self.bytes(&[0x66, 0x0f, 0x7e, 0xc0]),       // movd eax, xmm0
        }
    }
    fn binary_float(&mut self, op: BinaryOp, ty: ir::FloatTy) -> Result<(), String> {
        let prefix = match ty {
            ir::FloatTy::F64 => 0xf2,
            ir::FloatTy::F32 => 0xf3,
        };
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                self.float_operands(ty);
                let opcode = match op {
                    BinaryOp::Add => 0x58,
                    BinaryOp::Sub => 0x5c,
                    BinaryOp::Mul => 0x59,
                    _ => 0x5e,
                };
                self.bytes(&[prefix, 0x0f, opcode, 0xc1]);
                self.float_result(ty);
            }
            BinaryOp::Rem => {
                // Rust float `%` is the exact truncated remainder (fmod).
                // x87 FPREM computes it exactly, iterated until C2 clears.
                // The two pushes sit below every frame slot and are popped here.
                let memory = match ty {
                    ir::FloatTy::F64 => 0xdd,
                    ir::FloatTy::F32 => 0xd9,
                };
                self.bytes(&[0x50, 0x51]); // push rax (a); push rcx (b)
                self.bytes(&[memory, 0x04, 0x24]); // fld [rsp] = b
                self.bytes(&[memory, 0x44, 0x24, 0x08]); // fld [rsp+8] = a
                // fprem; fnstsw ax; test ah, 4; jnz back to fprem
                self.bytes(&[0xd9, 0xf8, 0xdf, 0xe0, 0xf6, 0xc4, 0x04, 0x75, 0xf7]);
                self.bytes(&[0xdd, 0xd9]); // fstp st(1): drop divisor
                self.bytes(&[memory, 0x5c, 0x24, 0x08]); // fstp [rsp+8]
                self.bytes(&[0x59, 0x58]); // pop rcx; pop rax = remainder bits
                if ty == ir::FloatTy::F32 {
                    self.bytes(&[0x89, 0xc0]); // mov eax, eax: clear upper half
                }
            }
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => {
                self.float_operands(ty);
                if ty == ir::FloatTy::F64 {
                    self.bytes(&[0x66]);
                }
                // ucomis xmm0,xmm1 (left vs right), or swapped for < and <=,
                // so only CF/ZF "above" tests are needed: unordered is false.
                let swapped = matches!(op, BinaryOp::Lt | BinaryOp::Le);
                self.bytes(&[0x0f, 0x2e, if swapped { 0xc8 } else { 0xc1 }]);
                match op {
                    // Unordered (NaN) sets ZF, PF and CF: only != is then true.
                    BinaryOp::Eq => self.bytes(&[0x0f, 0x94, 0xc0, 0x0f, 0x9b, 0xc1, 0x20, 0xc8]),
                    BinaryOp::Ne => self.bytes(&[0x0f, 0x95, 0xc0, 0x0f, 0x9a, 0xc1, 0x08, 0xc8]),
                    BinaryOp::Gt | BinaryOp::Lt => self.bytes(&[0x0f, 0x97, 0xc0]),
                    _ => self.bytes(&[0x0f, 0x93, 0xc0]),
                }
                self.bytes(&[0x0f, 0xb6, 0xc0]);
            }
            _ => return Err("float operator is unsupported".into()),
        }
        Ok(())
    }
    fn float_cast(&mut self, from: ir::NumTy, to: ir::NumTy) -> Result<(), String> {
        use ir::{FloatTy, NumTy};
        if matches!(from, NumTy::Int(integer) if integer.bits() == 128)
            || matches!(to, NumTy::Int(integer) if integer.bits() == 128)
        {
            self.snapshot_number(from, 0);
            return self.wide_float_cast(from, to);
        }
        match (from, to) {
            (NumTy::Float(FloatTy::F32), NumTy::Float(FloatTy::F32))
            | (NumTy::Float(FloatTy::F64), NumTy::Float(FloatTy::F64)) => {}
            (NumTy::Float(FloatTy::F32), NumTy::Float(FloatTy::F64)) => {
                self.bytes(&[0x66, 0x0f, 0x6e, 0xc0, 0xf3, 0x0f, 0x5a, 0xc0]);
                self.float_result(FloatTy::F64);
            }
            (NumTy::Float(FloatTy::F64), NumTy::Float(FloatTy::F32)) => {
                self.bytes(&[0x66, 0x48, 0x0f, 0x6e, 0xc0, 0xf2, 0x0f, 0x5a, 0xc0]);
                self.float_result(FloatTy::F32);
            }
            (NumTy::Int(integer), NumTy::Float(float)) => {
                // Transport already sign/zero-extended the source to 64 bits.
                let prefix = if float == FloatTy::F64 { 0xf2 } else { 0xf3 };
                if matches!(integer, ir::IntTy::U64 | ir::IntTy::Usize) {
                    self.bytes(&[0x48, 0x85, 0xc0]); // test rax, rax
                    let high = self.jump(&[0x0f, 0x88]); // js
                    self.bytes(&[prefix, 0x48, 0x0f, 0x2a, 0xc0]); // cvtsi2s? xmm0, rax
                    let done = self.jump(&[0xe9]);
                    self.patch(high, self.position())?;
                    // Halve with a sticky low bit so the single rounding is exact.
                    self.bytes(&[0x48, 0x89, 0xc1, 0x48, 0xd1, 0xe9, 0x83, 0xe0, 0x01]);
                    self.bytes(&[0x48, 0x09, 0xc1, prefix, 0x48, 0x0f, 0x2a, 0xc1]);
                    self.bytes(&[prefix, 0x0f, 0x58, 0xc0]); // double it back
                    self.patch(done, self.position())?;
                } else {
                    self.bytes(&[prefix, 0x48, 0x0f, 0x2a, 0xc0]);
                }
                self.float_result(float);
            }
            (NumTy::Float(float), NumTy::Int(integer)) => self.float_to_int(float, integer)?,
            (NumTy::Int(_), NumTy::Int(_)) => {
                return Err("float cast requires a float endpoint".into());
            }
        }
        Ok(())
    }
    // Saturating truncation: NaN -> 0, at or below MIN -> MIN, at or above
    // MAX+1 -> MAX. Bounds are exact powers of two compared in double
    // precision after an exact f32 widening, so no bound is itself rounded.
    fn float_to_int(&mut self, float: ir::FloatTy, integer: ir::IntTy) -> Result<(), String> {
        match float {
            ir::FloatTy::F64 => self.bytes(&[0x66, 0x48, 0x0f, 0x6e, 0xc0]),
            ir::FloatTy::F32 => {
                self.bytes(&[0x66, 0x0f, 0x6e, 0xc0, 0xf3, 0x0f, 0x5a, 0xc0]);
            }
        }
        let bits = integer.bits() as i32;
        let (minimum, maximum, lower, upper) = if integer.is_signed() {
            (
                u64::MAX << (bits - 1),
                (integer.mask() >> 1) as u64,
                -(2f64.powi(bits - 1)),
                2f64.powi(bits - 1),
            )
        } else {
            // Unsigned lower saturation: anything <= -1.0 maps to zero, as does
            // (-1, 0) via truncation. Use -1 as an exact bound.
            (0, integer.mask() as u64, -1.0, 2f64.powi(bits))
        };
        let mut exits = Vec::new();
        self.bytes(&[0x66, 0x0f, 0x2e, 0xc0]); // ucomisd xmm0, xmm0
        let nan = self.jump(&[0x0f, 0x8a]); // jp
        let bound = |e: &mut Self, value: f64| {
            e.bytes(&[0x48, 0xb9]);
            e.bytes(&value.to_bits().to_le_bytes());
            // movq xmm1, rcx; ucomisd xmm0, xmm1
            e.bytes(&[0x66, 0x48, 0x0f, 0x6e, 0xc9, 0x66, 0x0f, 0x2e, 0xc1]);
        };
        bound(self, lower);
        let low = self.jump(&[0x0f, 0x86]); // jbe: x <= lower bound
        bound(self, upper);
        let high = self.jump(&[0x0f, 0x83]); // jae: x >= upper bound
        if matches!(integer, ir::IntTy::U64 | ir::IntTy::Usize) {
            bound(self, 2f64.powi(63));
            let big = self.jump(&[0x0f, 0x83]);
            self.bytes(&[0xf2, 0x48, 0x0f, 0x2c, 0xc0]); // cvttsd2si rax, xmm0
            exits.push(self.jump(&[0xe9]));
            self.patch(big, self.position())?;
            self.bytes(&[0xf2, 0x0f, 0x5c, 0xc1]); // subsd xmm0, xmm1 (2^63)
            self.bytes(&[0xf2, 0x48, 0x0f, 0x2c, 0xc0]);
            self.bytes(&[0x48, 0x0f, 0xba, 0xf8, 0x3f]); // btc rax, 63
        } else {
            self.bytes(&[0xf2, 0x48, 0x0f, 0x2c, 0xc0]);
        }
        exits.push(self.jump(&[0xe9]));
        self.patch(nan, self.position())?;
        self.word(0);
        exits.push(self.jump(&[0xe9]));
        self.patch(low, self.position())?;
        self.word(minimum);
        exits.push(self.jump(&[0xe9]));
        self.patch(high, self.position())?;
        self.word(maximum);
        for exit in exits {
            self.patch(exit, self.position())?;
        }
        self.normalize_integer(integer);
        Ok(())
    }
    fn binary_noninteger(&mut self, op: BinaryOp) -> Result<(), String> {
        if matches!(op, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor) {
            // Exact CFG validation limits this dispatch to canonical Booleans.
            // The eager fullword operations retain 0/1 without integer checks.
            self.bytes(&[
                0x48,
                match op {
                    BinaryOp::BitAnd => 0x21,
                    BinaryOp::BitOr => 0x09,
                    _ => 0x31,
                },
                0xc8,
            ]);
            return Ok(());
        }
        let cc = match op {
            BinaryOp::Eq => 0x94,
            BinaryOp::Ne => 0x95,
            _ => return Err("noninteger binary operator is unsupported".into()),
        };
        self.bytes(&[0x48, 0x39, 0xc8, 0x0f, cc, 0xc0, 0x0f, 0xb6, 0xc0]);
        Ok(())
    }
    fn println(&mut self, text: &str) -> Result<(), String> {
        self.literal_output(text, ir::OutputStream::Stdout, true)
    }
    fn literal_output(
        &mut self,
        text: &str,
        stream: ir::OutputStream,
        newline: bool,
    ) -> Result<(), String> {
        let length = text
            .len()
            .checked_add(usize::from(newline))
            .ok_or("literal output storage overflow")?;
        if length == 0 {
            self.immediate(0);
            return Ok(());
        }
        let budget = self.concrete.facts().budget().clone();
        budget.charge_work(length)?;
        budget.retain_bytes(length)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| "literal output allocation failed")?;
        budget.retain_bytes(bytes.capacity().saturating_sub(length))?;
        bytes.extend_from_slice(text.as_bytes());
        if newline {
            bytes.push(b'\n');
        }
        if self.object.strings.len() == self.object.strings.capacity() {
            let old = self.object.strings.capacity();
            let entry = std::mem::size_of::<(usize, Vec<u8>)>();
            budget.retain_bytes(entry)?;
            self.object
                .strings
                .try_reserve_exact(1)
                .map_err(|_| "literal output relocation allocation failed")?;
            let extra = (self.object.strings.capacity() - old)
                .checked_sub(1)
                .and_then(|count| count.checked_mul(entry))
                .ok_or("literal output relocation storage overflow")?;
            budget.retain_bytes(extra)?;
        }
        self.bytes(&[0x48, 0xbe]);
        let at = self.position();
        self.bytes(&[0; 8]);
        let length = u64::try_from(bytes.len()).map_err(|_| "literal too large")?;
        self.object.strings.push((at, bytes));
        self.bytes(&[0x48, 0xba]);
        self.bytes(&length.to_le_bytes());
        let retry = self.position();
        let descriptor = match stream {
            ir::OutputStream::Stdout => 1,
            ir::OutputStream::Stderr => 2,
        };
        self.bytes(&[0xb8, 1, 0, 0, 0, 0xbf, descriptor, 0, 0, 0, 0x0f, 0x05]);
        self.bytes(&[0x48, 0x83, 0xf8, 0xfc]);
        let interrupted = self.jump(&[0x0f, 0x84]);
        self.patch(interrupted, retry)?;
        self.bytes(&[0x48, 0x85, 0xc0]);
        self.trap_if(0x8e);
        self.bytes(&[0x48, 0x01, 0xc6, 0x48, 0x29, 0xc2]);
        let more = self.jump(&[0x0f, 0x85]);
        self.patch(more, retry)?;
        self.immediate(0);
        Ok(())
    }
    /// A panic is terminal, not a value-producing print followed by a trap.
    /// Linux's kernel sigaction layout uses an eight-byte signal mask here.
    fn panic(&mut self, panic: &ir::PanicMessage) -> Result<(), String> {
        let length = panic
            .location
            .len()
            .checked_add(panic.message.len())
            .and_then(|length| length.checked_add(b"panicked at :\n\n".len()))
            .ok_or("panic diagnostic storage overflow")?;
        let budget = self.concrete.facts().budget().clone();
        budget.charge_work(length)?;
        budget.retain_bytes(length)?;
        let mut diagnostic = Vec::new();
        diagnostic
            .try_reserve_exact(length)
            .map_err(|_| "panic diagnostic allocation failed")?;
        budget.retain_bytes(diagnostic.capacity().saturating_sub(length))?;
        diagnostic.extend_from_slice(b"panicked at ");
        diagnostic.extend_from_slice(panic.location.as_bytes());
        diagnostic.extend_from_slice(b":\n");
        diagnostic.extend_from_slice(panic.message.as_bytes());
        diagnostic.push(b'\n');

        // Reserve a zeroed kernel sigaction (handler, flags, restorer, mask).
        self.bytes(&[0x48, 0x83, 0xec, 0x20, 0x31, 0xc0]);
        self.bytes(&[0x48, 0x89, 0x04, 0x24]);
        for offset in [8, 16, 24] {
            self.bytes(&[0x48, 0x89, 0x44, 0x24, offset]);
        }
        // Restore SIGABRT's default disposition, even when inherited ignored.
        self.bytes(&[0xb8, 13, 0, 0, 0, 0xbf, 6, 0, 0, 0]);
        self.bytes(&[
            0x48, 0x89, 0xe6, 0x31, 0xd2, 0x41, 0xba, 8, 0, 0, 0, 0x0f, 0x05,
        ]);
        self.bytes(&[0x48, 0x85, 0xc0]);
        let failed_sigabrt = self.jump(&[0x0f, 0x85]);
        // Ignore SIGPIPE so a closed stderr still reaches the abort operation.
        self.bytes(&[0x48, 0xc7, 0x04, 0x24, 1, 0, 0, 0]);
        self.bytes(&[0xb8, 13, 0, 0, 0, 0xbf, 13, 0, 0, 0]);
        self.bytes(&[
            0x48, 0x89, 0xe6, 0x31, 0xd2, 0x41, 0xba, 8, 0, 0, 0, 0x0f, 0x05,
        ]);
        self.bytes(&[0x48, 0x85, 0xc0]);
        let failed_sigpipe = self.jump(&[0x0f, 0x85]);

        self.bytes(&[0x48, 0xbe]);
        let relocation = self.position();
        self.bytes(&[0; 8]);
        let length = u64::try_from(diagnostic.len()).map_err(|_| "panic diagnostic too large")?;
        if self.object.strings.len() == self.object.strings.capacity() {
            let old = self.object.strings.capacity();
            let entry = std::mem::size_of::<(usize, Vec<u8>)>();
            budget.retain_bytes(entry)?;
            self.object
                .strings
                .try_reserve_exact(1)
                .map_err(|_| "panic relocation allocation failed")?;
            let added = self.object.strings.capacity() - old;
            let extra = added
                .checked_sub(1)
                .and_then(|count| count.checked_mul(entry))
                .ok_or("panic relocation storage overflow")?;
            budget.retain_bytes(extra)?;
        }
        self.object.strings.push((relocation, diagnostic));
        self.bytes(&[0x48, 0xba]);
        self.bytes(&length.to_le_bytes());
        let retry = self.position();
        self.bytes(&[0xb8, 1, 0, 0, 0, 0xbf, 2, 0, 0, 0, 0x0f, 0x05]);
        self.bytes(&[0x48, 0x83, 0xf8, 0xfc]);
        let interrupted = self.jump(&[0x0f, 0x84]);
        self.patch(interrupted, retry)?;
        self.bytes(&[0x48, 0x85, 0xc0]);
        let failed_write = self.jump(&[0x0f, 0x8e]);
        self.bytes(&[0x48, 0x01, 0xc6, 0x48, 0x29, 0xc2]);
        let more = self.jump(&[0x0f, 0x85]);
        self.patch(more, retry)?;
        let abort = self.position();
        self.patch(failed_write, abort)?;

        // This abort-only runtime does not produce core files. In particular a
        // piped core_pattern must not delay termination in an external service.
        // PR_SET_DUMPABLE must run after exec, which resets the inherited flag.
        self.bytes(&[0xb8, 157, 0, 0, 0, 0xbf, 4, 0, 0, 0, 0x31, 0xf6]);
        self.bytes(&[0x31, 0xd2, 0x45, 0x31, 0xd2, 0x45, 0x31, 0xc0, 0x0f, 0x05]);
        self.bytes(&[0x48, 0x85, 0xc0]);
        let failed_dumpable = self.jump(&[0x0f, 0x85]);
        // Unblock SIGABRT, then raise it in this process. This remains fatal
        // even with an inherited ignored disposition or blocked signal mask.
        self.bytes(&[0x48, 0xc7, 0x04, 0x24, 32, 0, 0, 0]);
        self.bytes(&[0xb8, 14, 0, 0, 0, 0xbf, 1, 0, 0, 0]);
        self.bytes(&[
            0x48, 0x89, 0xe6, 0x31, 0xd2, 0x41, 0xba, 8, 0, 0, 0, 0x0f, 0x05,
        ]);
        self.bytes(&[0x48, 0x85, 0xc0]);
        let failed_sigprocmask = self.jump(&[0x0f, 0x85]);
        self.bytes(&[0xb8, 39, 0, 0, 0, 0x0f, 0x05]);
        // A sandbox may reject getpid. Never reinterpret its negative errno
        // as kill(-1, ...), which would target unrelated permitted processes.
        self.bytes(&[0x48, 0x85, 0xc0]);
        let invalid_pid = self.jump(&[0x0f, 0x8e]);
        self.bytes(&[0x89, 0xc7, 0xbe, 6, 0, 0, 0, 0xb8, 62, 0, 0, 0, 0x0f, 0x05]);
        // A denied signal syscall must still never return to Rust code.
        let fallback = self.position();
        for failed in [
            failed_sigabrt,
            failed_sigpipe,
            failed_dumpable,
            failed_sigprocmask,
            invalid_pid,
        ] {
            self.patch(failed, fallback)?;
        }
        self.bytes(&[
            0xbf, 101, 0, 0, 0, 0xb8, 231, 0, 0, 0, 0x0f, 0x05, 0x0f, 0x0b,
        ]);
        Ok(())
    }
    fn rvalue(&mut self, value: &cfg::Rvalue) -> Result<(), String> {
        match value {
            cfg::Rvalue::Use(operand) => self.operand(operand)?,
            cfg::Rvalue::IntCast { value, to } => {
                // Exact CFG validation established an integer source. Extend
                // SOURCE signedness before destination-width normalization.
                // Wide endpoints use limbs, never the legacy word kernel.
                self.integer_cast(value, *to)?;
            }
            cfg::Rvalue::FloatCast { value, to } => {
                let from = ir::NumTy::of(&self.function().operand_ty(value)?)
                    .ok_or("float cast requires numeric source")?;
                self.operand(value)?;
                self.float_cast(from, *to)?;
            }
            cfg::Rvalue::Borrow(loan) => {
                // Immediate and two-phase loans both materialize the genuine
                // place address exactly once. Reservation/activation permission
                // belongs to the exact ConcreteCfg analysis, not runtime code.
                let site = self
                    .function()
                    .loan_sites
                    .get(loan.0)
                    .ok_or("invalid checked loan site")?;
                self.place_address(&site.place)?;
            }
            cfg::Rvalue::IndexCopy { array, index } => {
                let element = self.index_address(array, index)?;
                if !indirect(element)? {
                    self.bytes(&[0x48, 0x8b, 0x00]);
                    if let ir::Ty::Int(integer) = element {
                        self.normalize_integer(*integer);
                    }
                }
            }
            cfg::Rvalue::Repeat { ty, value } => self.repeat(ty, value)?,
            cfg::Rvalue::EnumVariant {
                ty,
                variant,
                fields,
            } => {
                self.enum_variant(ty, *variant, fields)?;
            }
            cfg::Rvalue::Discriminant(place) => {
                // Observing a tag is not a semantic Copy or Move of its enum.
                // The tag is always the first word, regardless of scalar or
                // indirect enum transport and any enclosing aggregate layout.
                self.place_address(place)?;
                self.bytes(&[0x8b, 0x00]);
            }
            cfg::Rvalue::Unary { op, value } => {
                let ty = self.function().operand_ty(value)?;
                if ty == ir::Ty::Never {
                    // The operand diverged, so this dead CFG operation has no
                    // runtime value to load and no concrete integer width.
                    return Ok(());
                }
                self.operand(value)?;
                if let ir::Ty::Int(integer) = ty
                    && integer.bits() == 128
                {
                    self.snapshot_number(ir::NumTy::Int(integer), 0);
                    return self.wide_unary(*op, integer);
                }
                match (op, ty) {
                    (UnaryOp::Neg, ir::Ty::Int(integer)) if integer.is_signed() => {
                        self.bytes(&[0x48, 0xf7, 0xd8]);
                        self.trap_if(0x80);
                        self.check_integer_range(integer);
                    }
                    (UnaryOp::Neg, ir::Ty::Float(ir::FloatTy::F64)) => {
                        self.bytes(&[0x48, 0x0f, 0xba, 0xf8, 0x3f]); // btc rax, 63
                    }
                    (UnaryOp::Neg, ir::Ty::Float(ir::FloatTy::F32)) => {
                        self.bytes(&[0x0f, 0xba, 0xf8, 0x1f]); // btc eax, 31
                    }
                    (UnaryOp::Not, ir::Ty::Int(integer)) => {
                        self.bytes(&[0x48, 0xf7, 0xd0]);
                        self.normalize_integer(integer);
                    }
                    (UnaryOp::Not, ir::Ty::Bool) => self.bytes(&[0x83, 0xf0, 0x01]),
                    _ => return Err("invalid checked unary operand type".into()),
                }
            }
            cfg::Rvalue::Binary { op, left, right } => {
                let ty = self.function().operand_ty(left)?;
                if ty == ir::Ty::Never || self.function().operand_ty(right)? == ir::Ty::Never {
                    // Operand effects already occurred in earlier CFG steps.
                    // Divergence leaves no value to load or arithmetic to run.
                    return Ok(());
                }
                if let (ir::Ty::Int(integer), ir::Ty::Int(rhs)) =
                    (&ty, self.function().operand_ty(right)?)
                    && (integer.bits() == 128 || rhs.bits() == 128)
                {
                    self.operand(left)?;
                    self.snapshot_number(ir::NumTy::Int(*integer), 0);
                    self.operand(right)?;
                    self.snapshot_number(ir::NumTy::Int(rhs), 16);
                    return self.wide_binary(*op, *integer, rhs);
                }
                self.operand(left)?;
                self.bytes(&[0x49, 0x89, 0xc0]); // r8 = normalized left, full width
                self.operand(right)?;
                self.bytes(&[0x48, 0x89, 0xc1, 0x4c, 0x89, 0xc0]);
                if let ir::Ty::Int(integer) = ty {
                    self.binary_integer(*op, integer)?;
                } else if let ir::Ty::Float(float) = ty {
                    self.binary_float(*op, float)?;
                } else {
                    self.binary_noninteger(*op)?;
                }
            }
            cfg::Rvalue::Aggregate { ty, fields } => {
                // Evaluate the whole aggregate into scratch, so destination
                // projections cannot overwrite a later Copy operand's source.
                if matches!(ty, ir::Ty::Array { len: 0, .. })
                    || matches!(ty, ir::Ty::Struct(structure) if structure.def().fields.is_empty())
                {
                    self.immediate(0);
                    self.store_value(self.aggregate_scratch, &ir::Ty::Unit)?;
                }
                for (index, operand) in fields.iter().enumerate() {
                    let (field, field_ty) = if matches!(ty, ir::Ty::Array { .. }) {
                        array_offset(ty, index)?
                    } else {
                        field_offset(ty, index)?
                    };
                    self.operand(operand)?;
                    let offset = self
                        .aggregate_scratch
                        .checked_add(field)
                        .ok_or("aggregate offset too large")?;
                    self.store_value(offset, field_ty)?;
                }
                self.address(self.aggregate_scratch);
            }
            cfg::Rvalue::Println(text) => self.println(text)?,
            cfg::Rvalue::Output(output) => {
                self.literal_output(&output.text, output.stream, output.newline)?;
            }
            cfg::Rvalue::ArrayToSlice { reference } => {
                // N comes only from the validated operand type. The operand is
                // the thin array pointer (the dummy word address for N == 0).
                let source = self.function().operand_ty(reference)?;
                let (element, len) = source
                    .shared_array_element_len()
                    .ok_or("array-to-slice requires exactly shared &[T; N]")?;
                let layout = shared_slice_layout(element)?;
                self.operand(reference)?;
                self.array_to_slice(layout, self.aggregate_scratch, len)?;
            }
            cfg::Rvalue::SliceLen { slice } => {
                self.slice_operand(slice)?;
                self.slice_len(SHARED_BYTE_SLICE_LAYOUT);
            }
            cfg::Rvalue::SliceIndex { slice, index } => {
                if self.function().operand_ty(index)? != ir::Ty::Int(ir::IntTy::Usize) {
                    return Err("slice index requires genuine usize operand".into());
                }
                let slice_ty = self.function().operand_ty(slice)?;
                let element = slice_ty
                    .shared_slice_element()
                    .ok_or("slice index requires exactly shared &[T]")?;
                let layout = shared_slice_layout(element)?;
                self.slice_operand(slice)?;
                self.snapshot_slice(layout);
                // Index transport clobbers only rax: a constant immediate or a
                // whole-local usize load, preserving the rsi/rdx snapshot.
                match index {
                    cfg::Operand::Constant(_) => {}
                    cfg::Operand::Copy(place) if place.projection.is_empty() => {}
                    _ => return Err("slice index requires a whole-local or constant usize".into()),
                }
                self.operand(index)?;
                self.checked_slice_index_load(layout, element)?;
            }
        }
        Ok(())
    }
    fn statement(&mut self, statement: &cfg::Statement) -> Result<(), String> {
        match statement {
            // Admitted values have fixed storage and no destructors this slice.
            cfg::Statement::StorageLive(_)
            | cfg::Statement::StorageDead(_)
            | cfg::Statement::Inspect(_) => {}
            cfg::Statement::IndexInspect { array, index } => {
                self.index_address(array, index)?;
            }
            cfg::Statement::IndexAssign {
                array,
                index,
                value,
            } => {
                let element = self.index_address(array, index)?;
                self.bytes(&[0x49, 0x89, 0xc1]); // r9 = checked destination
                self.operand(value)?;
                self.bytes(&[0x4c, 0x89, 0xcf]);
                self.store_address_value(element)?;
            }
            cfg::Statement::IndexAssignOp {
                array,
                index,
                value,
                op,
            } => {
                let element = self.index_address(array, index)?;
                if let (ir::Ty::Int(integer), ir::Ty::Int(rhs)) =
                    (element, self.function().operand_ty(value)?)
                    && (integer.bits() == 128 || rhs.bits() == 128)
                {
                    // Preserve the selected address across a kernel that may
                    // clobber every caller-saved register. Never recompute it.
                    self.bytes(&[0x50]);
                    if integer.bits() <= 64 {
                        self.bytes(&[0x48, 0x8b, 0x00]);
                        self.normalize_integer(*integer);
                    }
                    self.snapshot_number(ir::NumTy::Int(*integer), 0);
                    self.operand(value)?;
                    self.snapshot_number(ir::NumTy::Int(rhs), 16);
                    self.wide_binary(*op, *integer, rhs)?;
                    self.bytes(&[0x5f]); // pop rdi: exact selected destination
                    self.store_address_value(element)?;
                    return Ok(());
                }
                // RHS/index effects are already owning CFG snapshots. Keep the
                // one selected address across every closed scalar kernel,
                // including multiplication/division's RDX and shifts' RCX.
                self.bytes(&[0x49, 0x89, 0xc1]); // r9 = checked destination
                self.operand(value)?;
                self.bytes(&[0x48, 0x89, 0xc1, 0x49, 0x8b, 0x01]); // rhs RCX, old [r9] RAX
                match element {
                    ir::Ty::Int(integer) => {
                        self.normalize_integer(*integer);
                        self.binary_integer(*op, *integer)?;
                    }
                    ir::Ty::Bool => self.binary_noninteger(*op)?,
                    ir::Ty::Float(float) => self.binary_float(*op, *float)?,
                    _ => return Err("indexed compound update requires scalar operands".into()),
                }
                self.bytes(&[0x4c, 0x89, 0xcf]); // same checked destination
                self.store_address_value(element)?;
            }
            cfg::Statement::Assign { destination, value } => {
                self.rvalue(value)?;
                self.store_place(destination)?;
            }
        }
        Ok(())
    }
    fn terminator(&mut self, terminator: &cfg::Terminator) -> Result<(), String> {
        match terminator {
            cfg::Terminator::Goto(target) => self.edge(&[0xe9], *target),
            cfg::Terminator::SwitchVariant {
                place,
                variant,
                then_block,
                else_block,
            } => {
                // Raw discriminant inspection grants no semantic Copy of the
                // enum. The true edge's selected-variant proof is established
                // by CFG validation and ownership analysis, not by codegen.
                let ty = self.function().place_ty(place)?;
                let ir::Ty::Enum(enum_ty) = ty else {
                    return Err("variant switch requires enum storage".into());
                };
                if variant.owner != enum_ty.def().id
                    || variant.index >= enum_ty.def().variants.len()
                {
                    return Err("variant switch identity mismatch".into());
                }
                if enum_ty.def().variants.len() == 1 {
                    // A singleton's variant is known from its type even when
                    // its storage is uninitialized or moved. Analysis permits
                    // this observation without a tag read, so codegen must too.
                    self.edge(&[0xe9], *then_block);
                    return Ok(());
                }
                self.place_address(place)?;
                self.bytes(&[0x8b, 0x00]);
                self.bytes(&[0x3d]);
                self.i32(
                    i32::try_from(variant.index)
                        .map_err(|_| "enum tag exceeds private representation")?,
                );
                self.edge(&[0x0f, 0x84], *then_block);
                self.edge(&[0xe9], *else_block);
            }
            cfg::Terminator::Switch {
                condition,
                then_block,
                else_block,
            } => {
                self.operand(condition)?;
                self.bytes(&[0x85, 0xc0]);
                self.edge(&[0x0f, 0x85], *then_block);
                self.edge(&[0xe9], *else_block);
            }
            cfg::Terminator::Call {
                target,
                signature,
                args,
                destination,
                next,
                ..
            } => {
                let ir::Callee::Direct(target) = target else {
                    return Err("unresolved trait dispatch has no native ABI".into());
                };
                if args.len() > MAX_ARGS {
                    return Err(format!("at most {MAX_ARGS} arguments are supported"));
                }
                if args.len() != signature.params.len() {
                    return Err("call argument count does not match signature".into());
                }
                let words = parameter_words(&signature.params)?;
                let aggregate = result_indirect(&signature.result)?;
                let cleanup = call_cleanup_bytes(&words, aggregate)?;
                // Left-to-right evaluation. A two-word shared slice
                // operand leaves its carrier address in rax; both words are
                // pushed (data, then len), never the carrier pointer itself.
                let mut wide_argument = 0;
                for ((operand, words), param) in args.iter().zip(&words).zip(&signature.params) {
                    match words {
                        1 if wide_integer(param) => {
                            // A materialized constant lives in shared result
                            // scratch. Give each argument a stable distinct
                            // snapshot until this call returns.
                            self.operand(operand)?;
                            let slot = self.wide_scratch + WIDE_SCRATCH_BYTES + wide_argument * 16;
                            self.store_value(slot, param)?;
                            self.address(slot);
                            self.bytes(&[0x50]);
                            wide_argument += 1;
                        }
                        1 => {
                            self.operand(operand)?;
                            self.bytes(&[0x50]);
                        }
                        2 if param.shared_slice_element().is_some()
                            && self
                                .function()
                                .operand_ty(operand)?
                                .shared_slice_element()
                                .is_some() =>
                        {
                            self.slice_operand(operand)?;
                            self.push_slice_argument(SHARED_BYTE_SLICE_LAYOUT);
                        }
                        _ => return Err("invalid multi-word argument transport".into()),
                    }
                }
                if aggregate {
                    self.place_address(destination)?;
                    self.bytes(&[0x50]);
                }
                let at = self.jump(&[0xe8]);
                self.object.calls.push((at, (self.resolve)(target)?));
                if cleanup != 0 {
                    self.bytes(&[0x48, 0x81, 0xc4]);
                    self.i32(cleanup);
                }
                if !aggregate {
                    self.store_place(destination)?;
                }
                self.edge(&[0xe9], *next);
            }
            cfg::Terminator::Return => {
                self.load_place(&ir::Place {
                    local: self.function().return_local,
                    projection: Vec::new(),
                })?;
                let at = self.jump(&[0xe9]);
                self.returns.push(at);
            }
            cfg::Terminator::Unreachable => self.bytes(&[0x0f, 0x0b]),
            cfg::Terminator::Panic(panic) => self.panic(panic)?,
        }
        Ok(())
    }
}

/// Native entry point for an exact closed CFG with successful concrete analysis.
/// Only the private successful-analysis capability authorizes native layout.
/// Universal template proofs, unsealed CFGs and caller-supplied analysis reports
/// are not native capabilities. Graph callers retain this fact immutably, while
/// standalone checked-body callers should use `emit_checked_function`.
/// Reference calls additionally require whole-program noescape and exact callee
/// signature evidence. Reference returns require the exact canonical RefContract
/// and successful read-phase return-provenance/region verification as part of
/// that analysis. Mutable returns additionally consume the unique handle with
/// Move. The pointer load below is physical transport, not semantic Copy.
/// Projected returns require exact external-root plus typed suffix membership
/// and candidate-to-actual-origin correlation, never layout offsets in summaries.
/// Local analysis alone cannot establish the whole-program call contract.
/// Every call must already be a canonical Direct target with discharged
/// predicates. This backend never selects trait implementations or resolves
/// obligations, including for unreachable metadata.
/// Every local and lowering temporary has stable invocation-owned frame storage.
pub fn emit_cfg_function(
    concrete: &ownership::ConcreteCfg,
    resolve: &dyn Fn(&ir::CallTarget) -> Result<String, String>,
) -> Result<Object, String> {
    let function = concrete.function().as_ref();
    let validate_slice = |ty: &ir::Ty| -> Result<(), String> {
        if let Some(element) = ty.shared_slice_element() {
            // Universal reference-free admission is not a concrete layout
            // certificate. Recheck the fully substituted type and its exact
            // sealed facts before emitting any descriptor or memory access.
            ir::validate_shared_slice_element(
                element,
                ir::FactsCtx::Validated(concrete.facts()),
                concrete.facts().budget(),
            )?;
            shared_slice_layout(element)?;
        }
        Ok(())
    };
    for local in &function.locals {
        concrete.facts().budget().charge_work(1)?;
        validate_slice(&local.ty)?;
    }
    for block in &function.blocks {
        concrete.facts().budget().charge_work(1)?;
        if let cfg::Terminator::Call { signature, .. } = &block.terminator.value {
            for param in &signature.params {
                concrete.facts().budget().charge_work(1)?;
                validate_slice(param)?;
            }
        }
    }
    if function.params.len() > MAX_ARGS {
        return Err(format!("at most {MAX_ARGS} parameters are supported"));
    }
    let param_tys = function
        .params
        .iter()
        .map(|local| {
            function
                .locals
                .get(local.0)
                .map(|local| local.ty.clone())
                .ok_or_else(|| "invalid CFG parameter local".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let param_words = parameter_words(&param_tys)?;
    let mut needs_wide = function.locals.iter().any(|local| wide_integer(&local.ty));
    let mut max_wide_arguments = 0_i32;
    for block in &function.blocks {
        let mut inspect = |operand: &cfg::Operand| {
            // ConcreteCfg already proves Copy/Move permissions using its
            // sealed canonical facts. This is physical layout classification,
            // not a second proof with the weaker FactsCtx::Concrete fallback.
            needs_wide |= match operand {
                cfg::Operand::Copy(place) | cfg::Operand::Move(place) => {
                    wide_integer(function.place_ty(place)?)
                }
                cfg::Operand::Constant(_) => wide_integer(&function.operand_ty(operand)?),
            };
            Ok(())
        };
        for statement in &block.statements {
            statement.value.visit_operands(&mut inspect)?;
        }
        block.terminator.value.visit_operands(&mut inspect)?;
        // A cast may write straight into a wide projected destination while
        // its only operand is narrow. Do not rely on a lowering temporary.
        for statement in &block.statements {
            if let cfg::Statement::Assign { destination, .. } = &statement.value {
                needs_wide |= wide_integer(function.place_ty(destination)?);
            }
        }
        if let cfg::Terminator::Call { signature, .. } = &block.terminator.value {
            let count = signature
                .params
                .iter()
                .filter(|ty| wide_integer(ty))
                .count();
            max_wide_arguments = max_wide_arguments
                .max(i32::try_from(count).map_err(|_| "wide argument scratch count overflow")?);
        }
    }
    let mut frame_bytes = 0;
    let locals = function
        .locals
        .iter()
        .map(|local| allocate_value(&mut frame_bytes, &local.ty))
        .collect::<Result<Vec<_>, _>>()?;
    let mut scratch_size = 0;
    for block in &function.blocks {
        for statement in &block.statements {
            if let cfg::Statement::Assign { value, .. } = &statement.value {
                let size = match value {
                    cfg::Rvalue::Aggregate { ty, .. } | cfg::Rvalue::Repeat { ty, .. } => {
                        value_size(ty)?
                    }
                    cfg::Rvalue::EnumVariant { ty, .. } => value_size(&ir::Ty::Enum(ty.clone()))?,
                    cfg::Rvalue::ArrayToSlice { .. } => {
                        2 * i32::from(SHARED_BYTE_SLICE_LAYOUT.word_bytes)
                    }
                    cfg::Rvalue::SliceIndex { slice, .. } => {
                        let slice_ty = function.operand_ty(slice)?;
                        let element = slice_ty
                            .shared_slice_element()
                            .ok_or("slice index requires exactly shared &[T]")?;
                        if indirect(element)? {
                            value_size(element)?
                        } else {
                            0
                        }
                    }
                    _ => 0,
                };
                scratch_size = scratch_size.max(size);
            }
        }
    }
    frame_bytes = frame_bytes
        .checked_add(scratch_size)
        .filter(|size| *size <= MAX_FRAME_BYTES)
        .ok_or("function exceeds private frame limit")?;
    let aggregate_scratch = -frame_bytes;
    if needs_wide {
        let wide_bytes = max_wide_arguments
            .checked_mul(16)
            .and_then(|bytes| bytes.checked_add(WIDE_SCRATCH_BYTES))
            .ok_or("wide argument scratch exceeds private frame limit")?;
        frame_bytes = frame_bytes
            .checked_add(wide_bytes)
            .filter(|size| *size <= MAX_FRAME_BYTES)
            .ok_or("wide arithmetic scratch exceeds private frame limit")?;
    }
    let wide_scratch = -frame_bytes;
    let aggregate_result = result_indirect(&function.result)?;
    let result_size = value_size(&function.result)?;
    let param_offsets = argument_slot_offsets(&param_words, aggregate_result)?;
    let mut emitter = Emitter {
        object: Object {
            code: Vec::new(),
            calls: Vec::new(),
            strings: Vec::new(),
        },
        concrete,
        resolve,
        locals,
        aggregate_scratch,
        wide_scratch,
        returns: Vec::new(),
        traps: Vec::new(),
        edges: Vec::new(),
    };
    emitter.bytes(&[0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec]);
    emitter.i32(frame_bytes);
    for ((local, offset), words) in function.params.iter().zip(&param_offsets).zip(&param_words) {
        let local_ty = &function.locals[local.0].ty;
        if *words == 2 && local_ty.shared_slice_element().is_some() {
            // len at offset, data at offset+8, into the local carrier.
            emitter.store_slice_parameter(
                SHARED_BYTE_SLICE_LAYOUT,
                *offset,
                emitter.local_offset(*local)?,
            )?;
            continue;
        }
        if *words != 1 {
            return Err("invalid multi-word parameter transport".into());
        }
        emitter.bytes(&[0x48, 0x8b, 0x85]);
        emitter.i32(*offset);
        emitter.store_value(emitter.local_offset(*local)?, &function.locals[local.0].ty)?;
    }
    emitter.edge(&[0xe9], function.entry);
    let mut labels = Vec::with_capacity(function.blocks.len());
    for block in &function.blocks {
        labels.push(emitter.position());
        for statement in &block.statements {
            emitter.statement(&statement.value)?;
        }
        emitter.terminator(&block.terminator.value)?;
    }
    for (at, target) in std::mem::take(&mut emitter.edges) {
        emitter.patch(at, *labels.get(target.0).ok_or("invalid CFG block target")?)?;
    }
    let epilogue = emitter.position();
    if aggregate_result {
        emitter.bytes(&[0x48, 0x89, 0xc6, 0x48, 0x8b, 0xbd]);
        emitter.i32(16);
        emitter.copy_pointer_value(result_size);
        emitter.bytes(&[0x48, 0x89, 0xf8]);
    }
    emitter.bytes(&[0xc9, 0xc3]);
    for at in std::mem::take(&mut emitter.returns) {
        emitter.patch(at, epilogue)?;
    }
    emitter.trap_block()?;
    Ok(emitter.object)
}

fn seal_concrete_cfg(
    function: cfg::Function,
    facts: std::sync::Arc<super::traits::ValidatedFacts>,
) -> Result<ownership::ConcreteCfg, String> {
    let name = function.name.clone();
    let closed = cfg::close(std::sync::Arc::new(function), facts.clone(), facts.budget())?;
    ownership::analyze_closed(closed).map_err(|analysis| {
        format!(
            "{name}: initialization/move analysis (complete={}): {:?}",
            analysis.complete, analysis.diagnostics
        )
    })
}

/// Standalone call-free checked-body entry point. The caller supplies exact
/// closed instance facts from the validated world, including entry obligations.
/// Close all metadata and bind concrete analysis to that exact immutable CFG
/// before requesting layout. Calls require the common sealed instance plan,
/// available through `emit`.
pub fn emit_checked_function(
    function: &ir::CheckedFunction,
    facts: std::sync::Arc<super::traits::ValidatedFacts>,
) -> Result<Object, String> {
    let lowered = cfg::lower_with_facts(function, &facts, facts.budget())?;
    if lowered.blocks.iter().any(|block| {
        matches!(&block.terminator.value, cfg::Terminator::Call { signature, .. }
            if signature.params.iter().any(|ty| matches!(ty, ir::Ty::Ref { .. }))
                || matches!(signature.result, ir::Ty::Ref { .. }))
    }) {
        return Err(
            "reference calls require whole-program noescape and callee-signature evidence".into(),
        );
    }
    let has_calls = lowered
        .blocks
        .iter()
        .any(|block| matches!(block.terminator.value, cfg::Terminator::Call { .. }));
    let concrete = seal_concrete_cfg(lowered, facts)?;
    if has_calls {
        return Err("unsupported standalone call target requires an instance plan".into());
    }
    emit_cfg_function(&concrete, &|_| {
        Err("unsupported standalone call target requires an instance plan".into())
    })
}

fn put16(bytes: &mut [u8], at: usize, n: u16) {
    bytes[at..at + 2].copy_from_slice(&n.to_le_bytes());
}
fn put32(bytes: &mut [u8], at: usize, n: u32) {
    bytes[at..at + 4].copy_from_slice(&n.to_le_bytes());
}
fn put64(bytes: &mut [u8], at: usize, n: u64) {
    bytes[at..at + 8].copy_from_slice(&n.to_le_bytes());
}
fn page_align(n: usize) -> Result<usize, String> {
    n.checked_add(PAGE - 1)
        .map(|n| n & !(PAGE - 1))
        .ok_or("ELF too large".into())
}
fn segment(bytes: &mut [u8], index: usize, kind: u32, flags: u32, offset: usize, size: usize) {
    let at = 64 + index * 56;
    put32(bytes, at, kind);
    put32(bytes, at + 4, flags);
    put64(bytes, at + 8, offset as u64);
    put64(bytes, at + 16, BASE + offset as u64);
    put64(bytes, at + 24, BASE + offset as u64);
    put64(bytes, at + 32, size as u64);
    put64(bytes, at + 40, size as u64);
    put64(bytes, at + 48, PAGE as u64);
}

/// Link private objects into a fixed-address ET_EXEC with separate R headers,
/// RX code and R literals. GNU_STACK explicitly requests a non-executable stack.
pub fn link(functions: &[(String, Object)]) -> Result<Vec<u8>, String> {
    let mut code = vec![
        0xe8, 0, 0, 0, 0, 0x31, 0xff, 0xb8, 60, 0, 0, 0, 0x0f, 0x05, 0x0f, 0x0b,
    ];
    let mut symbols = HashMap::new();
    for (name, object) in functions {
        if symbols.insert(name.as_str(), code.len()).is_some() {
            return Err(format!("duplicate function `{name}`"));
        }
        code.extend_from_slice(&object.code);
    }
    let main = *symbols.get("main").ok_or("missing main function")?;
    let main_displacement =
        i32::try_from(main as i128 - 5).map_err(|_| "main outside rel32 range")?;
    code[1..5].copy_from_slice(&main_displacement.to_le_bytes());
    let data_offset = page_align(PAGE.checked_add(code.len()).ok_or("ELF too large")?)?;
    let mut data = Vec::new();
    for (name, object) in functions {
        let start = symbols[name.as_str()];
        for (at, callee) in &object.calls {
            if at.checked_add(4).is_none_or(|end| end > object.code.len()) {
                return Err("invalid call relocation".into());
            }
            let target = *symbols
                .get(callee.as_str())
                .ok_or_else(|| format!("unknown function `{callee}`"))?;
            let delta = i32::try_from(target as i128 - (start + at + 4) as i128)
                .map_err(|_| "call outside rel32 range")?;
            code[start + at..start + at + 4].copy_from_slice(&delta.to_le_bytes());
        }
        for (at, literal) in &object.strings {
            if at.checked_add(8).is_none_or(|end| end > object.code.len()) {
                return Err("invalid string relocation".into());
            }
            let address = BASE
                .checked_add(data_offset as u64)
                .and_then(|a| a.checked_add(data.len() as u64))
                .ok_or("ELF address overflow")?;
            code[start + at..start + at + 8].copy_from_slice(&address.to_le_bytes());
            data.extend_from_slice(literal);
        }
    }
    let length = data_offset.checked_add(data.len()).ok_or("ELF too large")?;
    let mut elf = vec![0; length];
    elf[..16].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    put16(&mut elf, 16, 2);
    put16(&mut elf, 18, 62);
    put32(&mut elf, 20, 1);
    put64(&mut elf, 24, BASE + PAGE as u64);
    put64(&mut elf, 32, 64);
    put16(&mut elf, 52, 64);
    put16(&mut elf, 54, 56);
    put16(&mut elf, 56, 4);
    segment(&mut elf, 0, 1, 4, 0, PAGE);
    segment(&mut elf, 1, 1, 5, PAGE, code.len());
    segment(&mut elf, 2, 1, 4, data_offset, data.len());
    segment(&mut elf, 3, 0x6474e551, 6, 0, 0);
    elf[PAGE..PAGE + code.len()].copy_from_slice(&code);
    elf[data_offset..].copy_from_slice(&data);
    Ok(elf)
}

/// Compatibility entry point for AST callers. Use the same declaration,
/// instance-plan and all-body gates as graph compilation without reparsing.
/// The provider calls only `emit_cfg_function`, so this does not recurse.
pub fn emit(program: &Program) -> Result<Vec<u8>, String> {
    super::compile_program_observed(program.clone(), 1, false)
        .map_err(|failure| failure.to_string())?
        .executable
        .ok_or_else(|| "common instance plan did not publish an executable".into())
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::super::frontend;
    use super::super::frontend::{Block, Expr, Function, Stmt, Type};
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::process::{Command, Output, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn source_fixture_facts(
        signatures: &frontend::Signatures,
        index: usize,
    ) -> Result<std::sync::Arc<super::super::traits::ValidatedFacts>, String> {
        let plan = signatures.plan()?;
        plan.instance_facts(&ir::CallTarget {
            declaration: plan.callable_decl(index)?,
            type_args: [].into(),
        })
    }

    fn emit_checked_fixture(
        function: &ir::CheckedFunction,
        signatures: &frontend::Signatures,
        index: usize,
    ) -> Result<Object, String> {
        super::emit_checked_function(function, source_fixture_facts(signatures, index)?)
    }

    fn fixture_modules() -> std::sync::Arc<super::super::module_identity::ModuleArena> {
        super::super::module_identity::ModuleArenaBuilder::for_catalog(
            super::super::token_primitives::ExpansionBudget::default(),
        )
        .unwrap()
        .freeze()
        .unwrap()
    }

    fn raw_fixture_facts(
        modules: std::sync::Arc<super::super::module_identity::ModuleArena>,
        function: &cfg::Function,
    ) -> Result<std::sync::Arc<super::super::traits::ValidatedFacts>, String> {
        if function
            .blocks
            .iter()
            .any(|block| matches!(block.terminator.value, cfg::Terminator::Call { .. }))
        {
            return Err("test CFG calls require a sealed instance plan".into());
        }
        let signature = ir::Signature {
            ref_contract: function.ref_contract.clone(),
            params: function
                .params
                .iter()
                .map(|local| {
                    function
                        .locals
                        .get(local.0)
                        .map(|local| local.ty.clone())
                        .ok_or_else(|| "invalid fixture parameter local".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?,
            result: function.result.clone(),
        };
        let types = function
            .locals
            .iter()
            .map(|local| local.ty.clone())
            .collect::<Vec<_>>();
        super::super::traits::test_support::facts_for_signature(
            modules,
            &signature,
            &types,
            &ir::PlanBudget::default(),
        )
        .map_err(|error| error.to_string())
    }

    fn emit_test_cfg(
        modules: std::sync::Arc<super::super::module_identity::ModuleArena>,
        function: &cfg::Function,
    ) -> Result<Object, String> {
        let concrete = seal_concrete_cfg(function.clone(), raw_fixture_facts(modules, function)?)?;
        emit_cfg_function(&concrete, &|_| {
            Err("test CFG calls require a sealed instance plan".into())
        })
    }

    fn unit_concrete_fixture() -> ownership::ConcreteCfg {
        let program = frontend::parse("fn main() {}").unwrap();
        let signatures = frontend::signature_table(&program).unwrap();
        let checked = frontend::type_function_with_signatures(&program, 0, &signatures).unwrap();
        let facts = source_fixture_facts(&signatures, 0).unwrap();
        seal_concrete_cfg(
            cfg::lower_with_facts(&checked, &facts, facts.budget()).unwrap(),
            facts,
        )
        .unwrap()
    }

    fn block(statements: Vec<Stmt>, tail: Option<Expr>) -> Block {
        Block {
            statements,
            tail: tail.map(Box::new),
        }
    }
    fn function(
        name: &str,
        params: Vec<(String, Type)>,
        result: Type,
        body: Block,
        builder: &mut frontend::ProgramBuilder,
    ) -> Function {
        Function {
            constness: crate::compiler::frontend::Constness::Runtime,
            visibility: ir::Visibility::Public,
            module: builder.module_root().unwrap(),
            lexical_scope: None,
            name: builder.path(&format!("crate::{name}")).unwrap(),
            regions: Default::default(),
            type_params: vec![],
            bounds: vec![],
            params: params
                .into_iter()
                .map(|(name, ty)| {
                    (
                        frontend::BindingName::Named(builder.ident(&name).unwrap()),
                        ty,
                    )
                })
                .collect(),
            result,
            body,
        }
    }
    fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
        Expr::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }
    fn var(name: &str, builder: &mut frontend::ProgramBuilder) -> Expr {
        Expr::Var(builder.path(name).unwrap())
    }
    fn call(name: &str, args: Vec<Expr>, builder: &mut frontend::ProgramBuilder) -> Expr {
        Expr::Call {
            // These fixtures construct the resolved AST directly. Local names
            // remain relative, but their root function callees are canonical.
            name: builder.path(&format!("crate::{name}")).unwrap(),
            args,
        }
    }
    fn crash() -> Expr {
        binary(BinaryOp::Div, Expr::i32(1), Expr::i32(0))
    }
    fn assertion(condition: Expr) -> Stmt {
        Stmt::Expr(Expr::If {
            condition: Box::new(condition),
            then_block: block(vec![], None),
            else_block: block(vec![Stmt::Expr(crash())], None),
        })
    }
    fn bool_equals(value: Expr, expected: bool) -> Stmt {
        assertion(binary(BinaryOp::Eq, value, Expr::Bool(expected)))
    }
    fn crash_bool() -> Expr {
        binary(BinaryOp::Eq, crash(), Expr::i32(0))
    }
    fn equals(value: Expr, expected: i32) -> Stmt {
        assertion(binary(BinaryOp::Eq, value, Expr::i32(expected)))
    }
    fn fixture_builder() -> frontend::ProgramBuilder {
        let budget = crate::compiler::token_primitives::ExpansionBudget::default();
        frontend::ProgramBuilder::new(std::path::Path::new("<backend-fixture>"), "", &budget)
            .unwrap()
    }

    fn owned_program(
        statements: Vec<Stmt>,
        mut others: Vec<Function>,
        mut builder: frontend::ProgramBuilder,
    ) -> Program {
        others.push(function(
            "main",
            vec![],
            Type::Unit,
            block(statements, None),
            &mut builder,
        ));
        builder.functions = others;
        builder.finish().unwrap()
    }

    fn program(statements: Vec<Stmt>, others: Vec<Function>) -> Program {
        assert!(
            others.is_empty(),
            "named fixtures require their original builder"
        );
        owned_program(statements, others, fixture_builder())
    }

    // Test-only selection of an already checked root declaration. Production
    // resolver and callable authority use canonical scoped semantic keys.
    fn authored_function(program: &Program, name: &str) -> usize {
        program
            .functions
            .iter()
            .position(|function| {
                function.name.anchor() == frontend::PathAnchor::Crate
                    && function.name.len() == 1
                    && program.names.spelling(function.name.segments()[0]).unwrap() == name
            })
            .unwrap()
    }
    fn executable(program: &Program) -> std::path::PathBuf {
        executable_bytes(&emit(program).unwrap())
    }
    fn executable_bytes(bytes: &[u8]) -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .filter(|path| path.is_dir())
            .unwrap_or_else(std::env::temp_dir);
        let directory = root.join(format!(
            "jr-native-backend-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let path = directory.join("program");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&path)
            .unwrap();
        file.write_all(bytes).unwrap();
        drop(file);
        path
    }
    fn run(program: &Program) -> Output {
        let path = executable(program);
        let output = execute(Command::new(&path).stdout(Stdio::piped()));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        output
    }

    fn execute(command: &mut Command) -> Output {
        command.stdin(Stdio::null()).stderr(Stdio::piped());
        // A concurrent test's fork can briefly inherit the writer's descriptor
        // before its CLOEXEC takes effect. Linux returns ETXTBSY in that window.
        for _ in 0..100 {
            match command.spawn() {
                Ok(mut child) => {
                    fn drain<R: Read + Send + 'static>(
                        reader: Option<R>,
                    ) -> std::thread::JoinHandle<Vec<u8>> {
                        std::thread::spawn(move || {
                            let mut output = Vec::new();
                            if let Some(mut reader) = reader {
                                reader.read_to_end(&mut output).unwrap();
                            }
                            output
                        })
                    }
                    let stdout = drain(child.stdout.take());
                    let stderr = drain(child.stderr.take());
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                    let (status, timed_out) = loop {
                        if let Some(status) = child.try_wait().unwrap() {
                            break (status, false);
                        }
                        if std::time::Instant::now() >= deadline {
                            child.kill().unwrap();
                            break (child.wait().unwrap(), true);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    };
                    let output = Output {
                        status,
                        stdout: stdout.join().unwrap(),
                        stderr: stderr.join().unwrap(),
                    };
                    assert!(!timed_out, "native executable timed out: {output:?}");
                    return output;
                }
                Err(error) if error.raw_os_error() == Some(26) => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(error) => panic!("cannot execute native test: {error}"),
            }
        }
        panic!("native executable remained busy after bounded retries");
    }

    #[test]
    fn panic_native_aborts_with_diagnostic_even_if_signal_is_ignored_or_blocked() {
        use std::os::unix::process::{CommandExt, ExitStatusExt};

        unsafe extern "C" {
            fn syscall(number: std::ffi::c_long, ...) -> std::ffi::c_long;
        }
        #[repr(C)]
        struct SignalAction {
            handler: usize,
            flags: usize,
            restorer: usize,
            mask: u64,
        }
        for (source, expected) in [
            ("fn main() {\n    panic!();\n}", "explicit panic"),
            (
                "fn main() {\n    panic!(\"diagnostic 🦀 {{braces}}\");\n    println!(\"after\");\n}",
                "diagnostic 🦀 {braces}",
            ),
        ] {
            let program = frontend::parse(source).unwrap();
            let path = executable(&program);
            for hostile in [false, true] {
                let mut command = Command::new(&path);
                command.stdout(Stdio::piped());
                // SAFETY: this runs only in the forked child and invokes Linux
                // x86_64 syscalls with fixed-size, stack-resident kernel ABI data.
                // No allocation, locking or mutation of the parent is performed.
                unsafe {
                    command.pre_exec(move || {
                        let core_limit = [0u64; 2];
                        if syscall(160, 4usize, core_limit.as_ptr()) != 0
                            || syscall(157, 4usize, 0usize, 0usize, 0usize, 0usize) != 0
                        {
                            return Err(std::io::Error::last_os_error());
                        }
                        if hostile {
                            let action = SignalAction {
                                handler: 1,
                                flags: 0,
                                restorer: 0,
                                mask: 0,
                            };
                            if syscall(13, 6usize, &action as *const SignalAction, 0usize, 8usize)
                                != 0
                            {
                                return Err(std::io::Error::last_os_error());
                            }
                            let mask = 1u64 << 5;
                            if syscall(14, 0usize, &mask as *const u64, 0usize, 8usize) != 0 {
                                return Err(std::io::Error::last_os_error());
                            }
                        }
                        Ok(())
                    });
                }
                let output = execute(&mut command);
                assert_eq!(output.status.signal(), Some(6), "{output:?}");
                assert!(output.stdout.is_empty(), "{output:?}");
                let diagnostic = String::from_utf8(output.stderr).unwrap();
                assert!(diagnostic.contains("panicked at "), "{diagnostic}");
                assert!(diagnostic.contains(":2:5:\n"), "{diagnostic}");
                assert!(
                    diagnostic.ends_with(&format!("{expected}\n")),
                    "{diagnostic}"
                );
            }
            let mut command = Command::new(&path);
            command.stdout(Stdio::piped());
            // SAFETY: only fixed Linux syscalls execute in the child. Closing
            // its descriptor 2 exercises the panic write-error path.
            unsafe {
                command.pre_exec(|| {
                    let core_limit = [0u64; 2];
                    if syscall(160, 4usize, core_limit.as_ptr()) != 0
                        || syscall(157, 4usize, 0usize, 0usize, 0usize, 0usize) != 0
                        || syscall(3, 2usize) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let output = execute(&mut command);
            assert_eq!(output.status.signal(), Some(6), "{output:?}");
            assert!(output.stdout.is_empty());
            assert!(output.stderr.is_empty());
            std::fs::remove_file(&path).unwrap();
            std::fs::remove_dir(path.parent().unwrap()).unwrap();
        }
    }

    fn assert_panic_setup_denied(number: u32, argument: u32, broken_pipe: bool) {
        use std::os::unix::process::{CommandExt, ExitStatusExt};
        unsafe extern "C" {
            fn syscall(number: i64, ...) -> i64;
        }
        #[repr(C)]
        struct Filter {
            code: u16,
            jt: u8,
            jf: u8,
            value: u32,
        }
        #[repr(C)]
        struct Program {
            len: u16,
            filter: *const Filter,
        }
        let path = executable(
            &frontend::parse("fn main(){panic!(\"denied setup\");println!(\"returned\");}")
                .unwrap(),
        );
        let mut command = Command::new(&path);
        // SAFETY: only fixed, stack-backed Linux syscalls execute after fork.
        // The pipe has no reader. Kill is trapped even if the runtime regresses.
        unsafe {
            command.pre_exec(move || {
                if broken_pipe {
                    let mut pipe = [0i32; 2];
                    if syscall(22, pipe.as_mut_ptr()) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if syscall(3, pipe[0]) != 0
                        || syscall(33, pipe[1], 2usize) < 0
                        || syscall(3, pipe[1]) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    // Override Rust's inherited ignored SIGPIPE before exec.
                    let action = [0u64; 4];
                    let mask = 1u64 << 12;
                    if syscall(13, 13usize, action.as_ptr(), 0usize, 8usize) != 0
                        || syscall(14, 1usize, &mask as *const u64, 0usize, 8usize) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                let filters = [
                    Filter {
                        code: 0x20,
                        jt: 0,
                        jf: 0,
                        value: 0,
                    },
                    Filter {
                        code: 0x15,
                        jt: 0,
                        jf: 1,
                        value: 62,
                    },
                    Filter {
                        code: 0x06,
                        jt: 0,
                        jf: 0,
                        value: 0x30000,
                    },
                    Filter {
                        code: 0x15,
                        jt: 0,
                        jf: 3,
                        value: number,
                    },
                    Filter {
                        code: 0x20,
                        jt: 0,
                        jf: 0,
                        value: 16,
                    },
                    Filter {
                        code: 0x15,
                        jt: 0,
                        jf: 1,
                        value: argument,
                    },
                    Filter {
                        code: 0x06,
                        jt: 0,
                        jf: 0,
                        value: 0x50001,
                    },
                    Filter {
                        code: 0x06,
                        jt: 0,
                        jf: 0,
                        value: 0x7fff0000,
                    },
                ];
                let program = Program {
                    len: filters.len() as u16,
                    filter: filters.as_ptr(),
                };
                let core_limit = [0u64; 2];
                if syscall(160, 4usize, core_limit.as_ptr()) != 0
                    || syscall(157, 38usize, 1usize, 0usize, 0usize, 0usize) != 0
                    || syscall(
                        157,
                        22usize,
                        2usize,
                        &program as *const Program,
                        0usize,
                        0usize,
                    ) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let output = execute(&mut command);
        assert_eq!(output.status.signal(), None, "{output:?}");
        assert_eq!(output.status.code(), Some(101), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        if number == 13 {
            assert!(output.stderr.is_empty(), "{output:?}");
        } else {
            assert!(output.stderr.ends_with(b"denied setup\n"), "{output:?}");
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn panic_denied_sigabrt_setup_exits_without_kill() {
        assert_panic_setup_denied(13, 6, false);
    }

    #[test]
    fn panic_denied_sigpipe_setup_exits_without_kill() {
        assert_panic_setup_denied(13, 13, false);
    }

    #[test]
    fn panic_denied_sigpipe_setup_with_broken_pipe_exits_before_write() {
        assert_panic_setup_denied(13, 13, true);
    }

    #[test]
    fn panic_denied_dumpable_setup_exits_without_kill() {
        assert_panic_setup_denied(157, 4, false);
    }

    #[test]
    fn panic_denied_sigprocmask_setup_exits_without_kill() {
        assert_panic_setup_denied(14, 1, false);
    }

    #[test]
    fn panic_denied_getpid_falls_back_without_attempting_kill() {
        use std::os::unix::process::{CommandExt, ExitStatusExt};
        unsafe extern "C" {
            fn syscall(number: i64, ...) -> i64;
        }
        #[repr(C)]
        struct Filter {
            code: u16,
            jt: u8,
            jf: u8,
            value: u32,
        }
        #[repr(C)]
        struct Program {
            len: u16,
            filter: *const Filter,
        }
        let path = executable(&frontend::parse("fn main(){panic!(\"denied pid\");}").unwrap());
        for errno in [0u32, 1] {
            let mut command = Command::new(&path);
            // SAFETY: only fixed, stack-backed Linux seccomp/prctl syscalls
            // execute in the child. Kill is trapped, never permitted, even if
            // the generated getpid failure branch regresses.
            unsafe {
                command.pre_exec(move || {
                    let filters = [
                        Filter {
                            code: 0x20,
                            jt: 0,
                            jf: 0,
                            value: 0,
                        },
                        Filter {
                            code: 0x15,
                            jt: 0,
                            jf: 1,
                            value: 39,
                        },
                        Filter {
                            code: 0x06,
                            jt: 0,
                            jf: 0,
                            value: 0x50000 | errno,
                        },
                        Filter {
                            code: 0x15,
                            jt: 0,
                            jf: 1,
                            value: 62,
                        },
                        Filter {
                            code: 0x06,
                            jt: 0,
                            jf: 0,
                            value: 0x30000,
                        },
                        Filter {
                            code: 0x06,
                            jt: 0,
                            jf: 0,
                            value: 0x7fff0000,
                        },
                    ];
                    let program = Program {
                        len: filters.len() as u16,
                        filter: filters.as_ptr(),
                    };
                    let core_limit = [0u64; 2];
                    if syscall(160, 4usize, core_limit.as_ptr()) != 0
                        || syscall(157, 4usize, 0usize, 0usize, 0usize, 0usize) != 0
                        || syscall(157, 38usize, 1usize, 0usize, 0usize, 0usize) != 0
                        || syscall(
                            157,
                            22usize,
                            2usize,
                            &program as *const Program,
                            0usize,
                            0usize,
                        ) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let output = execute(&mut command);
            assert_eq!(output.status.signal(), None, "{output:?}");
            assert_eq!(output.status.code(), Some(101), "{output:?}");
            assert!(output.stdout.is_empty());
            assert!(
                String::from_utf8(output.stderr)
                    .unwrap()
                    .ends_with("denied pid\n")
            );
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn wide_integer_private_layout_uses_indirect_scalar_storage() {
        for integer in [ir::IntTy::I128, ir::IntTy::U128] {
            let ty = ir::Ty::Int(integer);
            assert_eq!(value_size(&ty), Ok(16));
            assert_eq!(indirect(&ty), Ok(true));
            assert_eq!(result_indirect(&ty), Ok(true));
            assert_eq!(argument_words(&ty), Ok(1));
            let reference = ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: std::sync::Arc::new(ty.clone()),
            };
            assert_eq!(value_size(&reference), Ok(8));
            assert_eq!(indirect(&reference), Ok(false));
            let tuple =
                ir::Ty::Tuple(vec![ir::Ty::Int(ir::IntTy::U8), ty.clone(), ir::Ty::Bool].into());
            assert_eq!(value_size(&tuple), Ok(32));
            assert_eq!(field_offset(&tuple, 1).unwrap().0, 8);
            assert_eq!(field_offset(&tuple, 2).unwrap().0, 24);
            let array = ir::Ty::Array {
                element: std::sync::Arc::new(ty),
                len: 3,
            };
            assert_eq!(value_size(&array), Ok(48));
            assert_eq!(array_offset(&array, 2).unwrap().0, 32);
            let mut frame = 8;
            assert_eq!(allocate_value(&mut frame, &array), Ok(-56));
        }
    }

    #[test]
    fn wide_integer_private_transport_keeps_constants_fields_and_returns_distinct() {
        let program = frontend::parse(
            r#"
            struct Packet { before:u8, wide:u128, after:u64 }
            enum Choice { Empty, Full(u128) }
            fn id<T>(value:T)->T { value }
            fn pair(a:u128,b:u128)->bool {
                a==18446744073709551617u128 && b==36893488147419103235u128
            }
            fn read(value:&u128)->u128 { *value }
            fn write(value:&mut u128, next:u128) { *value=next; }
            fn main() {
                let a=id(18446744073709551617u128);
                let b=id(36893488147419103235u128);
                let packet=id(Packet { before:3u8,wide:a,after:5u64 });
                let mut values=[a;3];
                let index=1usize;
                values[index]=b;
                values[index]+=1u128;
                let mut value=0u128;
                write(&mut value,b);
                let signed=-1i8 as i128;
                let negative=-1i128 as u128;
                let chosen=match Choice::Full(b) {
                    Choice::Empty=>0u128,
                    Choice::Full(inner)=>inner
                };
                if pair(18446744073709551617u128,36893488147419103235u128)
                    && packet.before==3u8 && packet.wide==a && packet.after==5u64
                    && values[0usize]==a && values[index]==36893488147419103236u128
                    && values[2usize]==a && read(&value)==b && chosen==b
                    && signed==(-1i128) && negative==340282366920938463463374607431768211455u128
                    && (a as u64)==1u64 && (1u8<<1u128)==2u8
                { println!("wide transport ok"); }
            }
            "#,
        )
        .unwrap();
        let output = run(&program);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"wide transport ok\n");
    }

    #[test]
    fn wide_integer_referenced_array_compounds_keep_rhs_first_and_selected_cell() {
        let program = frontend::parse(
            r#"
            fn index()->usize { println!("index"); 1usize }
            fn count()->u128 { println!("rhs"); 64u128 }
            fn update(values:&mut [u128;2]) {
                let mut selected=0usize;
                values[selected] <<= { selected=1usize; println!("move rhs"); 64u128 };
                values[index()] >>= count();
            }
            fn signed(values:&mut [i128;2]) {
                let selected=1usize;
                values[selected] >>= 65u128;
            }
            fn narrow(values:&mut [i32;2]) {
                let selected=1usize;
                values[selected] >>= 1i128;
                values[selected] <<= 1u128;
            }
            fn main() {
                let mut values=[1u128,2u128];
                update(&mut values);
                let mut negative=[-8i128,-73786976294838206464i128];
                signed(&mut negative);
                let mut old=[-8i32,-8i32];
                narrow(&mut old);
                if values[0usize]==1u128 && values[1usize]==2u128
                    && negative[0usize]==(-8i128) && negative[1usize]==(-2i128)
                    && old[0usize]==(-8i32) && old[1usize]==(-8i32)
                { println!("referenced compound ok"); }
            }
            "#,
        )
        .unwrap();
        let output = run(&program);
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            output.stdout,
            b"move rhs\nrhs\nindex\nreferenced compound ok\n"
        );
    }

    #[test]
    fn wide_integer_referenced_array_compound_checks_bounds_before_cell_access() {
        let program = frontend::parse(
            r#"
            fn index()->usize { println!("index"); 2usize }
            fn count()->u128 { println!("rhs"); 1u128 }
            fn update(values:&mut [u128;2]) { values[index()] <<= count(); }
            fn main() { let mut values=[1u128,2u128]; update(&mut values); }
            "#,
        )
        .unwrap();
        let output = run(&program);
        assert_eq!(output.status.code(), Some(101), "{output:?}");
        assert_eq!(output.stdout, b"rhs\nindex\n");
    }

    #[test]
    fn wide_integer_scratch_planning_preserves_sealed_nominal_copy_facts() {
        let compilation = super::super::compile(
            r#"
            struct Wrap<T> { value:T }
            impl<T:Copy> Clone for Wrap<T> { fn clone(&self)->Self { *self } }
            impl<T:Copy> Copy for Wrap<T> {}
            fn twice<T:Copy>(value:T)->(T,T) { (value,value) }
            fn main() {
                let wide=twice::<Wrap<u128>>(Wrap { value:18446744073709551617u128 });
                let floats=twice::<Wrap<f64>>(Wrap { value:2.5f64 });
                if wide.0.value==18446744073709551617u128
                    && wide.1.value==18446744073709551617u128
                    && floats.0.value+floats.1.value==5.0f64
                { println!("sealed copy ok"); }
            }
            "#
            .into(),
            1,
            false,
        )
        .unwrap();
        let path = executable_bytes(&compilation.executable.unwrap());
        let output = execute(Command::new(&path).stdout(Stdio::piped()));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"sealed copy ok\n");
    }

    #[test]
    fn expanded_argument_words_preserve_single_word_abi_and_reserve_two_word_slots() {
        assert_eq!(
            SHARED_BYTE_SLICE_LAYOUT,
            SliceLayout {
                data_offset: 0,
                len_offset: 8,
                word_bytes: 8,
                element_stride: 8
            }
        );
        let u8_array = ir::Ty::Array {
            element: std::sync::Arc::new(ir::Ty::Int(ir::IntTy::U8)),
            len: 3,
        };
        for ty in [
            ir::Ty::Int(ir::IntTy::I32),
            ir::Ty::Int(ir::IntTy::Usize),
            ir::Ty::Bool,
            ir::Ty::Unit,
            ir::Ty::Never,
            u8_array.clone(),
            ir::Ty::Tuple(vec![ir::Ty::Bool, ir::Ty::Unit].into()),
        ] {
            assert_eq!(argument_words(&ty), Ok(1), "{ty:?}");
        }
        // Exactly the legacy one-word-per-parameter formula for every arity.
        for sret in [false, true] {
            for count in [0, 1, 2, 7, MAX_ARGS] {
                let words = vec![1; count];
                let offsets = argument_slot_offsets(&words, sret).unwrap();
                for (index, offset) in offsets.iter().enumerate() {
                    let legacy = 16 + usize::from(sret) * 8 + (count - index - 1) * 8;
                    assert_eq!(*offset as usize, legacy);
                }
                assert_eq!(
                    call_cleanup_bytes(&words, sret).unwrap() as usize,
                    (count + usize::from(sret)) * 8
                );
            }
        }
        // Frozen two-word contract: slot offset addresses len, data at +8.
        assert_eq!(
            argument_slot_offsets(&[1, 2, 1], true),
            Ok(vec![48, 32, 24])
        );
        assert_eq!(argument_slot_offsets(&[2], false), Ok(vec![16]));
        assert_eq!(call_cleanup_bytes(&[1, 2, 1], true), Ok(40));
        assert_eq!(
            call_cleanup_bytes(&[2; MAX_ARGS], true),
            Ok(8 * MAX_ARG_WORDS as i32)
        );
        assert!(call_cleanup_bytes(&[2; MAX_ARGS + 1], false).is_err());
        assert!(argument_slot_offsets(&[usize::MAX, 1], false).is_err());
        assert!(parameter_words(&vec![ir::Ty::Bool; MAX_ARGS + 1]).is_err());
        assert_eq!(parameter_words(&[ir::Ty::Bool, u8_array]), Ok(vec![1, 1]));
    }

    /// Run hand-emitted helper code as `main`, exiting with eax.
    fn run_slice_helpers(index: u64, len: usize) -> Option<i32> {
        let concrete = unit_concrete_fixture();
        let resolve = |_: &ir::CallTarget| -> Result<String, String> { Err("no calls".into()) };
        let mut e = Emitter {
            object: Object {
                code: Vec::new(),
                calls: Vec::new(),
                strings: Vec::new(),
            },
            concrete: &concrete,
            resolve: &resolve,
            locals: Vec::new(),
            aggregate_scratch: 0,
            wide_scratch: 0,
            returns: Vec::new(),
            traps: Vec::new(),
            edges: Vec::new(),
        };
        let layout = SHARED_BYTE_SLICE_LAYOUT;
        e.bytes(&[0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec]);
        e.i32(128);
        // Private u8 array leaves at rbp-128, -120, -112. 300 checks that the
        // loaded element is normalized to u8.
        for (offset, value) in [(-128, 7_u64), (-120, 300), (-112, 9)] {
            e.word(value);
            e.bytes(&[0x48, 0x89, 0x85]);
            e.i32(offset);
        }
        e.address(-128);
        e.array_to_slice(layout, -32, len).unwrap(); // carrier A
        e.copy_slice_to_frame(layout, -16).unwrap(); // A -> B
        // Two-word argument transport: after `sub rsp,128` two pushes land
        // len at rbp-144 and data at rbp-136, the callee's slot convention.
        e.push_slice_argument(layout);
        e.store_slice_parameter(layout, -144, -48).unwrap(); // -> C
        e.bytes(&[0x48, 0x83, 0xc4, 0x10]);
        e.bytes(&[0x48, 0x8d, 0xbd]);
        e.i32(-64);
        e.address(-48);
        e.copy_slice_to_address(layout); // C -> D, rax = &D
        e.bytes(&[0x50]);
        e.slice_len(layout);
        e.bytes(&[0x49, 0x89, 0xc2]); // r10 = len
        e.bytes(&[0x58]);
        e.snapshot_slice(layout);
        e.word(index);
        e.checked_slice_index_load(layout, &ir::Ty::Int(ir::IntTy::U8))
            .unwrap();
        e.bytes(&[0x4c, 0x01, 0xd0]); // rax += len
        e.bytes(&[0x89, 0xc7, 0xb8, 60, 0, 0, 0, 0x0f, 0x05]);
        e.trap_block().unwrap();
        let elf = link(&[("main".into(), e.object)]).unwrap();
        let path = executable_bytes(&elf);
        let output = execute(&mut Command::new(&path));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        output.status.code()
    }

    #[test]
    fn shared_byte_slice_helpers_preserve_both_words_and_trap_before_addressing() {
        assert_eq!(run_slice_helpers(0, 3), Some(7 + 3));
        assert_eq!(run_slice_helpers(1, 3), Some(44 + 3));
        assert_eq!(run_slice_helpers(2, 3), Some(9 + 3));
        // Length is the runtime carrier word, preserved across every copy.
        assert_eq!(run_slice_helpers(1, 2), Some(44 + 2));
        for (index, len) in [(3, 3), (2, 2), (0, 0), (u64::MAX, 3), (1 << 63, 3)] {
            assert_eq!(run_slice_helpers(index, len), Some(101), "{index} {len}");
        }
    }

    #[test]
    fn typed_slice_layout_uses_exact_array_stride_and_two_word_transport() {
        for element in [
            ir::Ty::Int(ir::IntTy::I8),
            ir::Ty::Int(ir::IntTy::U128),
            ir::Ty::Float(ir::FloatTy::F32),
            ir::Ty::Bool,
            ir::Ty::Tuple(vec![ir::Ty::Int(ir::IntTy::U128), ir::Ty::Bool].into()),
            ir::Ty::Array {
                element: std::sync::Arc::new(ir::Ty::Int(ir::IntTy::U64)),
                len: 40,
            },
            ir::Ty::Array {
                element: std::sync::Arc::new(ir::Ty::Bool),
                len: 0,
            },
        ] {
            let layout = shared_slice_layout(&element).unwrap();
            assert_eq!(layout.element_stride as i32, value_size(&element).unwrap());
            let slice = ir::Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: std::sync::Arc::new(ir::Ty::Slice {
                    element: std::sync::Arc::new(element.clone()),
                }),
            };
            assert_eq!(argument_words(&slice), Ok(2));
            assert_eq!(value_size(&slice), Ok(16));
            assert_eq!(indirect(&slice), Ok(true));
            assert!(result_indirect(&slice).is_err());
            assert!(!slice.shared_byte_slice());
            let array = ir::Ty::Array {
                element: std::sync::Arc::new(element),
                len: 2,
            };
            assert_eq!(
                array_offset(&array, 1).unwrap().0,
                layout.element_stride as i32
            );
        }
        let large = ir::Ty::Array {
            element: std::sync::Arc::new(ir::Ty::Int(ir::IntTy::U64)),
            len: 40,
        };
        assert_eq!(shared_slice_layout(&large).unwrap().element_stride, 320);
        let parameter = ir::Ty::Param(ir::ParamId {
            owner: ir::BinderId {
                declaration: ir::DeclId {
                    snapshot: ir::SnapshotId::allocate().unwrap(),
                    index: 0,
                },
            },
            index: 0,
        });
        assert!(shared_slice_layout(&parameter).is_err());
    }

    // Physical helper control only, not a substitute for sealed source/CFG
    // admission. All backing, frame and expected representations are bounded.
    fn run_typed_slice_helper(
        element: &ir::Ty,
        backing: &[u64],
        expected: &[u64],
        index: u64,
        len: u64,
        pointer: Option<u64>,
    ) -> Option<i32> {
        assert!(backing.len() <= 80 && expected.len() <= 40);
        let concrete = unit_concrete_fixture();
        let resolve = |_: &ir::CallTarget| -> Result<String, String> { Err("no calls".into()) };
        let mut e = Emitter {
            object: Object {
                code: Vec::new(),
                calls: Vec::new(),
                strings: Vec::new(),
            },
            concrete: &concrete,
            resolve: &resolve,
            locals: Vec::new(),
            aggregate_scratch: -2048,
            wide_scratch: 0,
            returns: Vec::new(),
            traps: Vec::new(),
            edges: Vec::new(),
        };
        e.bytes(&[0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec]);
        e.i32(2048);
        for (word, value) in backing.iter().enumerate() {
            e.word(*value);
            e.bytes(&[0x48, 0x89, 0x85]);
            e.i32(-1024 + i32::try_from(word).unwrap() * 8);
        }
        if let Some(pointer) = pointer {
            e.word(pointer);
        } else {
            e.address(-1024);
        }
        let layout = shared_slice_layout(element).unwrap();
        e.array_to_slice(layout, -32, usize::try_from(len).unwrap())
            .unwrap();
        e.snapshot_slice(layout);
        e.word(index);
        e.checked_slice_index_load(layout, element).unwrap();
        e.bytes(&[0x49, 0x89, 0xc1]); // r9 = independent result or result pointer
        // Destroy original backing after extraction. An aggregate alias would
        // now read zeros rather than the complete expected snapshot.
        for word in 0..backing.len() {
            e.immediate(0);
            e.bytes(&[0x48, 0x89, 0x85]);
            e.i32(-1024 + i32::try_from(word).unwrap() * 8);
        }
        if indirect(element).unwrap() {
            for (word, value) in expected.iter().enumerate() {
                e.word(*value);
                e.bytes(&[0x49, 0x39, 0x81]); // cmp [r9+disp32], rax
                e.i32(i32::try_from(word).unwrap() * 8);
                e.trap_if(0x85);
            }
        } else if let Some(value) = expected.first() {
            e.word(*value);
            e.bytes(&[0x49, 0x39, 0xc1]); // cmp r9, rax
            e.trap_if(0x85);
        }
        e.bytes(&[0xbf, 42, 0, 0, 0, 0xb8, 60, 0, 0, 0, 0x0f, 0x05]);
        e.trap_block().unwrap();
        let elf = link(&[("main".into(), e.object)]).unwrap();
        let path = executable_bytes(&elf);
        let output = execute(&mut Command::new(&path));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        output.status.code()
    }

    #[test]
    fn typed_slice_helpers_preserve_signed_float_bool_wide_and_aggregate_values() {
        assert_eq!(
            run_typed_slice_helper(
                &ir::Ty::Int(ir::IntTy::I8),
                &[249],
                &[(-7_i64) as u64],
                0,
                1,
                None
            ),
            Some(42)
        );
        assert_eq!(
            run_typed_slice_helper(
                &ir::Ty::Float(ir::FloatTy::F32),
                &[u64::from((-2.5_f32).to_bits())],
                &[u64::from((-2.5_f32).to_bits())],
                0,
                1,
                None
            ),
            Some(42)
        );
        assert_eq!(
            run_typed_slice_helper(
                &ir::Ty::Float(ir::FloatTy::F64),
                &[(-2.5_f64).to_bits()],
                &[(-2.5_f64).to_bits()],
                0,
                1,
                None
            ),
            Some(42)
        );
        assert_eq!(
            run_typed_slice_helper(&ir::Ty::Bool, &[1], &[1], 0, 1, None),
            Some(42)
        );
        assert_eq!(
            run_typed_slice_helper(
                &ir::Ty::Int(ir::IntTy::U128),
                &[1, 2, 3, 4],
                &[3, 4],
                1,
                2,
                None
            ),
            Some(42)
        );
        let tuple = ir::Ty::Tuple(vec![ir::Ty::Int(ir::IntTy::U128), ir::Ty::Bool].into());
        assert_eq!(
            run_typed_slice_helper(&tuple, &[3, 4, 1, 5, 6, 0], &[5, 6, 0], 1, 2, None),
            Some(42)
        );
        let large = ir::Ty::Array {
            element: std::sync::Arc::new(ir::Ty::Int(ir::IntTy::U64)),
            len: 40,
        };
        assert_eq!(
            run_typed_slice_helper(&large, &[7; 80], &[7; 40], 1, 2, None),
            Some(42)
        );
    }

    #[test]
    fn typed_slice_helpers_trap_bounds_extent_and_pointer_overflow_before_load() {
        let element = ir::Ty::Int(ir::IntTy::U128);
        for (index, len, pointer) in [
            (0, 0, Some(u64::MAX)),
            (1, 1, Some(u64::MAX)),
            (u64::MAX, 1, Some(u64::MAX)),
            (1 << 63, 2, Some(u64::MAX)),
            (0, (u64::MAX / 16) + 1, Some(0)),
            (1, 2, Some(u64::MAX - 15)),
        ] {
            assert_eq!(
                run_typed_slice_helper(&element, &[], &[], index, len, pointer),
                Some(101),
                "{index} {len} {pointer:?}"
            );
        }
    }

    #[test]
    fn fieldless_enum_tags_calls_returns_and_nonconsuming_matches() {
        let p = frontend::parse(
            r#"
            enum Color { Red, Green, Blue }
            fn identity(value: Color) -> Color { value }
            fn code(value: Color) -> i32 {
                let first = match value {
                    Color::Red => 11,
                    Color::Green => 22,
                    Color::Blue => 33,
                };
                let second = match value {
                    Color::Blue => 33,
                    Color::Red => 11,
                    Color::Green => 22,
                };
                if first != second { 1 / 0; }
                first
            }
            fn main() {
                if code(identity(Color::Red)) != 11 { 1 / 0; }
                if code(identity(Color::Green)) != 22 { 1 / 0; }
                if code(identity(Color::Blue)) != 33 { 1 / 0; }
                println!("enum-tags");
            }
            "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"enum-tags\n");
    }

    #[test]
    fn fieldless_enum_nested_projection_and_aggregate_transport() {
        let p = frontend::parse(
            r#"
            enum State { Off, On, Unknown }
            struct Packet { before: i32, state: State, after: i32 }
            fn relay(value: Packet) -> Packet { value }
            fn main() {
                let packet = relay(Packet { before: 17, state: State::On, after: 29 });
                let first = match packet.state {
                    State::Off => 0,
                    State::On => 1,
                    State::Unknown => 2,
                };
                if first != 1 { 1 / 0; }
                if packet.before != 17 { 1 / 0; }
                if packet.after != 29 { 1 / 0; }
                let state = packet.state;
                let second = match state { State::On => 7, _ => 9 };
                if second != 7 { 1 / 0; }
                let pair = (31, State::Unknown);
                let third = match pair.1 {
                    State::Unknown => 5,
                    _ => 6,
                };
                if pair.0 != 31 { 1 / 0; }
                if third != 5 { 1 / 0; }
                println!("enum-aggregate");
            }
            "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"enum-aggregate\n");
    }

    #[test]
    fn generic_native_instances_payload_owners_recursion_and_aggregate_abi() {
        let program = frontend::parse(
            r#"
            struct Wrap<T> { value:T }
            enum Maybe<T> { None, Some(T) }
            fn identity<T>(value:T)->T { value }
            fn forward<T>(value:T)->T { identity::<T>(value) }
            fn packed<T>(value:T)->Wrap<T> { Wrap::<T> { value } }
            fn unpack<T>(value:Wrap<T>)->T { value.value }
            fn recur<T>(value:T, count:i32)->T {
                if count == 0 { value } else { recur::<T>(value,count-1) }
            }
            fn even<T>(value:T, count:i32)->T {
                if count == 0 { value } else { odd::<T>(value,count-1) }
            }
            fn odd<T>(value:T, count:i32)->T {
                if count == 0 { value } else { even::<T>(value,count-1) }
            }
            fn main() {
                if identity::<i8>(-127i8) != -127i8 { 1/0; }
                if identity::<u64>(18446744073709551615u64) != 18446744073709551615u64 { 1/0; }
                let narrow=packed::<u8>(255u8);
                if unpack::<u8>(narrow) != 255u8 { 1/0; }
                let wide=packed::<u64>(18446744073709551615u64);
                let selected=Maybe::<Wrap<u64>>::Some(wide);
                let number=match selected {
                    Maybe::<Wrap<u64>>::None => 0u64,
                    Maybe::<Wrap<u64>>::Some(inner) => unpack::<u64>(inner),
                };
                if number != 18446744073709551615u64 { 1/0; }
                let tuple=forward::<(u64,bool)>((18446744073709551615u64,true));
                if tuple.0 != 18446744073709551615u64 { 1/0; }
                if !tuple.1 { 1/0; }
                let array=unpack::<[u64;9]>(packed::<[u64;9]>([18446744073709551615u64;9]));
                let array=recur::<[u64;9]>(array,3);
                let array=even::<[u64;9]>(array,4);
                if array[0] != 18446744073709551615u64 { 1/0; }
                if array[8] != 18446744073709551615u64 { 1/0; }
                println!("generic-native");
            }
            "#,
        )
        .unwrap();
        let output = run(&program);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"generic-native\n");
    }

    #[test]
    fn generic_reference_native_thin_abi_and_two_phase_receiver_order() {
        let program = frontend::parse(
            r#"
            fn shared<T>(value:&T)->&T { value }
            fn forward<T>(value:&T)->&T { shared::<T>(value) }
            fn unique<T>(value:&mut T)->&mut T { value }
            struct Large { words:[u64;9], value:u64 }
            impl Large {
                fn get(&self)->u64 { self.value }
                fn set(&mut self,value:u64) { self.value=value; }
                fn field(&self,other:&u64)->&u64 { &self.value }
                fn field_mut(&mut self)->&mut u64 { &mut self.value }
            }
            fn carrier(value:&mut Large)->&mut Large { println!("carrier"); value }
            fn argument()->u64 { println!("argument"); 1u64 }
            fn add(value:&mut u64,more:u64) { *value=*value+more; }
            fn main() {
                let mut big=Large { words:[18446744073709551615u64;9],value:7u64 };
                let first=forward::<Large>(&big);
                let copied=first;
                if first.words[8] != 18446744073709551615u64 { 1/0; }
                if copied.value != 7u64 { 1/0; }
                let changed=unique::<Large>(&mut big);
                changed.value=9u64;
                big.set(big.get()+argument());
                if big.get() != 10u64 { 1/0; }
                carrier(&mut big).set(argument());
                let mut count=0;
                while count < 2 { big.set(big.get()+1u64); count=count+1; }
                if big.get() != 3u64 { 1/0; }
                let output=big.field_mut();
                *output=9223372036854775808u64;
                let chosen:&u64;
                { let other=13u64; chosen=big.field(&other); }
                if *chosen != 9223372036854775808u64 { 1/0; }
                let mut scalar=5u64;
                let reference=&mut scalar;
                add(reference,*reference);
                if *reference != 10u64 { 1/0; }
                println!("generic-reference-native");
            }
            "#,
        )
        .unwrap();
        let serial = emit(&program).unwrap();
        let parallel = super::super::compile_program_observed(program, 4, false)
            .unwrap()
            .executable
            .unwrap();
        assert_eq!(serial, parallel, "borrowed ABI depends on worker order");
        let path = executable_bytes(&serial);
        let output = execute(Command::new(&path).stdout(Stdio::piped()));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            output.stdout,
            b"argument\ncarrier\nargument\ngeneric-reference-native\n"
        );
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn generic_reference_native_trait_refinement_preserves_actual_return_pointers() {
        let program = frontend::parse(
            r#"
            trait View {
                fn field(&self)->&u64;
                fn field_mut(&mut self)->&mut u64;
            }
            trait Pick { fn pick<'a>(known:&'a u64,whole:&'a Self)->&'a Self; }
            mod hidden {
                pub struct Data { pad:[u64;9],value:u64 }
                impl crate::View for Data {
                    fn field<'p>(&'p self)->&'p u64 { &self.value }
                    fn field_mut<'q>(&'q mut self)->&'q mut u64 { &mut self.value }
                }
                impl crate::Pick for Data {
                    fn pick<'a>(known:&'a u64,whole:&'a Self)->&'a Self { whole }
                }
                pub fn make()->Data {
                    Data { pad:[18446744073709551615u64;9],value:18446744073709551615u64 }
                }
            }
            struct Wrapper<T> { inner:T }
            impl<T:View> View for Wrapper<T> {
                fn field(&self)->&u64 { self.inner.field() }
                fn field_mut(&mut self)->&mut u64 { self.inner.field_mut() }
            }
            fn view<T:View>(value:&T)->&u64 { value.field() }
            fn view_mut<T:View>(value:&mut T)->&mut u64 { value.field_mut() }
            fn concrete(value:&hidden::Data)->&u64 { value.field() }
            fn pick<'a,T:Pick>(known:&'a u64,whole:&'a T)->&'a T { T::pick(known,whole) }
            fn choose<'a,'b,T,U>(left:&'a T,right:&'b U)->&'a T { left }
            fn main() {
                let mut wrapped=Wrapper::<hidden::Data> { inner:hidden::make() };
                if *view::<Wrapper<hidden::Data>>(&wrapped) != 18446744073709551615u64 { 1/0; }
                *view_mut::<Wrapper<hidden::Data>>(&mut wrapped)=9223372036854775808u64;
                if *concrete(&wrapped.inner) != 9223372036854775808u64 { 1/0; }
                let known=1u64;
                let selected=pick::<hidden::Data>(&known,&wrapped.inner);
                if *selected.field() != 9223372036854775808u64 { 1/0; }
                let outer=17u64;
                let chosen:&u64;
                { let short=19u64; chosen=choose::<u64,u64>(&outer,&short); }
                if *chosen != 17u64 { 1/0; }
                println!("generic-reference-trait-native");
            }
            "#,
        )
        .unwrap();
        let output = run(&program);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"generic-reference-trait-native\n");
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn trait_native_conditional_dispatch_preserves_scalar_aggregate_abi_and_receiver_order() {
        let program = frontend::parse(
            r#"
            trait Pass {
                fn take(value: Self) -> Self;
                fn relay(self) -> Self;
                fn code(self) -> u64;
            }
            enum Tag { Ready }
            impl Pass for Tag {
                fn take(value: Self) -> Self { value }
                fn relay(self) -> Self { self }
                fn code(self) -> u64 { 41u64 }
            }
            struct Packet<T> { value: T, words: [u64;9] }
            impl<T: Pass> Pass for Packet<T> {
                fn take(value: Self) -> Self { value }
                fn relay(self) -> Self { self }
                fn code(self) -> u64 {
                    if self.words[8] != 18446744073709551615u64 { 1/0; }
                    self.value.code()
                }
            }
            trait Combine { fn combine(self, extra: u64) -> u64; }
            impl<T: Pass> Combine for Packet<T> {
                fn combine(mut self, extra: u64) -> u64 {
                    self.words[0] = extra;
                    self.value.code() + self.words[0]
                }
            }
            fn relay<T: Pass>(value: T) -> T { T::take(value.relay()) }
            fn read<T: Pass>(value: T) -> u64 { <T as Pass>::code(value) }
            fn receiver() -> Packet<Tag> {
                println!("receiver");
                Packet::<Tag> { value: Tag::Ready, words: [18446744073709551615u64;9] }
            }
            fn argument() -> u64 { println!("argument"); 1u64 }
            fn main() {
                if read::<Tag>(relay::<Tag>(Tag::Ready)) != 41u64 { 1/0; }
                let packet = Packet::<Tag> {
                    value: Tag::Ready,
                    words: [18446744073709551615u64;9],
                };
                if read::<Packet<Tag>>(relay::<Packet<Tag>>(packet)) != 41u64 { 1/0; }
                if receiver().combine(argument()) != 42u64 { 1/0; }
                println!("trait-native");
            }
            "#,
        )
        .unwrap();
        let serial = emit(&program).unwrap();
        let parallel = super::super::compile_program_observed(program, 4, false)
            .unwrap()
            .executable
            .unwrap();
        assert_eq!(
            serial, parallel,
            "trait target symbols depend on worker order"
        );
        let path = executable_bytes(&serial);
        let output = execute(Command::new(&path).stdout(Stdio::piped()));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"receiver\nargument\ntrait-native\n");
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn trait_native_seals_reject_wrong_authority_and_dead_unclosed_calls() {
        let program = frontend::parse(
            r#"
            trait Read { fn read(self) -> u64; }
            struct Cell { value: u64 }
            impl Read for Cell { fn read(self) -> u64 { self.value } }
            fn required<T: Read>(value: T) -> u64 { value.read() }
            fn other() -> u64 { 7u64 }
            fn main() {}
            "#,
        )
        .unwrap();
        let signatures = frontend::signature_table(&program).unwrap();
        let plan = signatures.plan().unwrap();
        let checked = frontend::type_function_with_signatures(&program, 2, &signatures).unwrap();
        let facts = source_fixture_facts(&signatures, 2).unwrap();
        let lowered = cfg::lower_with_facts(&checked, &facts, facts.budget()).unwrap();
        assert!(seal_concrete_cfg(lowered.clone(), facts.clone()).is_ok());

        let universal = plan.callable_facts(plan.callable_decl(2).unwrap()).unwrap();
        assert!(
            cfg::close(
                std::sync::Arc::new(lowered.clone()),
                universal.clone(),
                universal.budget(),
            )
            .unwrap_err()
            .contains("phase")
        );
        assert!(
            cfg::close(
                std::sync::Arc::new(lowered.clone()),
                facts.clone(),
                &ir::PlanBudget::default(),
            )
            .unwrap_err()
            .contains("budget")
        );
        let wrong_signature = source_fixture_facts(&signatures, 1).unwrap();
        assert!(
            seal_concrete_cfg(lowered.clone(), wrong_signature)
                .unwrap_err()
                .contains("signature")
        );

        let unproved = ir::CallTarget {
            declaration: plan.callable_decl(0).unwrap(),
            type_args: [ir::Ty::Int(ir::IntTy::I32)].into(),
        };
        assert!(plan.instance_facts(&unproved).is_err());

        let with_dead_call = |target| {
            let mut function = lowered.clone();
            let destination = ir::LocalId(function.locals.len());
            function.locals.push(ir::LocalDecl {
                name: "unreachable_result".into(),
                ty: ir::Ty::Int(ir::IntTy::U64),
                mutable: false,
            });
            function.blocks.push(cfg::BasicBlock {
                statements: vec![],
                terminator: cfg::Located {
                    origin: None,
                    value: cfg::Terminator::Call {
                        target,
                        name: "diagnostic_name_is_not_dispatch".into(),
                        signature: ir::Signature {
                            ref_contract: Default::default(),
                            params: vec![ir::Ty::Int(ir::IntTy::I32)],
                            result: ir::Ty::Int(ir::IntTy::U64),
                        },
                        args: vec![cfg::Operand::Constant(cfg::Constant::Int(
                            ir::IntValue::i32(3),
                        ))],
                        destination: ir::Place {
                            local: destination,
                            projection: vec![],
                        },
                        next: function.entry,
                    },
                },
            });
            function
        };
        let direct = with_dead_call(ir::Callee::Direct(unproved));
        direct.validate().unwrap();
        assert!(seal_concrete_cfg(direct, facts.clone()).is_err());

        let symbolic = frontend::type_function_with_signatures(&program, 0, &signatures).unwrap();
        let mut trait_call = symbolic
            .exprs
            .iter()
            .find_map(|expression| match &expression.kind {
                ir::ExprKind::Call {
                    target: ir::Callee::TraitItem(call),
                    ..
                } => Some(call.clone()),
                _ => None,
            })
            .expect("universal dispatch must retain its trait item");
        trait_call.trait_ref.self_ty = ir::Ty::Int(ir::IntTy::I32);
        assert!(
            seal_concrete_cfg(with_dead_call(ir::Callee::TraitItem(trait_call)), facts).is_err()
        );
    }

    #[test]
    fn associated_native_scalar_large_aggregate_and_method_generics_preserve_abi() {
        let program = frontend::parse(
            r#"
            enum ScalarTag { First, Second }
            impl ScalarTag {
                fn code(self) -> u8 {
                    match self { ScalarTag::First => 1u8, ScalarTag::Second => 2u8 }
                }
            }
            struct Scalar { value: u64 }
            impl Scalar {
                fn new(value: u64) -> Self { Self { value } }
                fn increment(mut self) -> Self {
                    self.value = self.value + 1u64;
                    self
                }
                fn get(self) -> u64 { self.value }
            }
            struct Packet<T> { value: T }
            impl<T> Packet<T> {
                fn new(value: T) -> Self { Self { value } }
                fn relay(self) -> Self { self }
                fn pair<U>(self, extra: U) -> (T, U) { (self.value, extra) }
            }
            fn main() {
                if ScalarTag::First.code() != 1u8 { 1/0; }
                if ScalarTag::code(ScalarTag::Second) != 2u8 { 1/0; }
                if Scalar::new(40u64).increment().get() != 41u64 { 1/0; }
                let scalar = Packet::<u64>::new(18446744073709551615u64)
                    .pair::<bool>(true);
                if scalar.0 != 18446744073709551615u64 { 1/0; }
                if !scalar.1 { 1/0; }
                let large = Packet::<[u64;9]>::new([18446744073709551615u64;9])
                    .relay().pair::<Scalar>(Scalar::new(6u64));
                if large.0[0] != 18446744073709551615u64 { 1/0; }
                if large.0[8] != 18446744073709551615u64 { 1/0; }
                if large.1.get() != 6u64 { 1/0; }
                let qualified = Scalar::get(Scalar::new(9u64));
                if qualified != 9u64 { 1/0; }
                println!("associated-abi");
            }
            "#,
        )
        .unwrap();
        let output = run(&program);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"associated-abi\n");
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn associated_native_disjoint_symbols_and_receiver_effects_are_stable() {
        let program = frontend::parse(
            r#"
            struct Token<T> { value: T }
            impl Token<u8> {
                fn get(self) -> u8 { self.value }
            }
            impl Token<u64> {
                fn get(self) -> u64 { self.value }
                fn with(self, extra: u64) -> u64 {
                    println!("callee");
                    self.value + extra
                }
            }
            fn receiver() -> Token<u64> {
                println!("receiver");
                Token::<u64> { value: 40u64 }
            }
            fn argument() -> u64 { println!("argument"); 2u64 }
            fn main() {
                if (Token::<u8> { value: 255u8 }).get() != 255u8 { 1/0; }
                if (Token::<u64> { value: 18446744073709551615u64 }).get()
                    != 18446744073709551615u64 { 1/0; }
                if receiver().with(argument()) != 42u64 { 1/0; }
                println!("associated-stable");
            }
            "#,
        )
        .unwrap();
        let serial = emit(&program).unwrap();
        let parallel = super::super::compile_program_observed(program, 4, false)
            .unwrap()
            .executable
            .unwrap();
        assert_eq!(
            serial, parallel,
            "associated symbols depend on worker order"
        );
        let path = executable_bytes(&serial);
        let output = execute(Command::new(&path).stdout(Stdio::piped()));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            output.stdout,
            b"receiver\nargument\ncallee\nassociated-stable\n"
        );
        assert!(output.stderr.is_empty(), "{output:?}");
    }

    #[test]
    fn unresolved_associated_types_never_acquire_native_layout() {
        // Structural layout helpers do not mint semantic projection authority.
        // Even dead zero-length storage must not hide an unresolved term.
        let snapshot = ir::SnapshotId::allocate().unwrap();
        let projection =
            ir::Ty::AssociatedType(std::sync::Arc::new(ir::AssociatedTypeProjection {
                trait_ref: ir::TraitRef {
                    declaration: ir::DeclId { snapshot, index: 0 },
                    self_ty: ir::Ty::Unit,
                    args: std::sync::Arc::from([]),
                },
                item: ir::DeclId { snapshot, index: 1 },
            }));
        for ty in [
            projection.clone(),
            ir::Ty::Array {
                element: std::sync::Arc::new(projection.clone()),
                len: 0,
            },
            ir::Ty::Tuple(std::sync::Arc::from([ir::Ty::Bool, projection])),
        ] {
            assert!(indirect(&ty).unwrap_err().contains("associated type"));
            assert!(value_size(&ty).unwrap_err().contains("associated type"));
        }
    }

    #[test]
    fn generic_native_closedness_standalone_gate_and_deterministic_symbols() {
        let snapshot = ir::SnapshotId::allocate().unwrap();
        let parameter = ir::Ty::Param(ir::ParamId {
            owner: ir::BinderId {
                declaration: ir::DeclId { snapshot, index: 0 },
            },
            index: 0,
        });
        assert!(indirect(&parameter).unwrap_err().contains("no native ABI"));
        assert!(
            value_size(&parameter)
                .unwrap_err()
                .contains("no native layout")
        );
        let unit = unit_concrete_fixture();
        let mut forged = unit.function().as_ref().clone();
        forged.locals.push(ir::LocalDecl {
            name: "dead_hidden_parameter".into(),
            ty: ir::Ty::Array {
                element: std::sync::Arc::new(parameter),
                len: 0,
            },
            mutable: false,
        });
        assert!(
            cfg::close(
                std::sync::Arc::new(forged),
                unit.facts().clone(),
                unit.facts().budget(),
            )
            .is_err()
        );

        let program = frontend::parse(
            r#"
            fn identity<T>(value:T)->T { value }
            fn helper()->u64 { identity::<u64>(18446744073709551615u64) }
            fn main() {
                if identity::<u8>(255u8) != 255u8 { 1/0; }
                if helper() != 18446744073709551615u64 { 1/0; }
            }
            "#,
        )
        .unwrap();
        let env = frontend::signature_table(&program).unwrap();
        let index = authored_function(&program, "main");
        let checked = frontend::type_function_with_signatures(&program, index, &env).unwrap();
        assert!(
            emit_checked_fixture(&checked, &env, index)
                .unwrap_err()
                .contains("instance plan")
        );
        let first = emit(&program).unwrap();
        let parallel = super::super::compile_program_observed(program, 4, false)
            .unwrap()
            .executable
            .unwrap();
        assert_eq!(
            first, parallel,
            "instance symbols and layout must not depend on worker order"
        );
        let path = executable_bytes(&first);
        let output = execute(&mut Command::new(&path));
        std::fs::remove_file(path).unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }

    #[test]
    fn compound_indexed_all_integer_kernels_and_preserved_division_address() {
        use ir::IntTy;
        for ty in [
            IntTy::I8,
            IntTy::I16,
            IntTy::I32,
            IntTy::I64,
            IntTy::Isize,
            IntTy::U8,
            IntTy::U16,
            IntTy::U32,
            IntTy::U64,
            IntTy::Usize,
        ] {
            let name = ty.name();
            let source = format!(
                "fn main() {{ let mut a=[12{name},7{name}]; let index=0usize;
                    a[index] += 3{name}; if a[0] != 15{name} {{ 1/0; }}
                    a[index] -= 3{name}; if a[0] != 12{name} {{ 1/0; }}
                    a[index] *= 3{name}; if a[0] != 36{name} {{ 1/0; }}
                    a[index] /= 3{name}; if a[0] != 12{name} {{ 1/0; }}
                    a[index] %= 5{name}; if a[0] != 2{name} {{ 1/0; }}
                    a[index] |= 8{name}; if a[0] != 10{name} {{ 1/0; }}
                    a[index] &= 11{name}; if a[0] != 10{name} {{ 1/0; }}
                    a[index] ^= 3{name}; if a[0] != 9{name} {{ 1/0; }}
                    a[index] <<= 1u64; if a[0] != 18{name} {{ 1/0; }}
                    a[index] >>= 1i8; if a[0] != 9{name} {{ 1/0; }}
                    if a[1] != 7{name} {{ 1/0; }}
                }}"
            );
            let program = frontend::parse(&source).unwrap();
            let output = run(&program);
            assert_eq!(output.status.code(), Some(0), "{name}: {output:?}");
        }
        let program = frontend::parse(
            r#"
            fn main() {
                let index=1usize;
                let mut a=[7u64,18446744073709551614u64,9u64];
                a[index] /= 3u64;
                if a[1] != 6148914691236517204u64 { 1/0; }
                a[index] *= 3u64;
                if a[1] != 18446744073709551612u64 { 1/0; }
                a[index] %= 10u64;
                if a[1] != 2u64 { 1/0; }
                if a[0] != 7u64 { 1/0; }
                if a[2] != 9u64 { 1/0; }
                let mut signed=[-21i64]; let zero=0usize;
                signed[zero] /= 3i64;
                if signed[0] != -7i64 { 1/0; }
                signed[zero] %= 3i64;
                if signed[0] != -1i64 { 1/0; }
                signed[zero] >>= 63u8;
                if signed[0] != -1i64 { 1/0; }
            }
            "#,
        )
        .unwrap();
        let output = run(&program);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }

    #[test]
    fn compound_rhs_first_current_carrier_index_once_bool_and_traps() {
        let program = frontend::parse(
            r#"
            fn main() {
                let mut value=1; value += { value=10; 1 };
                if value != 11 { 1/0; }
                let mut deferred:i32; deferred += { deferred=7; 2 };
                if deferred != 9 { 1/0; }
                let mut x=1; let mut y=10; let mut pointer=&mut x;
                *pointer += { pointer=&mut y; 3 };
                if *pointer != 13 { 1/0; }
                if x != 1 { 1/0; }
                let mut array=[1,2]; let mut trace=0;
                array[{ trace=trace*10+2; array=[10,20]; 1usize }] += {
                    trace=trace*10+1; 3
                };
                if trace != 12 { 1/0; }
                if array[0] != 10 { 1/0; }
                if array[1] != 23 { 1/0; }
                let mut flags=[false]; let index=0usize;
                flags[index] &= { trace=trace+1; true };
                if flags[0] { 1/0; }
                flags[index] |= { trace=trace+1; true };
                if !flags[0] { 1/0; }
                flags[index] ^= { trace=trace+1; true };
                if flags[0] { 1/0; }
                if trace != 15 { 1/0; }
                println!("compound-order");
            }
            "#,
        )
        .unwrap();
        let output = run(&program);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"compound-order\n");
        for (initial, rhs, operation) in [
            ("255u8", "1u8", "+="),
            ("0u8", "1u8", "-="),
            ("128u8", "2u8", "*="),
            ("1i64", "0i64", "/="),
            ("1i64", "0i64", "%="),
            ("-9223372036854775808i64", "-1i64", "/="),
            ("-9223372036854775808i64", "-1i64", "%="),
            ("1u8", "18446744073709551615u64", "<<="),
            ("1u8", "-1i64", ">>="),
        ] {
            let source = format!(
                "fn main() {{ let mut array=[{initial}]; let index=0usize;
                 let count={rhs}; array[index] {operation} count; }}"
            );
            let program = frontend::parse(&source).unwrap();
            let output = run(&program);
            assert_eq!(output.status.code(), Some(101), "{source}: {output:?}");
        }
    }

    #[test]
    fn shifts_all_hundred_type_pairs_sign_right_and_left_truncation() {
        use ir::IntTy;
        let types = [
            IntTy::I8,
            IntTy::I16,
            IntTy::I32,
            IntTy::I64,
            IntTy::Isize,
            IntTy::U8,
            IntTy::U16,
            IntTy::U32,
            IntTy::U64,
            IntTy::Usize,
        ];
        for lhs in types {
            let name = lhs.name();
            let high = 1_i128 << (lhs.bits() - 1);
            let (all, top, right, doubled) = if lhs.is_signed() {
                (-1, -high, -1, -2)
            } else {
                (high * 2 - 1, high, 1, high * 2 - 2)
            };
            let mut functions = String::new();
            let mut calls = String::new();
            for rhs in types {
                let count = rhs.name();
                functions.push_str(&format!(
                    "fn pair_{count}(value:{name}, count:{count})->bool {{
                        if (value << count) != {top}{name} {{ return false; }}
                        if (value >> count) != {right}{name} {{ return false; }}
                        true
                    }}"
                ));
                calls.push_str(&format!(
                    "if !pair_{count}({all}{name},{}{count}) {{ 1/0; }}",
                    lhs.bits() - 1
                ));
            }
            let source = format!(
                "{functions} fn main() {{ {calls}
                    let all={all}{name}; let zero=0u64; let one=1i8;
                    if (all << zero) != all {{ 1/0; }}
                    if (all >> zero) != all {{ 1/0; }}
                    if (all << one) != {doubled}{name} {{ 1/0; }}
                }}"
            );
            let program = frontend::parse(&source).unwrap();
            let output = run(&program);
            assert_eq!(output.status.code(), Some(0), "{name}: {output:?}");
        }
    }

    #[test]
    fn shifts_full_count_guards_eager_snapshots_and_typed_divergence() {
        let program = frontend::parse(
            r#"
            fn left(trace:&mut i32)->u8 { *trace=*trace*10+1; 129u8 }
            fn count(trace:&mut i32)->u64 { *trace=*trace*10+2; 1u64 }
            fn divergent_left()->u8 { ({ return 3u8; 1u8 }) << 1u32 }
            fn divergent_right()->u8 { 1u8 << { return 4u8; 1u64 } }
            fn main() {
                let mut trace=0;
                let result=left(&mut trace) << count(&mut trace);
                if result != 2u8 { 1/0; }
                if trace != 12 { 1/0; }
                let mut bits=1u8;
                let snapshot=bits << { bits=9u8; 1u32 };
                if snapshot != 2u8 { 1/0; }
                if bits != 9u8 { 1/0; }
                if divergent_left() != 3u8 { 1/0; }
                if divergent_right() != 4u8 { 1/0; }
                println!("checked-shifts");
            }
            "#,
        )
        .unwrap();
        let output = run(&program);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"checked-shifts\n");
        for (lhs, rhs, count) in [
            ("u8", "i8", "-1i8"),
            ("i64", "i64", "-1i64"),
            ("u8", "u64", "8u64"),
            ("u64", "u8", "64u8"),
            ("i32", "u64", "4294967296u64"),
            ("u64", "u64", "18446744073709551615u64"),
        ] {
            for op in ["<<", ">>"] {
                let source = format!(
                    "fn shift(value:{lhs},count:{rhs})->{lhs} {{ value {op} count }}
                     fn main() {{ let result=shift(1{lhs},{count}); }}"
                );
                let program = frontend::parse(&source).unwrap();
                let output = run(&program);
                assert_eq!(output.status.code(), Some(101), "{source}: {output:?}");
            }
        }
    }

    #[test]
    fn bitwise_integer_widths_signed_high_bits_and_scalar_transport() {
        use ir::IntTy;
        for ty in [
            IntTy::I8,
            IntTy::I16,
            IntTy::I32,
            IntTy::I64,
            IntTy::Isize,
            IntTy::U8,
            IntTy::U16,
            IntTy::U32,
            IntTy::U64,
            IntTy::Usize,
        ] {
            let name = ty.name();
            let high = 1_i128 << (ty.bits() - 1);
            let (all, top, low) = if ty.is_signed() {
                (-1, -high, high - 1)
            } else {
                (high * 2 - 1, high, high - 1)
            };
            let source = format!(
                "struct Value {{ padding: [u8; 9], bits: {name} }}
                 fn and(a:{name}, b:{name})->{name} {{ a & b }}
                 fn or(a:{name}, b:{name})->{name} {{ a | b }}
                 fn xor(a:{name}, b:{name})->{name} {{ a ^ b }}
                 fn record()->Value {{ Value {{ padding:[0u8;9], bits:{all}{name} }} }}
                 fn main() {{
                    let value=record(); let reference=&value;
                    if and(reference.bits,{top}{name}) != {top}{name} {{ 1/0; }}
                    if or({low}{name},{top}{name}) != {all}{name} {{ 1/0; }}
                    if xor(reference.bits,{top}{name}) != {low}{name} {{ 1/0; }}
                    if (reference.bits ^ reference.bits) != 0{name} {{ 1/0; }}
                 }}"
            );
            let program = frontend::parse(&source).unwrap();
            let output = run(&program);
            assert_eq!(output.status.code(), Some(0), "{name}: {output:?}");
        }
    }

    #[test]
    fn bitwise_boolean_eager_order_snapshot_truth_tables_and_traps() {
        let program = frontend::parse(
            r#"
            fn value(trace:&mut i32, digit:i32, result:bool)->bool {
                *trace = *trace * 10 + digit; result
            }
            fn main() {
                let mut trace=0;
                if value(&mut trace,1,false) & value(&mut trace,2,true) { 1/0; }
                if !(value(&mut trace,3,true) | value(&mut trace,4,false)) { 1/0; }
                if !(value(&mut trace,5,false) ^ value(&mut trace,6,true)) { 1/0; }
                if trace != 123456 { 1/0; }
                if !(true & true) { 1/0; }
                if false | false { 1/0; }
                if true ^ true { 1/0; }
                let mut original=true;
                let snapshot=original & { original=false; true };
                if !snapshot { 1/0; }
                if original { 1/0; }
                println!("eager-bitwise");
            }
            "#,
        )
        .unwrap();
        let output = run(&program);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"eager-bitwise\n");
        for expression in ["false & fail()", "true | fail()", "false ^ fail()"] {
            let source = format!(
                "fn fail()->bool {{ let zero=0; 1/zero; true }} fn main() {{ let result={expression}; }}"
            );
            let program = frontend::parse(&source).unwrap();
            let output = run(&program);
            assert_eq!(output.status.code(), Some(101), "{expression}: {output:?}");
        }
    }

    #[test]
    fn integer_casts_all_hundred_pairs_normalize_mathematical_boundary_values() {
        use ir::IntTy;
        let types = [
            IntTy::I8,
            IntTy::I16,
            IntTy::I32,
            IntTy::I64,
            IntTy::Isize,
            IntTy::U8,
            IntTy::U16,
            IntTy::U32,
            IntTy::U64,
            IntTy::Usize,
        ];
        for source in types {
            let source_modulus = 1_i128 << source.bits();
            let (minimum, maximum) = if source.is_signed() {
                (-source_modulus / 2, source_modulus / 2 - 1)
            } else {
                (0, source_modulus - 1)
            };
            let body = |number: i128| {
                let mut body = String::new();
                for target in types {
                    // Independent mathematical modulo oracle in the host test,
                    // not the compiler's normalize/mask implementation.
                    let modulus = 1_i128 << target.bits();
                    let low = number.rem_euclid(modulus);
                    let expected = if target.is_signed() && low >= modulus / 2 {
                        low - modulus
                    } else {
                        low
                    };
                    let target = target.name();
                    body.push_str(&format!(
                        "if (value as {target}) != {expected}{target} {{ return false; }}"
                    ));
                }
                body.push_str("true");
                body
            };
            let name = source.name();
            let source = format!(
                "fn minimum(value: {name}) -> bool {{ {} }} fn maximum(value: {name}) -> bool {{ {} }} fn main() {{ if !minimum({minimum}{name}) {{ 1/0; }} if !maximum({maximum}{name}) {{ 1/0; }} }}",
                body(minimum),
                body(maximum)
            );
            let p = frontend::parse(&source).unwrap();
            let output = run(&p);
            assert_eq!(output.status.code(), Some(0), "{name}: {output:?}");
        }
    }

    #[test]
    fn integer_casts_operand_once_reference_snapshot_sret_and_typed_divergence() {
        let p = frontend::parse(
            r#"
            struct Record { guard: [u8; 9], value: i16 }
            fn source(counter: &mut i32) -> i16 { *counter = *counter + 1; -1i16 }
            fn array() -> [u8; 9] { [300i32 as u8; 9] }
            fn divergent() -> u8 { ({ return 7u8; 300i32 }) as u8 }
            fn main() {
                let mut counter = 0;
                let wide = source(&mut counter) as u64;
                if wide != 18446744073709551615u64 { 1 / 0; }
                if counter != 1 { 1 / 0; }
                let mut record = Record { guard: array(), value: -1i16 };
                let reference = &record;
                let narrow = reference.value as u8;
                if narrow != 255u8 { 1 / 0; }
                record.value = 257i16;
                if (record.value as u8) != 1u8 { 1 / 0; }
                if record.guard[8] != 44u8 { 1 / 0; }
                if (-1i16 as u8 as u16) != 255u16 { 1 / 0; }
                if ((!1i8) as u16) != 65534u16 { 1 / 0; }
                if ((200 + 100) as u8) != 44u8 { 1 / 0; }
                if ((if true { 300 } else { 1 }) as u8) != 44u8 { 1 / 0; }
                if ((match true { true => 300, false => 1 }) as u8) != 44u8 { 1 / 0; }
                if divergent() != 7u8 { 1 / 0; }
                println!("integer-casts");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"integer-casts\n");
    }

    #[test]
    fn integer_types_all_width_normalization_arithmetic_comparison_and_not() {
        for (ty, minimum, maximum) in [
            ("i8", "-128", "127"),
            ("i16", "-32768", "32767"),
            ("i32", "-2147483648", "2147483647"),
            ("i64", "-9223372036854775808", "9223372036854775807"),
            ("isize", "-9223372036854775808", "9223372036854775807"),
            ("u8", "0", "255"),
            ("u16", "0", "65535"),
            ("u32", "0", "4294967295"),
            ("u64", "0", "18446744073709551615"),
            ("usize", "0", "18446744073709551615"),
        ] {
            let signed = if ty.starts_with('i') {
                format!(
                    "if -a != -9{ty} {{ return false; }} if a / -b != -4{ty} {{ return false; }} if -a % b != -1{ty} {{ return false; }} if {minimum}{ty} >= 0{ty} {{ return false; }}"
                )
            } else {
                String::new()
            };
            let source = format!(
                r#"
                fn identity(value: {ty}) -> {ty} {{ value }}
                fn probe(a: {ty}, b: {ty}) -> bool {{
                    if a + b != 11{ty} {{ return false; }}
                    if a - b != 7{ty} {{ return false; }}
                    if a * b != 18{ty} {{ return false; }}
                    if a / b != 4{ty} {{ return false; }}
                    if a % b != 1{ty} {{ return false; }}
                    if a <= b {{ return false; }}
                    if b >= a {{ return false; }}
                    {signed}
                    true
                }}
                fn main() {{
                    if !probe(9{ty}, 2{ty}) {{ 1 / 0; }}
                    if identity({minimum}{ty}) != {minimum}{ty} {{ 1 / 0; }}
                    if identity({maximum}{ty}) != {maximum}{ty} {{ 1 / 0; }}
                    if !{minimum}{ty} != {maximum}{ty} {{ 1 / 0; }}
                    if {maximum}{ty} <= 9{ty} {{ 1 / 0; }}
                }}
            "#,
            );
            let p = frontend::parse(&source).unwrap();
            let output = run(&p);
            assert_eq!(output.status.code(), Some(0), "{ty}: {output:?}");
        }
    }

    #[test]
    fn integer_types_runtime_overflow_and_division_trap_at_declared_width() {
        for (ty, minimum, maximum) in [
            ("i8", "-128", "127"),
            ("i16", "-32768", "32767"),
            ("i32", "-2147483648", "2147483647"),
            ("i64", "-9223372036854775808", "9223372036854775807"),
            ("isize", "-9223372036854775808", "9223372036854775807"),
            ("u8", "0", "255"),
            ("u16", "0", "65535"),
            ("u32", "0", "4294967295"),
            ("u64", "0", "18446744073709551615"),
            ("usize", "0", "18446744073709551615"),
        ] {
            let mut cases = vec![
                ("a + b", maximum, "1"),
                ("a - b", minimum, "1"),
                ("a * b", maximum, "2"),
                ("a / b", "1", "0"),
                ("a % b", "1", "0"),
            ];
            if ty.starts_with('i') {
                cases.extend([
                    ("-a", minimum, "0"),
                    ("a / b", minimum, "-1"),
                    ("a % b", minimum, "-1"),
                ]);
            }
            for (expression, a, b) in cases {
                let source = format!(
                    "fn operate(a: {ty}, b: {ty}) -> {ty} {{ {expression} }} fn main() {{ operate({a}{ty}, {b}{ty}); }}"
                );
                let p = frontend::parse(&source).unwrap();
                let output = run(&p);
                assert_eq!(
                    output.status.code(),
                    Some(101),
                    "{ty} {expression}: {output:?}"
                );
            }
        }
    }

    #[test]
    fn integer_types_mixed_aggregate_reference_index_payload_and_sret_transport() {
        let p = frontend::parse(
            r#"
            struct Record { before: i8, values: [i16; 9], after: u32, wide: i64 }
            enum Payload { Empty, Value(i8, u64) }
            fn make() -> Record {
                Record { before: -128i8, values: [-32768i16; 9], after: 4294967295u32, wide: -9223372036854775808i64 }
            }
            fn pass(value: Record, marker: u8) -> Record { value }
            fn field<'a>(value: &'a mut Record) -> &'a mut i16 { &mut value.values[8] }
            fn main() {
                let mut value = pass(make(), 255u8);
                let returned = field(&mut value);
                *returned = -123i16;
                if value.values[8usize] != -123i16 { 1 / 0; }
                if value.values[0usize] != -32768i16 { 1 / 0; }
                if value.before != -128i8 { 1 / 0; }
                if value.after != 4294967295u32 { 1 / 0; }
                if value.wide != -9223372036854775808i64 { 1 / 0; }
                let payload = Payload::Value(-128i8, 18446744073709551615u64);
                let valid = match payload {
                    Payload::Empty => false,
                    Payload::Value(small, wide) => small == -128i8 && wide == 18446744073709551615u64
                };
                if !valid { 1 / 0; }
                println!("integer-transport");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"integer-transport\n");
    }

    #[test]
    fn mixed_reference_returns_shared_from_mutable_projected_high64_and_recursion() {
        let p = frontend::parse(
            r#"
            struct Record { guard: [usize; 9], value: usize }
            fn shared<'a>(value: &'a mut usize) -> &'a usize { value }
            fn projected<'a>(value: &'a mut Record, depth: i32) -> &'a usize {
                if depth == 0 { &value.value } else { projected(value, depth - 1) }
            }
            fn main() {
                let mut scalar = 18446744073709551615usize;
                let returned = shared(&mut scalar);
                let copied = returned;
                if *copied != 18446744073709551615usize { 1 / 0; }
                scalar = 13usize;
                if scalar != 13usize { 1 / 0; }
                let mut record = Record { guard: [17usize; 9], value: 9223372036854775808usize };
                let field = projected(&mut record, 3);
                if *field != 9223372036854775808usize { 1 / 0; }
                record.value = 29usize;
                if record.value != 29usize { 1 / 0; }
                if record.guard[8] != 17usize { 1 / 0; }
                println!("mixed-shared-return");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"mixed-shared-return\n");
    }

    #[test]
    fn mixed_reference_returns_mutable_with_unrelated_shared_lifetime_and_standalone_gate() {
        let p = frontend::parse(
            r#"
            struct Record { guard: [usize; 9], value: usize }
            fn target<'a, 'b>(flag: &'b bool, value: &'a mut Record) -> &'a mut usize {
                &mut value.value
            }
            fn main() {
                let mut record = Record { guard: [17usize; 9], value: 1usize };
                let returned: &mut usize;
                { let flag = true; returned = target(&flag, &mut record); }
                *returned = 18446744073709551615usize;
                if record.value != 18446744073709551615usize { 1 / 0; }
                if record.guard[8] != 17usize { 1 / 0; }
                println!("mixed-mutable-return");
            }
        "#,
        )
        .unwrap();
        let env = frontend::signature_table(&p).unwrap();
        let target = frontend::type_function_with_signatures(&p, 0, &env).unwrap();
        assert!(emit_checked_fixture(&target, &env, 0).is_ok());
        let main = frontend::type_function_with_signatures(&p, 1, &env).unwrap();
        assert!(
            emit_checked_fixture(&main, &env, 1)
                .unwrap_err()
                .contains("whole-program noescape")
        );
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"mixed-mutable-return\n");

        let dependent = frontend::parse(
            r#"
            fn target<'a>(flag: &'a bool, value: &'a mut usize) -> &'a mut usize { value }
            fn main() {
                let mut value = 1usize;
                let returned: &mut usize;
                { let flag = true; returned = target(&flag, &mut value); }
                *returned = 2usize;
            }
        "#,
        )
        .unwrap();
        assert!(emit(&dependent).is_err());
    }

    #[test]
    fn projected_reference_returns_native_tuple_array_aggregate_and_recursion() {
        let p = frontend::parse(
            r#"
            struct Record { before: usize, pair: (usize, [usize; 9]), after: usize }
            fn element<'a>(value: &'a Record) -> &'a usize { &(*value).pair.1[8] }
            fn array<'a>(value: &'a mut Record) -> &'a mut [usize; 9] { &mut (*value).pair.1 }
            fn recurse<'a>(value: &'a Record, depth: i32) -> &'a usize {
                if depth == 0 { &(*value).pair.1[8] } else { recurse(value, depth - 1) }
            }
            fn large() -> [usize; 9] { [9223372036854775808usize; 9] }
            fn main() {
                let mut record = Record {
                    before: 17usize, pair: (23usize, [0usize; 9]), after: 29usize
                };
                let returned = array(&mut record);
                *returned = large();
                (*returned)[8] = 18446744073709551615usize;
                let copied = *returned;
                if copied[0] != 9223372036854775808usize { 1 / 0; }
                if *element(&record) != 18446744073709551615usize { 1 / 0; }
                if *recurse(&record, 3) != 18446744073709551615usize { 1 / 0; }
                if record.before != 17usize { 1 / 0; }
                if record.pair.0 != 23usize { 1 / 0; }
                if record.after != 29usize { 1 / 0; }
                println!("projected-return");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"projected-return\n");
    }

    #[test]
    fn projected_reference_returns_native_candidate_actual_layout_correlation() {
        let p = frontend::parse(
            r#"
            struct Left { value: usize, guard: bool }
            struct Right { guard: bool, padding: [bool; 2], value: usize }
            fn choose<'a>(tag: i32, a: &'a mut Left, b: &'a mut Right, first: bool) -> &'a mut usize {
                if first { &mut (*a).value } else { &mut (*b).value }
            }
            fn main() {
                let mut a = Left { value: 1usize, guard: true };
                let mut b = Right { guard: true, padding: [false; 2], value: 2usize };
                let left = choose(0, &mut a, &mut b, true);
                *left = 9223372036854775808usize;
                let right = choose(1, &mut a, &mut b, false);
                *right = 18446744073709551615usize;
                if a.value != 9223372036854775808usize { 1 / 0; }
                if b.value != 18446744073709551615usize { 1 / 0; }
                if !a.guard { 1 / 0; }
                if !b.guard { 1 / 0; }
                if b.padding[0] { 1 / 0; }
                println!("correlated-layouts");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"correlated-layouts\n");
    }

    #[test]
    fn projected_reference_returns_native_private_getter_does_not_grant_source_privacy() {
        let p = frontend::parse(
            r#"
            mod hidden {
                pub struct Data { padding: [usize; 9], value: usize }
                pub fn make() -> Data {
                    Data { padding: [17usize; 9], value: 9223372036854775808usize }
                }
                pub fn get<'a>(data: &'a Data) -> &'a usize { &(*data).value }
                pub fn get_mut<'a>(data: &'a mut Data) -> &'a mut usize { &mut (*data).value }
                pub fn guard(data: &Data) -> usize { (*data).padding[8] }
            }
            fn main() {
                let mut data = hidden::make();
                if *hidden::get(&data) != 9223372036854775808usize { 1 / 0; }
                let returned = hidden::get_mut(&mut data);
                *returned = 18446744073709551615usize;
                if *hidden::get(&data) != 18446744073709551615usize { 1 / 0; }
                if hidden::guard(&data) != 17usize { 1 / 0; }
                println!("private-getter");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"private-getter\n");
        let private_access = frontend::parse(
            "mod hidden { pub struct Data { value: usize } pub fn make() -> Data { Data { value: 1usize } } } fn main() { let data = hidden::make(); let forbidden = data.value; }",
        );
        assert!(private_access.and_then(|p| emit(&p)).is_err());
    }

    #[test]
    fn mutable_reference_return_standalone_consuming_body_requires_call_evidence() {
        let p = frontend::parse(
            r#"
            fn identity<'a>(value: &'a mut usize) -> &'a mut usize { value }
            fn main() { let mut value = 7usize; let observed = *identity(&mut value); }
        "#,
        )
        .unwrap();
        let env = frontend::signature_table(&p).unwrap();
        let identity = frontend::type_function_with_signatures(&p, 0, &env).unwrap();
        assert!(emit_checked_fixture(&identity, &env, 0).is_ok());
        let main = frontend::type_function_with_signatures(&p, 1, &env).unwrap();
        assert!(
            emit_checked_fixture(&main, &env, 1)
                .unwrap_err()
                .contains("whole-program noescape")
        );
        let moved = frontend::parse(
            r#"
            fn identity<'a>(value: &'a mut usize) -> &'a mut usize { value }
            fn main() {
                let mut value = 1usize;
                let returned = identity(&mut value);
                let moved = returned;
                *moved = 2usize;
                *returned = 3usize;
            }
        "#,
        )
        .unwrap();
        assert!(emit(&moved).is_err());
    }

    #[test]
    fn mutable_reference_returns_native_unique_selection_recursion_and_parent_resume() {
        let p = frontend::parse(
            r#"
            fn identity<'a>(value: &'a mut usize) -> &'a mut usize { value }
            fn recurse<'a>(value: &'a mut usize, depth: i32) -> &'a mut usize {
                if depth == 0 { value } else { recurse(value, depth - 1) }
            }
            fn forward<'a>(value: &'a mut usize) -> &'a mut usize { recurse(value, 3) }
            fn choose<'a>(a: &'a mut usize, b: &'a mut usize, first: bool) -> &'a mut usize {
                if first { a } else { b }
            }
            fn main() {
                let mut a = 9223372036854775808usize;
                let mut b = 17usize;
                if *identity(&mut a) != 9223372036854775808usize { 1 / 0; }
                let returned = forward(&mut a);
                let moved = returned;
                *moved = 18446744073709551615usize;
                if *moved != 18446744073709551615usize { 1 / 0; }
                if a != 18446744073709551615usize { 1 / 0; }
                let first = choose(&mut a, &mut b, true);
                *first = 29usize;
                let second = choose(&mut a, &mut b, false);
                *second = 9223372036854775808usize;
                if a != 29usize { 1 / 0; }
                if b != 9223372036854775808usize { 1 / 0; }
                a = 41usize;
                if a != 41usize { 1 / 0; }
                println!("mutable-return");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"mutable-return\n");
    }

    #[test]
    fn mutable_reference_returns_native_aggregate_storage_children_and_lifetime_inputs() {
        let p = frontend::parse(
            r#"
            struct Record { before: usize, items: [usize; 9], after: usize }
            fn identity<'a>(value: &'a mut Record) -> &'a mut Record { value }
            fn independent<'a, 'b>(flag: &'b mut bool, value: &'a mut usize) -> &'a mut usize { value }
            fn large() -> [usize; 9] { [9223372036854775808usize; 9] }
            fn main() {
                let mut record = Record { before: 17usize, items: [0usize; 9], after: 29usize };
                let returned = identity(&mut record);
                (*returned).items = large();
                let child = &mut (*returned).items;
                (*child)[8] = 18446744073709551615usize;
                let copy = *child;
                if copy[0] != 9223372036854775808usize { 1 / 0; }
                if copy[8] != 18446744073709551615usize { 1 / 0; }
                if (*returned).before != 17usize { 1 / 0; }
                if (*returned).after != 29usize { 1 / 0; }
                if record.items[8] != 18446744073709551615usize { 1 / 0; }
                let mut value = 3usize;
                let independent_result: &mut usize;
                { let mut flag = true; independent_result = independent(&mut flag, &mut value); }
                *independent_result = 18446744073709551615usize;
                if value != 18446744073709551615usize { 1 / 0; }
                println!("mutable-return-aggregate");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"mutable-return-aggregate\n");
    }

    #[test]
    fn shared_reference_return_standalone_requires_provenance_and_call_evidence() {
        let p = frontend::parse(
            r#"
            fn identity<'a>(value: &'a usize) -> &'a usize { value }
            fn main() { let value = 7usize; let observed = *identity(&value); }
        "#,
        )
        .unwrap();
        let env = frontend::signature_table(&p).unwrap();
        let identity = frontend::type_function_with_signatures(&p, 0, &env).unwrap();
        assert!(emit_checked_fixture(&identity, &env, 0).is_ok());
        let main = frontend::type_function_with_signatures(&p, 1, &env).unwrap();
        assert!(
            emit_checked_fixture(&main, &env, 1)
                .unwrap_err()
                .contains("whole-program noescape")
        );
        for source in [
            "fn bad<'a>(value: &'a usize) -> &'a usize { let local = 1usize; &local } fn main() {}",
            "fn bad<'a>(value: &'a usize) -> &'a mut usize { value } fn main() {}",
        ] {
            let invalid = frontend::parse(source).unwrap();
            assert!(emit(&invalid).is_err(), "{source}");
        }
    }

    #[test]
    fn shared_reference_returns_preserve_thin_pointer_through_selection_and_recursion() {
        let p = frontend::parse(
            r#"
            struct Record { before: usize, items: [usize; 9], after: usize }
            fn identity<'a>(value: &'a usize) -> &'a usize { value }
            fn record_identity<'z>(value: &'z Record) -> &'z Record { value }
            fn choose<'a>(a: &'a usize, b: &'a usize, first: bool) -> &'a usize {
                if first { a } else { b }
            }
            fn recurse<'a>(value: &'a usize, depth: i32) -> &'a usize {
                if depth == 0 { value } else { recurse(value, depth - 1) }
            }
            fn forward<'a>(value: &'a usize) -> &'a usize { recurse(value, 4) }
            fn main() {
                let mut first = 9223372036854775808usize;
                let second = 18446744073709551615usize;
                if *identity(&first) != 9223372036854775808usize { 1 / 0; }
                let selected = choose(&first, &second, false);
                if *selected != 18446744073709551615usize { 1 / 0; }
                let returned = forward(&first);
                if *returned != 9223372036854775808usize { 1 / 0; }
                first = 13usize;
                if first != 13usize { 1 / 0; }
                let record = Record {
                    before: 17usize, items: [9223372036854775808usize; 9], after: 29usize
                };
                if (*record_identity(&record)).after != 29usize { 1 / 0; }
                let copy = (*record_identity(&record)).items;
                if copy[8] != 9223372036854775808usize { 1 / 0; }
                if record.before != 17usize { 1 / 0; }
                println!("shared-return");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"shared-return\n");
    }

    #[test]
    fn shared_reference_returns_separate_lifetime_inputs_from_pointer_candidates() {
        let p = frontend::parse(
            r#"
            fn independent<'a, 'b>(flag: &'b bool, value: &'a usize) -> &'a usize { value }
            fn shorter<'long: 'short, 'short>(value: &'long usize, flag: &'short bool) -> &'short usize { value }
            fn main() {
                let value = 18446744073709551615usize;
                let returned: &usize;
                { let flag = true; returned = independent(&flag, &value); }
                if *returned != 18446744073709551615usize { 1 / 0; }
                let flag = false;
                if *shorter(&value, &flag) != 18446744073709551615usize { 1 / 0; }
                println!("return-regions");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"return-regions\n");

        let same_region = frontend::parse(
            r#"
            fn pick<'a>(flag: &'a bool, value: &'a usize) -> &'a usize { value }
            fn main() {
                let value = 9usize;
                let returned: &usize;
                { let flag = true; returned = pick(&flag, &value); }
                let observed = *returned;
            }
        "#,
        )
        .unwrap();
        assert!(emit(&same_region).is_err());
    }

    #[test]
    fn reference_calls_require_program_evidence_but_standalone_parameter_body_is_valid() {
        let p = frontend::parse(
            r#"
            fn read(value: &usize) -> usize { *value }
            fn main() {
                let value = 18446744073709551615usize;
                if read(&value) != 18446744073709551615usize { 1 / 0; }
            }
        "#,
        )
        .unwrap();
        let env = frontend::signature_table(&p).unwrap();
        let read = frontend::type_function_with_signatures(&p, 0, &env).unwrap();
        assert!(emit_checked_fixture(&read, &env, 0).is_ok());
        let main = frontend::type_function_with_signatures(&p, 1, &env).unwrap();
        assert!(
            emit_checked_fixture(&main, &env, 1)
                .unwrap_err()
                .contains("whole-program noescape")
        );
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");

        let invalid_unused =
            frontend::parse("fn unused(value: &usize) { *value = 3usize; } fn main() {}").unwrap();
        assert!(emit(&invalid_unused).is_err());
    }

    #[test]
    fn reference_calls_nested_recursive_mutation_and_shared_aliases_preserve_caller_storage() {
        let p = frontend::parse(
            r#"
            fn recurse(value: &mut usize, count: i32) -> usize {
                if count == 0 { *value } else {
                    *value = *value + 1usize;
                    recurse(value, count - 1)
                }
            }
            fn forward(value: &mut usize) -> usize { recurse(value, 3) }
            fn read(value: &usize) -> usize { *value }
            fn same(a: &usize, b: &usize) -> bool { *a == *b }
            fn main() {
                let mut value = 9223372036854775808usize;
                {
                    let reference = &mut value;
                    if forward(reference) != 9223372036854775811usize { 1 / 0; }
                    if recurse(reference, 2) != 9223372036854775813usize { 1 / 0; }
                    if read(reference) != 9223372036854775813usize { 1 / 0; }
                    *reference = 18446744073709551615usize;
                }
                if value != 18446744073709551615usize { 1 / 0; }
                if !same(&value, &value) { 1 / 0; }
                println!("reference-calls");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"reference-calls\n");
    }

    #[test]
    fn reference_calls_aggregate_referents_results_and_mixed_sret_arguments() {
        let p = frontend::parse(
            r#"
            struct Record { before: usize, items: [usize; 9], after: usize }
            fn change(record: &mut Record, value: usize) -> [usize; 9] {
                (*record).items[8] = value;
                (*record).items
            }
            fn forward(record: &mut Record) -> [usize; 9] {
                change(record, 18446744073709551615usize)
            }
            fn mixed(a: &mut usize, padding: [usize; 9], b: &mut usize) -> [usize; 9] {
                *a = padding[0];
                *b = padding[8];
                padding
            }
            fn read(record: &Record) -> usize { (*record).items[8] }
            fn main() {
                let mut record = Record {
                    before: 17usize, items: [9223372036854775808usize; 9], after: 29usize
                };
                let result = forward(&mut record);
                if result[0] != 9223372036854775808usize { 1 / 0; }
                if result[8] != 18446744073709551615usize { 1 / 0; }
                if read(&record) != 18446744073709551615usize { 1 / 0; }
                if record.before != 17usize { 1 / 0; }
                if record.after != 29usize { 1 / 0; }
                let copied = mixed(&mut record.before, result, &mut record.after);
                if record.before != 9223372036854775808usize { 1 / 0; }
                if record.after != 18446744073709551615usize { 1 / 0; }
                if copied[8] != record.items[8] { 1 / 0; }
                println!("reference-sret");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"reference-sret\n");
    }

    #[test]
    fn borrowing_checked_emission_rejects_loan_conflicts_before_pointer_codegen() {
        let program = frontend::parse(
            r#"
            fn main() {
                let mut value = 1;
                let reference = &mut value;
                value = 2;
                let observed = *reference;
            }
        "#,
        )
        .unwrap();
        let signatures = frontend::signature_table(&program).unwrap();
        let checked = frontend::type_function_with_signatures(&program, 0, &signatures).unwrap();
        let facts = source_fixture_facts(&signatures, 0).unwrap();
        let lowered = cfg::lower_with_facts(&checked, &facts, facts.budget()).unwrap();
        assert!(!ownership::analyze(&lowered).passed());
        assert!(emit_checked_fixture(&checked, &signatures, 0).is_err());
        assert!(emit(&program).is_err());
    }

    #[test]
    fn borrowing_thin_shared_copies_and_mutable_move_preserve_usize64() {
        let p = frontend::parse(
            r#"
            fn main() {
                let mut value = 9223372036854775808usize;
                {
                    let reference = &value;
                    let copied = reference;
                    if *copied != 9223372036854775808usize { 1 / 0; }
                }
                {
                    let reference = &mut value;
                    let moved = reference;
                    *moved = 18446744073709551615usize;
                    if *moved != 18446744073709551615usize { 1 / 0; }
                }
                if value != 18446744073709551615usize { 1 / 0; }
                println!("thin-references");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"thin-references\n");
    }

    #[test]
    fn borrowing_write_resolves_current_carrier_after_rhs_and_child_keeps_pointer() {
        let p = frontend::parse(
            r#"
            fn main() {
                let mut x = 1;
                let mut y = 2;
                {
                    let mut reference = &mut x;
                    *reference = { reference = &mut y; 3 };
                }
                if x != 1 { 1 / 0; }
                if y != 3 { 1 / 0; }
                {
                    let mut parent = &mut x;
                    let child = &mut *parent;
                    parent = &mut y;
                    *child = 11;
                    *parent = 22;
                }
                if x != 11 { 1 / 0; }
                if y != 22 { 1 / 0; }
                let child: &mut i32;
                {
                    let parent = &mut x;
                    child = &mut *parent;
                }
                *child = 33;
                if *child != 33 { 1 / 0; }
                println!("current-carrier");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"current-carrier\n");
    }

    #[test]
    fn borrowing_aggregate_fields_arrays_dynamic_indices_and_call_results() {
        let p = frontend::parse(
            r#"
            struct Record { before: i32, values: [usize; 9], after: i32 }
            enum Flag { Off, On }
            fn large() -> [usize; 9] { [9223372036854775808usize; 9] }
            fn scalar() -> usize { 18446744073709551615usize }
            fn main() {
                let mut record = Record { before: 17, values: [0usize; 9], after: 29 };
                {
                    let reference = &mut record;
                    (*reference).values = large();
                    (*reference).values[1] = scalar();
                    let copy = (*reference).values;
                    if copy[8] != 9223372036854775808usize { 1 / 0; }
                    if copy[1] != 18446744073709551615usize { 1 / 0; }
                    if (*reference).before != 17 { 1 / 0; }
                    if (*reference).after != 29 { 1 / 0; }
                }
                let mut array = [0usize; 9];
                {
                    let reference = &mut array;
                    *reference = large();
                    (*reference)[2] = scalar();
                    let copied = *reference;
                    if copied[2usize] != 18446744073709551615usize { 1 / 0; }
                }
                let flag = Flag::On;
                {
                    let reference = &flag;
                }
                let n = match flag { Flag::Off => 0, Flag::On => 1 };
                if n != 1 { 1 / 0; }
                println!("reference-aggregate");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"reference-aggregate\n");
    }

    #[test]
    fn usize_full_width_arithmetic_complement_and_all_transports() {
        let p = frontend::parse(
            r#"
            struct Wrapped { value: usize }
            enum Carrier { Empty, Data(usize) }
            fn identity(value: usize) -> usize { value }
            fn wrapped(value: Wrapped) -> Wrapped { value }
            fn main() {
                let high = identity(9223372036854775808usize);
                let max: usize = !0usize;
                if max != 18446744073709551615usize { 1 / 0; }
                if high + 7usize != 9223372036854775815usize { 1 / 0; }
                if high - 1usize != 9223372036854775807usize { 1 / 0; }
                if 9223372036854775807usize + 1usize != high { 1 / 0; }
                if 4611686018427387904usize * 2usize != high { 1 / 0; }
                if max / high != 1usize { 1 / 0; }
                if max % high != 9223372036854775807usize { 1 / 0; }
                if !high != 9223372036854775807usize { 1 / 0; }
                if high < 1usize { 1 / 0; }
                if high <= 1usize { 1 / 0; }
                if !(high > 1usize) { 1 / 0; }
                if !(high >= 1usize) { 1 / 0; }
                if !(high <= high) { 1 / 0; }
                if !(high >= high) { 1 / 0; }
                if high == 0usize { 1 / 0; }
                let tuple = (high, max);
                if tuple.0 != high { 1 / 0; }
                if tuple.1 != max { 1 / 0; }
                let w = wrapped(Wrapped { value: high });
                if w.value != high { 1 / 0; }
                let carrier = Carrier::Data(max);
                let payload = match carrier { Carrier::Empty => 0usize, Carrier::Data(n) => n };
                if payload != max { 1 / 0; }
                let mut values = [high; 2];
                let i = 1usize;
                if values[i] != high { 1 / 0; }
                values[i] = max;
                if values[i] != max { 1 / 0; }
                println!("usize64");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"usize64\n");
    }

    #[test]
    fn usize_unsigned_overflow_zero_division_and_full_width_bounds_trap() {
        for source in [
            "fn main() { let n = 18446744073709551615usize + 1usize; }",
            "fn main() { let n = 0usize - 1usize; }",
            "fn main() { let n = 9223372036854775808usize * 2usize; }",
            "fn main() { let n = 7usize / 0usize; }",
            "fn main() { let n = 7usize % 0usize; }",
            "fn main() { let a = [7]; let i = 4294967296usize; let n = a[i]; }",
            "fn main() { let a = [7]; let i = 18446744073709551615usize; let n = a[i]; }",
            "fn main() { let mut a = [7]; let i = 4294967296usize; a[i] = 9; }",
            "fn main() { let a = [7; 0]; let i = 0usize; let n = a[i]; }",
            "struct Item { n: i32 } fn main() { let a = [Item { n: 7 }]; let i = 4294967296usize; let _ = a[i]; }",
        ] {
            let p = frontend::parse(source).unwrap();
            let output = run(&p);
            assert_eq!(output.status.code(), Some(101), "{source}: {output:?}");
        }
    }

    #[test]
    fn dynamic_index_reads_replacements_inspection_and_ordered_snapshots() {
        let p = frontend::parse(
            r#"
            struct Item { value: i32 }
            fn rhs() -> [usize; 9] {
                println!("rhs");
                [9223372036854775808usize; 9]
            }
            fn move_items(value: [Item; 1]) -> [Item; 1] { value }
            fn main() {
                let mut rows = [[1usize; 9]; 2];
                rows[{ println!("index"); 1usize }] = rhs();
                let saved = rows[1usize];
                rows[1usize] = [3usize; 9];
                if saved[8] != 9223372036854775808usize { 1 / 0; }
                let replaced = rows[{ rows = [[5usize; 9]; 2]; 0usize }];
                if replaced[8] != 5usize { 1 / 0; }
                let late: [i32; 1];
                let initialized = late[{ late = [7]; 0usize }];
                if initialized != 7 { 1 / 0; }
                let mut items = [Item { value: 17 }];
                let _ = items[0usize];
                items[0usize] = Item { value: 21 };
                let moved = move_items(items);
                if moved[0].value != 21 { 1 / 0; }
                let units = [(); 2];
                let unit = units[1usize];
                println!("dynamic-index");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"rhs\nindex\ndynamic-index\n");
    }

    #[test]
    fn array_literals_assignment_and_repeated_aggregate_transport() {
        let p = frontend::parse(
            r#"
            fn effect(n: i32) -> i32 {
                if n == 1 { println!("first"); }
                if n == 2 { println!("second"); }
                if n == 3 { println!("third"); }
                n
            }
            fn relay(value: [i32; 3]) -> [i32; 3] { value }
            fn tuples(value: [(i32, bool); 9]) -> [(i32, bool); 9] { value }
            fn main() {
                let mut values = relay([effect(1), effect(2), effect(3)]);
                values[1] = values[0] + values[2];
                if values[0] != 1 { 1 / 0; }
                if values[1] != 4 { 1 / 0; }
                if values[2] != 3 { 1 / 0; }
                let copies = tuples([(7, true); 9]);
                if copies[0].0 != 7 { 1 / 0; }
                if copies[8].0 != 7 { 1 / 0; }
                if !copies[8].1 { 1 / 0; }
                println!("array-transport");
            }
            "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"first\nsecond\nthird\narray-transport\n");
    }

    #[test]
    fn array_empty_constructors_zero_full_dummy_word() {
        let ty = ir::Ty::Array {
            element: std::sync::Arc::new(ir::Ty::Int(ir::IntTy::I32)),
            len: 0,
        };
        let concrete = unit_concrete_fixture();
        // The initialized dummy is a private ABI invariant, not observable
        // through source indexing because an empty array has no valid index.
        let mut emitter = Emitter {
            object: Object {
                code: vec![],
                calls: vec![],
                strings: vec![],
            },
            concrete: &concrete,
            resolve: &|_| Err("physical fixture has no instance calls".into()),
            locals: vec![],
            aggregate_scratch: -8,
            wide_scratch: -264,
            returns: vec![],
            traps: vec![],
            edges: vec![],
        };
        emitter.bytes(&[0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 8]);
        for value in [
            cfg::Rvalue::Aggregate {
                ty: ty.clone(),
                fields: vec![],
            },
            cfg::Rvalue::Repeat {
                ty,
                value: cfg::Operand::Constant(cfg::Constant::Int(ir::IntValue::i32(17))),
            },
        ] {
            emitter.bytes(&[0x48, 0xb8]);
            emitter.bytes(&u64::MAX.to_le_bytes());
            emitter.bytes(&[0x48, 0x89, 0x85]);
            emitter.i32(-8);
            emitter.rvalue(&value).unwrap();
            emitter.bytes(&[0x48, 0x8b, 0x85]);
            emitter.i32(-8);
            emitter.bytes(&[0x48, 0x85, 0xc0]);
            emitter.trap_if(0x85);
        }
        emitter.bytes(&[0x31, 0xc0, 0xc9, 0xc3]);
        let trap = emitter.position();
        emitter.bytes(&[
            0xbf, 101, 0, 0, 0, 0xb8, 60, 0, 0, 0, 0x0f, 0x05, 0x0f, 0x0b,
        ]);
        for at in std::mem::take(&mut emitter.traps) {
            emitter.patch(at, trap).unwrap();
        }
        let path = executable_bytes(&link(&[("main".into(), emitter.object)]).unwrap());
        let output = execute(Command::new(&path).stdout(Stdio::piped()));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }

    #[test]
    fn array_empty_and_single_noncopy_repeat_evaluate_once() {
        let p = frontend::parse(
            r#"
            struct Item { value: i32 }
            fn make() -> Item { println!("once"); Item { value: 17 } }
            fn empty(value: [Item; 0]) -> [Item; 0] { value }
            fn single(value: [Item; 1]) -> [Item; 1] { value }
            fn nested(value: [[i32; 0]; 3]) -> [[i32; 0]; 3] { value }
            fn main() {
                let zero = empty([make(); 0]);
                let one = single([make(); 1]);
                if one[0].value != 17 { 1 / 0; }
                let moved = one;
                if moved[0].value != 17 { 1 / 0; }
                let dummy = nested([[0; 0]; 3]);
                let moved_dummy = dummy;
                println!("empty-array");
            }
            "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"once\nonce\nempty-array\n");
        let empty = ir::Ty::Array {
            element: std::sync::Arc::new(ir::Ty::Int(ir::IntTy::I32)),
            len: 0,
        };
        assert!(indirect(&empty).unwrap());
        assert_eq!(value_size(&empty).unwrap(), 8);
        assert!(array_offset(&empty, 0).is_err());
        let nested = ir::Ty::Array {
            element: std::sync::Arc::new(empty),
            len: 3,
        };
        assert_eq!(value_size(&nested).unwrap(), 24);
        assert_eq!(array_offset(&nested, 2).unwrap().0, 16);
    }

    #[test]
    fn array_large_repeat_calls_returns_have_bounded_native_code() {
        let p = frontend::parse(
            r#"
            fn make() -> [i32; 4096] { [23; 4096] }
            fn relay(value: [i32; 4096]) -> [i32; 4096] { value }
            fn main() {
                let value = relay(make());
                if value[0] != 23 { 1 / 0; }
                if value[4095] != 23 { 1 / 0; }
                println!("large-array");
            }
            "#,
        )
        .unwrap();
        let signatures = frontend::signature_table(&p).unwrap();
        let checked = frontend::type_function_with_signatures(&p, 0, &signatures).unwrap();
        let object = emit_checked_fixture(&checked, &signatures, 0).unwrap();
        assert!(
            object.code.len() < 512,
            "repeat code expanded to {} bytes",
            object.code.len()
        );
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"large-array\n");
        let enormous = ir::Ty::Array {
            element: std::sync::Arc::new(ir::Ty::Array {
                element: std::sync::Arc::new(ir::Ty::Int(ir::IntTy::I32)),
                len: 4096,
            }),
            len: 4096,
        };
        assert!(value_size(&enormous).unwrap_err().contains("frame limit"));
    }

    #[test]
    fn singleton_variant_switch_never_loads_uninitialized_or_moved_tag() {
        let modules = fixture_modules();
        fn located<T>(value: T) -> cfg::Located<T> {
            cfg::Located {
                value,
                origin: None,
            }
        }
        fn place(local: usize) -> ir::Place {
            ir::Place {
                local: LocalId(local),
                projection: vec![],
            }
        }
        let snapshot = ir::SnapshotId::allocate().unwrap();
        let ty = ir::EnumTy::new(ir::EnumDef {
            id: ir::NominalAppId { snapshot, index: 0 },
            source_decl: super::super::traits::test_support::source_decl(
                snapshot,
                0,
                &ir::PlanBudget::default(),
            )
            .unwrap(),
            type_args: [].into(),
            name: "Singleton".into(),
            module: modules.core_root(),
            visibility: crate::compiler::ir::Visibility::Restricted(modules.core_root()),
            variants: vec![ir::VariantDef {
                name: "Only".into(),
                shape: ir::VariantShape::Tuple,
                fields: vec![ir::FieldDef {
                    name: "0".into(),
                    ty: ir::Ty::Int(ir::IntTy::I32),
                    visibility: crate::compiler::ir::Visibility::Public,
                }]
                .into(),
            }]
            .into(),
        });
        for moved in [false, true] {
            let mut statements = (0..3)
                .map(|local| located(cfg::Statement::StorageLive(LocalId(local))))
                .collect::<Vec<_>>();
            if moved {
                statements.push(located(cfg::Statement::Assign {
                    destination: place(1),
                    value: cfg::Rvalue::EnumVariant {
                        ty: ty.clone(),
                        variant: 0,
                        fields: vec![cfg::Operand::Constant(cfg::Constant::Int(
                            ir::IntValue::i32(7),
                        ))],
                    },
                }));
                statements.push(located(cfg::Statement::Assign {
                    destination: place(2),
                    value: cfg::Rvalue::Use(cfg::Operand::Move(place(1))),
                }));
            }
            let output_block = |text: &str| cfg::BasicBlock {
                statements: vec![located(cfg::Statement::Assign {
                    destination: place(0),
                    value: cfg::Rvalue::Println(text.into()),
                })],
                terminator: located(cfg::Terminator::Return),
            };
            let function = cfg::Function {
                body_substitution: None,
                call_evidence: std::sync::Arc::from([]),
                lowering_proof: None,
                ref_contract: Default::default(),
                loan_sites: vec![],
                name: "main".into(),
                params: vec![],
                result: ir::Ty::Unit,
                locals: vec![
                    ir::LocalDecl {
                        name: "result".into(),
                        ty: ir::Ty::Unit,
                        mutable: true,
                    },
                    ir::LocalDecl {
                        name: "value".into(),
                        ty: ir::Ty::Enum(ty.clone()),
                        mutable: true,
                    },
                    ir::LocalDecl {
                        name: "moved".into(),
                        ty: ir::Ty::Enum(ty.clone()),
                        mutable: true,
                    },
                ],
                entry: cfg::BlockId(0),
                return_local: LocalId(0),
                blocks: vec![
                    cfg::BasicBlock {
                        statements,
                        terminator: located(cfg::Terminator::SwitchVariant {
                            place: place(1),
                            variant: ir::VariantId {
                                owner: ty.def().id,
                                index: 0,
                            },
                            then_block: cfg::BlockId(1),
                            else_block: cfg::BlockId(2),
                        }),
                    },
                    output_block("singleton-then"),
                    output_block("wrong-else"),
                ],
            };
            function.validate().unwrap();
            let analysis = ownership::analyze(&function);
            assert!(
                analysis.passed(),
                "moved={moved}: {:?}",
                analysis.diagnostics
            );
            let concrete = seal_concrete_cfg(
                function.clone(),
                raw_fixture_facts(std::sync::Arc::clone(&modules), &function).unwrap(),
            )
            .unwrap();
            let mut branch = Emitter {
                object: Object {
                    code: vec![],
                    calls: vec![],
                    strings: vec![],
                },
                concrete: &concrete,
                resolve: &|_| Err("physical fixture has no instance calls".into()),
                locals: vec![],
                aggregate_scratch: 0,
                wide_scratch: 0,
                returns: vec![],
                traps: vec![],
                edges: vec![],
            };
            branch
                .terminator(&function.blocks[0].terminator.value)
                .unwrap();
            assert_eq!(branch.object.code, [0xe9, 0, 0, 0, 0]);
            assert_eq!(branch.edges, [(1, cfg::BlockId(1))]);
            let object = emit_test_cfg(std::sync::Arc::clone(&modules), &function).unwrap();
            // The root follows the eight-byte unit return slot in the frame.
            // Aggregate moves address it but must never load its tag into eax.
            let root_offset = -(8 + value_size(&ir::Ty::Enum(ty.clone())).unwrap());
            let mut tag_load = vec![0x8b, 0x85];
            tag_load.extend_from_slice(&root_offset.to_le_bytes());
            assert!(
                !object
                    .code
                    .windows(tag_load.len())
                    .any(|bytes| bytes == tag_load),
                "singleton tag load emitted, moved={moved}"
            );
            let path = executable_bytes(&link(&[("main".into(), object)]).unwrap());
            let output = execute(Command::new(&path).stdout(Stdio::piped()));
            std::fs::remove_file(&path).unwrap();
            std::fs::remove_dir(path.parent().unwrap()).unwrap();
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert_eq!(output.stdout, b"singleton-then\n");
        }
    }

    #[test]
    fn payload_enum_constructor_zeroes_inactive_backing_words() {
        let modules = fixture_modules();
        let field = |name: &str| ir::FieldDef {
            name: name.into(),
            ty: ir::Ty::Int(ir::IntTy::I32),
            visibility: crate::compiler::ir::Visibility::Public,
        };
        let snapshot = ir::SnapshotId::allocate().unwrap();
        let ty = ir::EnumTy::new(ir::EnumDef {
            id: ir::NominalAppId { snapshot, index: 0 },
            source_decl: ir::DeclId { snapshot, index: 0 },
            type_args: [].into(),
            name: "Backing".into(),
            module: modules.core_root(),
            visibility: crate::compiler::ir::Visibility::Restricted(modules.core_root()),
            variants: vec![
                ir::VariantDef {
                    name: "Empty".into(),
                    shape: ir::VariantShape::Unit,
                    fields: vec![].into(),
                },
                ir::VariantDef {
                    name: "Small".into(),
                    shape: ir::VariantShape::Tuple,
                    fields: vec![field("0")].into(),
                },
                ir::VariantDef {
                    name: "Large".into(),
                    shape: ir::VariantShape::Tuple,
                    fields: vec![field("0"), field("1"), field("2")].into(),
                },
            ]
            .into(),
        });
        assert!(indirect(&ir::Ty::Enum(ty.clone())).unwrap());
        assert_eq!(value_size(&ir::Ty::Enum(ty.clone())).unwrap(), 32);
        let concrete = unit_concrete_fixture();
        // This test exercises the private constructor emitter directly so it
        // can inspect inactive physical storage, which source Rust cannot read.
        let mut emitter = Emitter {
            object: Object {
                code: vec![],
                calls: vec![],
                strings: vec![],
            },
            concrete: &concrete,
            resolve: &|_| Err("physical fixture has no instance calls".into()),
            locals: vec![],
            aggregate_scratch: -32,
            wide_scratch: -288,
            returns: vec![],
            traps: vec![],
            edges: vec![],
        };
        emitter.bytes(&[0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 32]);
        for (variant, operands, expected) in [
            (
                1,
                vec![cfg::Operand::Constant(cfg::Constant::Int(
                    ir::IntValue::i32(7),
                ))],
                [1, 7, 0, 0],
            ),
            (0, vec![], [0, 0, 0, 0]),
        ] {
            // Poison all eight bytes of every word, not merely its low i32.
            emitter.bytes(&[0x48, 0xb8]);
            emitter.bytes(&u64::MAX.to_le_bytes());
            for offset in [-32, -24, -16, -8] {
                emitter.bytes(&[0x48, 0x89, 0x85]);
                emitter.i32(offset);
            }
            emitter.enum_variant(&ty, variant, &operands).unwrap();
            for (index, word) in expected.into_iter().enumerate() {
                emitter.bytes(&[0x48, 0x8b, 0x85]);
                emitter.i32(-32 + index as i32 * 8);
                emitter.bytes(&[0x48, 0x3d]);
                emitter.i32(word);
                emitter.trap_if(0x85);
            }
        }
        emitter.bytes(&[0x31, 0xc0, 0xc9, 0xc3]);
        let trap = emitter.position();
        emitter.bytes(&[
            0xbf, 101, 0, 0, 0, 0xb8, 60, 0, 0, 0, 0x0f, 0x05, 0x0f, 0x0b,
        ]);
        for at in std::mem::take(&mut emitter.traps) {
            emitter.patch(at, trap).unwrap();
        }
        let bytes = link(&[("main".into(), emitter.object)]).unwrap();
        let path = executable_bytes(&bytes);
        let output = execute(Command::new(&path).stdout(Stdio::piped()));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }

    #[test]
    fn payload_enum_calls_returns_nested_fields_and_source_order() {
        let p = frontend::parse(
            r#"
            struct Boxed { value: i32 }
            enum Value { Empty, Pair(i32, Boxed), Named { left: i32, right: (i32, bool) } }
            fn relay(value: Value) -> Value { value }
            fn make(n: i32) -> Value {
                if n == 0 { return Value::Empty; }
                Value::Pair(n, Boxed { value: n + 1 })
            }
            fn code(value: Value) -> i32 {
                match value {
                    Value::Empty => 3,
                    Value::Pair(a, Boxed { value: b }) => a * 10 + b,
                    Value::Named { left, right: (n, ready) } => {
                        if !ready { 1 / 0; }
                        left * 10 + n
                    },
                }
            }
            fn effect(n: i32) -> i32 {
                if n == 1 { println!("right"); } else { println!("left"); }
                n
            }
            fn main() {
                if code(relay(make(0))) != 3 { 1 / 0; }
                if code(relay(make(4))) != 45 { 1 / 0; }
                let named = Value::Named { right: (effect(1), true), left: effect(2) };
                if code(relay(named)) != 21 { 1 / 0; }
                let nested = (17, Value::Pair(5, Boxed { value: 6 }), 29);
                if code(nested.1) != 56 { 1 / 0; }
                if nested.0 != 17 { 1 / 0; }
                if nested.2 != 29 { 1 / 0; }
                println!("payload-abi");
            }
            "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"right\nleft\npayload-abi\n");
    }

    #[test]
    fn payload_enum_zero_field_shapes_use_declaration_wide_indirect_abi() {
        let p = frontend::parse(
            r#"
            enum Empty { Unit, Tuple(), Named {} }
            fn relay(value: Empty) -> Empty { value }
            fn code(value: Empty) -> i32 {
                match value {
                    Empty::Unit => 1,
                    Empty::Tuple() => 2,
                    Empty::Named {} => 3,
                }
            }
            fn main() {
                if code(relay(Empty::Unit)) != 1 { 1 / 0; }
                if code(relay(Empty::Tuple())) != 2 { 1 / 0; }
                if code(relay(Empty::Named {})) != 3 { 1 / 0; }
                println!("empty-payload-abi");
            }
            "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"empty-payload-abi\n");
    }

    #[test]
    fn checked_arithmetic_traps() {
        let cases = vec![
            binary(BinaryOp::Add, Expr::i32(i32::MAX), Expr::i32(1)),
            binary(BinaryOp::Sub, Expr::i32(i32::MIN), Expr::i32(1)),
            binary(BinaryOp::Mul, Expr::i32(i32::MAX), Expr::i32(2)),
            Expr::Unary {
                op: UnaryOp::Neg,
                value: Box::new(Expr::i32(i32::MIN)),
            },
            crash(),
            binary(BinaryOp::Rem, Expr::i32(1), Expr::i32(0)),
            binary(BinaryOp::Div, Expr::i32(i32::MIN), Expr::i32(-1)),
            binary(BinaryOp::Rem, Expr::i32(i32::MIN), Expr::i32(-1)),
        ];
        for expression in cases {
            let output = run(&program(vec![Stmt::Expr(expression)], vec![]));
            assert_eq!(output.status.code(), Some(101), "{output:?}");
        }
    }

    #[test]
    fn arithmetic_comparisons_and_short_circuit() {
        let mut statements = vec![
            equals(binary(BinaryOp::Add, Expr::i32(7), Expr::i32(-3)), 4),
            equals(binary(BinaryOp::Sub, Expr::i32(7), Expr::i32(-3)), 10),
            equals(binary(BinaryOp::Mul, Expr::i32(-7), Expr::i32(3)), -21),
            equals(binary(BinaryOp::Div, Expr::i32(-7), Expr::i32(3)), -2),
            equals(binary(BinaryOp::Rem, Expr::i32(-7), Expr::i32(3)), -1),
            equals(
                binary(BinaryOp::Div, Expr::i32(i32::MIN), Expr::i32(1)),
                i32::MIN,
            ),
            equals(
                Expr::Unary {
                    op: UnaryOp::Neg,
                    value: Box::new(Expr::i32(7)),
                },
                -7,
            ),
            bool_equals(
                Expr::Unary {
                    op: UnaryOp::Not,
                    value: Box::new(Expr::Bool(true)),
                },
                false,
            ),
            bool_equals(
                binary(BinaryOp::And, Expr::Bool(false), crash_bool()),
                false,
            ),
            bool_equals(binary(BinaryOp::Or, Expr::Bool(true), crash_bool()), true),
            bool_equals(
                binary(BinaryOp::And, Expr::Bool(true), Expr::Bool(false)),
                false,
            ),
            bool_equals(
                binary(BinaryOp::Or, Expr::Bool(false), Expr::Bool(true)),
                true,
            ),
        ];
        for (op, expected) in [
            (BinaryOp::Eq, 0),
            (BinaryOp::Ne, 1),
            (BinaryOp::Lt, 1),
            (BinaryOp::Le, 1),
            (BinaryOp::Gt, 0),
            (BinaryOp::Ge, 0),
        ] {
            statements.push(bool_equals(
                binary(op, Expr::i32(-7), Expr::i32(3)),
                expected != 0,
            ));
        }
        let output = run(&program(statements, vec![]));
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }

    #[test]
    fn recursion_many_arguments_returns_and_stable_scopes() {
        let mut builder = fixture_builder();
        let factorial = function(
            "factorial",
            vec![("n".into(), Type::Int(ir::IntTy::I32))],
            Type::Int(ir::IntTy::I32),
            block(
                vec![],
                Some(Expr::If {
                    condition: Box::new(binary(BinaryOp::Le, var("n", &mut builder), Expr::i32(1))),
                    then_block: block(vec![Stmt::Return(Some(Expr::i32(1)))], None),
                    else_block: block(
                        vec![],
                        Some(binary(
                            BinaryOp::Mul,
                            var("n", &mut builder),
                            call(
                                "factorial",
                                vec![binary(BinaryOp::Sub, var("n", &mut builder), Expr::i32(1))],
                                &mut builder,
                            ),
                        )),
                    ),
                }),
            ),
            &mut builder,
        );
        let pick = function(
            "pick",
            (0..12)
                .map(|i| (format!("p{i}"), Type::Int(ir::IntTy::I32)))
                .collect(),
            Type::Int(ir::IntTy::I32),
            block(
                vec![],
                Some(binary(
                    BinaryOp::Sub,
                    var("p0", &mut builder),
                    var("p11", &mut builder),
                )),
            ),
            &mut builder,
        );
        let local = |name: &str, n, builder: &mut frontend::ProgramBuilder| Stmt::Let {
            name: frontend::BindingName::Named(builder.ident(name).unwrap()),
            mutable: true,
            ty: Type::Int(ir::IntTy::I32),
            value: Expr::i32(n),
        };
        let statements = vec![
            local("x", 3, &mut builder),
            equals(
                binary(
                    BinaryOp::Add,
                    Expr::i32(2),
                    Expr::Block(block(
                        vec![local("x", 9, &mut builder)],
                        Some(var("x", &mut builder)),
                    )),
                ),
                11,
            ),
            equals(var("x", &mut builder), 3),
            Stmt::While {
                label: None,
                condition: binary(BinaryOp::Lt, var("x", &mut builder), Expr::i32(6)),
                body: block(
                    vec![
                        Stmt::Assign {
                            name: frontend::LocalName::Named(builder.ident("x").unwrap()),
                            value: binary(BinaryOp::Add, var("x", &mut builder), Expr::i32(1)),
                        },
                        local("temporary", 100, &mut builder),
                    ],
                    None,
                ),
            },
            equals(var("x", &mut builder), 6),
            equals(call("factorial", vec![Expr::i32(6)], &mut builder), 720),
            equals(
                call("pick", (0..12).map(Expr::i32).collect(), &mut builder),
                -11,
            ),
        ];
        let output = run(&owned_program(statements, vec![factorial, pick], builder));
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }

    #[test]
    fn literal_writes_and_write_failures() {
        let p = program(
            vec![
                Stmt::Expr(Expr::Println("Hello, 世界 🦀".into())),
                Stmt::Expr(Expr::Println(String::new())),
            ],
            vec![],
        );
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0));
        assert_eq!(output.stdout, "Hello, 世界 🦀\n\n".as_bytes());
        let path = executable(&p);
        let full = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap();
        let status = execute(Command::new(&path).stdout(Stdio::from(full))).status;
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        assert_eq!(status.code(), Some(101));
    }

    #[test]
    fn literal_output_streams_newlines_empty_and_unicode() {
        let p = frontend::parse(
            r#"fn main() {
                print!("");
                eprint!("");
                print!("out 世界 🦀");
                eprint!("err λ 🦀");
                println!(" legacy");
                eprintln!(" line");
                eprintln!();
                print!("tail\n");
                eprint!("tail\n");
            }"#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, "out 世界 🦀 legacy\ntail\n".as_bytes());
        assert_eq!(output.stderr, "err λ 🦀 line\n\ntail\n".as_bytes());
    }

    #[test]
    fn literal_output_errors_are_terminal_but_empty_output_does_not_write() {
        use std::os::unix::process::CommandExt;

        unsafe extern "C" {
            fn syscall(number: std::ffi::c_long, ...) -> std::ffi::c_long;
        }
        for (source, expected) in [
            ("fn main(){print!(\"\");eprint!(\"\");}", 0),
            ("fn main(){print!(\"x\");}", 101),
            ("fn main(){eprint!(\"x\");}", 101),
            ("fn main(){eprintln!();}", 101),
        ] {
            let p = frontend::parse(source).unwrap();
            let path = executable(&p);
            let full = std::fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap();
            let mut command = Command::new(&path);
            command.stdout(Stdio::from(full));
            // SAFETY: only the forked child redirects stderr to the already
            // installed stdout descriptor. dup2 uses no allocation or locks.
            unsafe {
                command.pre_exec(|| {
                    if syscall(33, 1usize, 2usize) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let output = execute(&mut command);
            std::fs::remove_file(&path).unwrap();
            std::fs::remove_dir(path.parent().unwrap()).unwrap();
            assert_eq!(output.status.code(), Some(expected), "{source}: {output:?}");
            assert!(output.stdout.is_empty());
            assert!(output.stderr.is_empty());
        }
    }

    #[test]
    fn literal_output_large_stderr_drains_without_pipe_deadlock() {
        let literal = "🦀abcdefgh".repeat(8192);
        let source = format!("fn main(){{eprint!(\"{literal}\");}}");
        let output = run(&frontend::parse(&source).unwrap());
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, literal.as_bytes());
    }

    #[test]
    fn argument_bound_and_early_return_with_temporaries() {
        let mut builder = fixture_builder();
        let pick = function(
            "pick",
            (0..MAX_ARGS)
                .map(|i| (format!("p{i}"), Type::Int(ir::IntTy::I32)))
                .collect(),
            Type::Int(ir::IntTy::I32),
            block(
                vec![],
                Some(binary(
                    BinaryOp::Sub,
                    var("p0", &mut builder),
                    var("p4095", &mut builder),
                )),
            ),
            &mut builder,
        );
        let early = function(
            "early",
            vec![],
            Type::Int(ir::IntTy::I32),
            block(
                vec![],
                Some(binary(
                    BinaryOp::Add,
                    Expr::i32(99),
                    Expr::Block(block(
                        vec![Stmt::Return(Some(Expr::i32(7)))],
                        Some(Expr::i32(0)),
                    )),
                )),
            ),
            &mut builder,
        );
        let extra = frontend::BindingName::Named(builder.ident("extra").unwrap());
        let too_many_arguments = call("pick", vec![Expr::i32(0); MAX_ARGS + 1], &mut builder);
        let p = owned_program(
            vec![
                equals(
                    call(
                        "pick",
                        (0..MAX_ARGS as i32).map(Expr::i32).collect(),
                        &mut builder,
                    ),
                    -4095,
                ),
                equals(
                    binary(
                        BinaryOp::Add,
                        Expr::i32(1),
                        call("early", vec![], &mut builder),
                    ),
                    8,
                ),
            ],
            vec![pick, early],
            builder,
        );
        assert_eq!(run(&p).status.code(), Some(0));
        let mut too_many = p.clone();
        too_many.functions[0]
            .params
            .push((extra, Type::Int(ir::IntTy::I32)));
        let error = emit(&too_many).unwrap_err();
        assert!(error.contains("4096"), "{error}");
        let mut too_many = p.clone();
        too_many.functions.last_mut().unwrap().body =
            block(vec![Stmt::Expr(too_many_arguments)], None);
        assert!(emit(&too_many).unwrap_err().contains("4096"));
    }

    #[test]
    fn unit_arguments_nested_calls_and_left_to_right_effects() {
        let mut builder = fixture_builder();
        let unit = function(
            "unit",
            vec![],
            Type::Unit,
            block(vec![Stmt::Expr(Expr::Println("unit".into()))], None),
            &mut builder,
        );
        let identity = function(
            "identity",
            vec![("n".into(), Type::Int(ir::IntTy::I32))],
            Type::Int(ir::IntTy::I32),
            block(vec![], Some(var("n", &mut builder))),
            &mut builder,
        );
        let combine = function(
            "combine",
            vec![
                ("a".into(), Type::Int(ir::IntTy::I32)),
                ("u".into(), Type::Unit),
                ("b".into(), Type::Int(ir::IntTy::I32)),
            ],
            Type::Int(ir::IntTy::I32),
            block(
                vec![],
                Some(binary(
                    BinaryOp::Add,
                    binary(BinaryOp::Mul, var("a", &mut builder), Expr::i32(10)),
                    var("b", &mut builder),
                )),
            ),
            &mut builder,
        );
        let early = function(
            "early",
            vec![],
            Type::Int(ir::IntTy::I32),
            block(
                vec![],
                Some(call(
                    "combine",
                    vec![
                        Expr::i32(1),
                        Expr::Block(block(vec![Stmt::Return(Some(Expr::i32(9)))], None)),
                        Expr::i32(3),
                    ],
                    &mut builder,
                )),
            ),
            &mut builder,
        );
        let effect = |label: &str, value, builder: &mut frontend::ProgramBuilder| {
            call(
                "identity",
                vec![Expr::Block(block(
                    vec![Stmt::Expr(Expr::Println(label.into()))],
                    Some(Expr::i32(value)),
                ))],
                builder,
            )
        };
        let p = owned_program(
            vec![
                equals(
                    binary(
                        BinaryOp::Add,
                        call(
                            "combine",
                            vec![
                                effect("first", 2, &mut builder),
                                call("unit", vec![], &mut builder),
                                effect("last", 3, &mut builder),
                            ],
                            &mut builder,
                        ),
                        call("identity", vec![Expr::i32(4)], &mut builder),
                    ),
                    27,
                ),
                equals(
                    binary(
                        BinaryOp::Mul,
                        Expr::i32(2),
                        call("early", vec![], &mut builder),
                    ),
                    18,
                ),
            ],
            vec![unit, identity, combine, early],
            builder,
        );
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"first\nunit\nlast\n");
    }

    #[test]
    fn large_literal_drains_without_pipe_deadlock() {
        let literal = "🦀abcdefgh".repeat(32 * 1024);
        let output = run(&program(
            vec![Stmt::Expr(Expr::Println(literal.clone()))],
            vec![],
        ));
        assert_eq!(output.status.code(), Some(0));
        let mut expected = literal.into_bytes();
        expected.push(b'\n');
        assert_eq!(output.stdout, expected);
    }

    #[test]
    fn elf_permissions_and_relocation_errors() {
        let p = program(vec![], vec![]);
        let bytes = emit(&p).unwrap();
        assert_eq!(&bytes[..4], b"\x7fELF");
        for index in 0..4 {
            let flags =
                u32::from_le_bytes(bytes[68 + index * 56..72 + index * 56].try_into().unwrap());
            assert_ne!(flags & 3, 3, "writable executable segment");
        }
        assert_eq!(run(&p).status.code(), Some(0));
        assert!(link(&[]).is_err());
        let signatures = frontend::signature_table(&p).unwrap();
        let checked = frontend::type_function_with_signatures(&p, 0, &signatures).unwrap();
        let object = emit_checked_fixture(&checked, &signatures, 0).unwrap();
        assert!(link(&[("main".into(), object.clone()), ("main".into(), object)]).is_err());
        let mut builder = fixture_builder();
        let missing = call("missing", vec![], &mut builder);
        assert!(emit(&owned_program(vec![Stmt::Expr(missing)], vec![], builder)).is_err());
    }

    #[test]
    fn checked_emission_uses_local_ids_not_display_names() {
        let mut builder = fixture_builder();
        let p = owned_program(
            vec![equals(
                call("shadow", vec![Expr::i32(17)], &mut builder),
                19,
            )],
            vec![function(
                "shadow",
                vec![("x".into(), Type::Int(ir::IntTy::I32))],
                Type::Int(ir::IntTy::I32),
                block(
                    vec![Stmt::Let {
                        name: frontend::BindingName::Named(builder.ident("x").unwrap()),
                        mutable: false,
                        ty: Type::Int(ir::IntTy::I32),
                        value: binary(BinaryOp::Add, var("x", &mut builder), Expr::i32(2)),
                    }],
                    Some(var("x", &mut builder)),
                ),
                &mut builder,
            )],
            builder,
        );
        assert_eq!(run(&p).status.code(), Some(0));
        let signatures = frontend::signature_table(&p).unwrap();
        let mut checked = frontend::type_function_with_signatures(&p, 0, &signatures).unwrap();
        let original = emit_checked_fixture(&checked, &signatures, 0).unwrap();
        for local in &mut checked.locals {
            local.name = "not_a_source_binding".into();
        }
        let renamed = emit_checked_fixture(&checked, &signatures, 0).unwrap();
        assert_eq!(original.code, renamed.code);
        assert_eq!(original.calls, renamed.calls);
        assert_eq!(original.strings, renamed.strings);
        // Both the parameter and shadow binding own a distinct monotonic slot.
        let frame = i32::from_le_bytes(original.code[7..11].try_into().unwrap());
        assert!(frame >= 16 && frame % 8 == 0);
        assert!(
            original
                .code
                .windows(7)
                .any(|bytes| bytes == [0x48, 0x8d, 0x85, 0xf8, 0xff, 0xff, 0xff])
        );
        assert!(
            original
                .code
                .windows(7)
                .any(|bytes| bytes == [0x48, 0x8d, 0x85, 0xf0, 0xff, 0xff, 0xff])
        );
    }

    #[test]
    fn loop_values_preserve_outer_operands_and_prior_call_arguments() {
        let p = frontend::parse(
            r#"
            fn pair(a: i32, b: i32) -> i32 { a * 10 + b }
            fn with_unit(a: (), b: i32) -> i32 { b }
            fn main() {
                let x = 100 + loop { break 7 + loop { break 2; }; };
                if x != 109 { 1 / 0; }
                let y = pair(3, loop { break pair(4, loop { break 5; }); });
                if y != 75 { 1 / 0; }
                let z = loop { break { break 11; }; };
                if z != 11 { 1 / 0; }
                let b = loop { break true; };
                if !b { 1 / 0; }
                loop { break; }
                let u = with_unit(loop { break println!("unit"); }, 8);
                if u != 8 { 1 / 0; }
                println!("values");
            }
            "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"unit\nvalues\n");
    }

    #[test]
    fn loop_jumps_discard_only_target_pending_temporaries() {
        let p = frontend::parse(
            r#"
            fn pair(a: i32, b: i32) -> i32 { a * 10 + b }
            fn work() -> i32 {
                let mut n = 0;
                let value = 50 + loop {
                    n = n + 1;
                    if n < 3 { pair(100, { continue; }); }
                    pair(200, { break 4; });
                };
                if n != 3 { 1 / 0; }
                value
            }
            fn main() {
                if work() != 54 { 1 / 0; }
                let mut n = 0;
                let value = loop {
                    n = n + 1;
                    if n < 3 { 90 + { continue; 0 }; }
                    80 + { break 6; 0 };
                };
                if value != 6 { 1 / 0; }
                if n != 3 { 1 / 0; }
                let mut turns = 0;
                let nested = loop {
                    turns = turns + 1;
                    let inner = 10 + loop { break 2; };
                    if inner != 12 { 1 / 0; }
                    if turns == 2 { break inner; }
                    continue;
                };
                if nested != 12 { 1 / 0; }
                println!("cleanup");
            }
            "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"cleanup\n");
    }

    #[test]
    fn while_nested_condition_loops_and_body_jumps_preserve_temporaries() {
        let p = frontend::parse(
            r#"
            fn pair(a: i32, b: i32) -> i32 { a + b }
            fn main() {
                let mut n = 0;
                let mut bodies = 0;
                let result = 100 + {
                    while {
                        n = n + 1;
                        let ready = loop {
                            if n < 3 {
                                n = n + 1;
                                pair(30, { continue; });
                            }
                            99 + { break n < 5; 0 };
                        };
                        ready
                    } {
                        bodies = bodies + 1;
                    }
                    7
                };
                if result != 107 { 1 / 0; }
                if n != 5 { 1 / 0; }
                if bodies != 2 { 1 / 0; }
                let mut k = 0;
                while k < 4 {
                    k = k + 1;
                    if k < 4 { pair(10, { continue; }); }
                    20 + { break; 0 };
                }
                if k != 4 { 1 / 0; }
                println!("while");
            }
            "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"while\n");
    }

    #[test]
    fn tuples_snapshot_fields_arguments_and_local_assignment() {
        let p = frontend::parse(
            r#"
            fn check(a: ((i32, bool), ((), i32)), b: i32) -> i32 {
                if a.0.0 != 1 { 1 / 0; }
                if !a.0.1 { 1 / 0; }
                if a.1.1 != 3 { 1 / 0; }
                let mut own = a;
                own.0.0 = 99;
                own.1.0 = ();
                b
            }
            fn main() {
                let mut t = ((1, true), ((), 3));
                let old = t;
                let pair = (t, { t.0.0 = 8; t.0.1 = false; 9 });
                if pair.0.0.0 != 1 { 1 / 0; }
                if !pair.0.0.1 { 1 / 0; }
                if pair.1 != 9 { 1 / 0; }
                t = old;
                let answer = check(t, { t.1.1 = 7; 11 });
                if answer != 11 { 1 / 0; }
                if t.0.0 != 1 { 1 / 0; }
                if t.1.1 != 7 { 1 / 0; }
                t.0 = { t.1.1 = 12; old.0 };
                if t.0.0 != 1 { 1 / 0; }
                if t.1.1 != 12 { 1 / 0; }
                let single = (42,);
                if single.0 != 42 { 1 / 0; }
                println!("snapshots");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"snapshots\n");
    }

    #[test]
    fn tuple_returns_recursion_branch_blocks_and_break_values() {
        let p = frontend::parse(
            r#"
            fn descend(n: i32, t: (i32, (bool, ()))) -> (i32, (bool, ())) {
                if n == 0 { return t; }
                descend(n - 1, (t.0 + 1, t.1))
            }
            fn make(n: i32) -> (i32, (bool, ())) {
                let mut own = (n, (true, ()));
                let result = if n > 0 { own } else { (0, (false, ())) };
                own.0 = 99;
                result
            }
            fn consume(a: (i32, (bool, ())), b: (i32, (bool, ())), c: i32) -> i32 {
                a.0 * 100 + b.0 * 10 + c
            }
            fn main() {
                let t = descend(4, make(2));
                if t.0 != 6 { 1 / 0; }
                if !t.1.0 { 1 / 0; }
                if make(5).0 != 5 { 1 / 0; }
                if ({ make(3) }).0 != 3 { 1 / 0; }
                let mut turns = 0;
                let value = consume(make(1), loop {
                    turns = turns + 1;
                    if turns < 3 { consume(make(8), { continue; }, 0); }
                    consume(make(9), { break make(2); }, 0);
                }, 3);
                if value != 123 { 1 / 0; }
                let projected = (loop { break (make(7), true); }).0.0;
                if projected != 7 { 1 / 0; }
                println!("returns");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"returns\n");
        let signatures = frontend::signature_table(&p).unwrap();
        let index = authored_function(&p, "make");
        let checked = frontend::type_function_with_signatures(&p, index, &signatures).unwrap();
        let object = emit_checked_fixture(&checked, &signatures, index).unwrap();
        assert!(
            object
                .code
                .windows(5)
                .any(|bytes| bytes == [0x48, 0x89, 0xf8, 0xc9, 0xc3]),
            "aggregate epilogue must return the caller destination pointer"
        );
    }

    #[test]
    fn aggregate_layout_and_frame_limits_are_checked() {
        use std::sync::Arc;
        let mixed = ir::Ty::Tuple(Arc::from([
            ir::Ty::Bool,
            ir::Ty::Tuple(Arc::from([ir::Ty::Unit, ir::Ty::Int(ir::IntTy::I32)])),
        ]));
        assert_eq!(value_size(&mixed).unwrap(), 24);
        assert_eq!(field_offset(&mixed, 1).unwrap().0, 8);
        let mut frame = MAX_FRAME_BYTES - 8;
        assert_eq!(
            allocate_value(&mut frame, &ir::Ty::Unit).unwrap(),
            -MAX_FRAME_BYTES
        );
        assert!(allocate_value(&mut frame, &ir::Ty::Bool).is_err());
        let too_large = ir::Ty::Tuple(
            vec![ir::Ty::Int(ir::IntTy::I32); MAX_FRAME_BYTES as usize / 8 + 1].into(),
        );
        assert!(value_size(&too_large).is_err());
        let checked = ir::CheckedFunction {
            body_substitution: None,
            call_evidence: std::sync::Arc::from([]),
            ref_contract: Default::default(),
            name: "oversized".into(),
            params: vec![],
            result: ir::Ty::Unit,
            locals: vec![ir::LocalDecl {
                name: "t".into(),
                ty: too_large,
                mutable: false,
            }],
            exprs: vec![],
            body: ir::Block {
                statements: vec![],
                tail: None,
            },
        };
        let facts = super::super::traits::test_support::facts_for_signature(
            fixture_modules(),
            &ir::Signature {
                ref_contract: checked.ref_contract.clone(),
                params: vec![],
                result: checked.result.clone(),
            },
            &checked
                .locals
                .iter()
                .map(|local| local.ty.clone())
                .collect::<Vec<_>>(),
            &ir::PlanBudget::default(),
        )
        .unwrap();
        let lowered = cfg::lower_with_facts(&checked, &facts, facts.budget()).unwrap();
        let error = seal_concrete_cfg(lowered, facts).unwrap_err();
        assert!(error.contains("ResourceLimit"), "{error}");
    }

    #[test]
    fn cfg_entry_edges_and_aggregate_assignment_snapshot_reads() {
        let modules = fixture_modules();
        fn located<T>(value: T) -> cfg::Located<T> {
            cfg::Located {
                value,
                origin: None,
            }
        }
        fn place(local: usize, fields: &[usize]) -> ir::Place {
            ir::Place {
                local: LocalId(local),
                projection: fields.iter().copied().map(ir::Projection::Field).collect(),
            }
        }
        fn constant(value: i32) -> cfg::Operand {
            cfg::Operand::Constant(cfg::Constant::Int(ir::IntValue::i32(value)))
        }
        fn assign(destination: ir::Place, value: cfg::Rvalue) -> cfg::Located<cfg::Statement> {
            located(cfg::Statement::Assign { destination, value })
        }
        let pair =
            ir::Ty::Tuple(vec![ir::Ty::Int(ir::IntTy::I32), ir::Ty::Int(ir::IntTy::I32)].into());
        let mut function = cfg::Function {
            body_substitution: None,
            call_evidence: std::sync::Arc::from([]),
            lowering_proof: None,
            ref_contract: Default::default(),
            loan_sites: vec![],
            name: "main".into(),
            params: vec![],
            result: ir::Ty::Unit,
            locals: vec![
                ir::LocalDecl {
                    name: "result".into(),
                    ty: ir::Ty::Unit,
                    mutable: true,
                },
                ir::LocalDecl {
                    name: "pair".into(),
                    ty: pair.clone(),
                    mutable: true,
                },
                ir::LocalDecl {
                    name: "condition".into(),
                    ty: ir::Ty::Bool,
                    mutable: true,
                },
            ],
            entry: cfg::BlockId(1),
            return_local: LocalId(0),
            blocks: vec![
                cfg::BasicBlock {
                    statements: vec![],
                    terminator: located(cfg::Terminator::Unreachable),
                },
                cfg::BasicBlock {
                    statements: vec![
                        located(cfg::Statement::StorageLive(LocalId(0))),
                        located(cfg::Statement::StorageLive(LocalId(2))),
                        located(cfg::Statement::StorageLive(LocalId(1))),
                        assign(
                            place(1, &[]),
                            cfg::Rvalue::Aggregate {
                                ty: pair.clone(),
                                fields: vec![constant(1), constant(2)],
                            },
                        ),
                    ],
                    terminator: located(cfg::Terminator::Goto(cfg::BlockId(3))),
                },
                cfg::BasicBlock {
                    statements: vec![
                        located(cfg::Statement::StorageDead(LocalId(1))),
                        assign(place(0, &[]), cfg::Rvalue::Println("cfg".into())),
                    ],
                    terminator: located(cfg::Terminator::Return),
                },
                cfg::BasicBlock {
                    statements: vec![
                        // A direct aggregate assignment must snapshot both
                        // operands before destination field zero is replaced.
                        assign(
                            place(1, &[]),
                            cfg::Rvalue::Aggregate {
                                ty: pair,
                                fields: vec![
                                    cfg::Operand::Copy(place(1, &[1])),
                                    cfg::Operand::Move(place(1, &[0])),
                                ],
                            },
                        ),
                        assign(
                            place(2, &[]),
                            cfg::Rvalue::Binary {
                                op: BinaryOp::Eq,
                                left: cfg::Operand::Copy(place(1, &[0])),
                                right: constant(2),
                            },
                        ),
                    ],
                    terminator: located(cfg::Terminator::Switch {
                        condition: cfg::Operand::Copy(place(2, &[])),
                        then_block: cfg::BlockId(4),
                        else_block: cfg::BlockId(0),
                    }),
                },
                cfg::BasicBlock {
                    statements: vec![assign(
                        place(2, &[]),
                        cfg::Rvalue::Binary {
                            op: BinaryOp::Eq,
                            left: cfg::Operand::Copy(place(1, &[1])),
                            right: constant(1),
                        },
                    )],
                    terminator: located(cfg::Terminator::Switch {
                        condition: cfg::Operand::Copy(place(2, &[])),
                        then_block: cfg::BlockId(2),
                        else_block: cfg::BlockId(0),
                    }),
                },
            ],
        };
        assert!(ownership::analyze(&function).passed());
        let object = emit_test_cfg(std::sync::Arc::clone(&modules), &function).unwrap();
        let bytes = link(&[("main".into(), object.clone())]).unwrap();
        let path = executable_bytes(&bytes);
        let output = execute(Command::new(&path).stdout(Stdio::piped()));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"cfg\n");
        // Provenance is metadata, not a source/type-checking/codegen input.
        for block in &mut function.blocks {
            block.terminator.origin = Some(ir::ExprId(usize::MAX));
            for statement in &mut block.statements {
                statement.origin = Some(ir::ExprId(usize::MAX));
            }
        }
        assert!(ownership::analyze(&function).passed());
        assert_eq!(emit_test_cfg(modules, &function).unwrap().code, object.code);
    }

    #[test]
    fn empty_nominal_private_carrier_has_owned_initialized_word_and_stable_abi() {
        let program =
            frontend::parse("struct Marker;fn make()->Marker{Marker}fn main(){}").unwrap();
        let signatures = frontend::signature_table(&program).unwrap();
        let index = authored_function(&program, "make");
        let checked =
            frontend::type_function_with_signatures(&program, index, &signatures).unwrap();
        let ty = checked.result.clone();
        assert!(matches!(ty, ir::Ty::Struct(_)));
        assert_ne!(ty, ir::Ty::Unit);
        assert!(!ty.is_copy());
        assert_eq!(ty.field_count(), Some(0));
        assert_eq!(value_size(&ty), Ok(8));
        assert_eq!(indirect(&ty), Ok(true));
        assert_eq!(result_indirect(&ty), Ok(true));
        assert_eq!(argument_words(&ty), Ok(1));
        let words =
            parameter_words(&[ty.clone(), ir::Ty::Int(ir::IntTy::I32), ty.clone()]).unwrap();
        assert_eq!(words, vec![1, 1, 1]);
        assert_eq!(argument_slot_offsets(&words, true), Ok(vec![40, 32, 24]));
        assert_eq!(call_cleanup_bytes(&words, true), Ok(32));
        let mut frame = 8;
        assert_eq!(allocate_value(&mut frame, &ty), Ok(-16));
        assert_eq!(allocate_value(&mut frame, &ty), Ok(-24));
        let tuple = ir::Ty::Tuple(vec![ty.clone(), ir::Ty::Int(ir::IntTy::I32), ty.clone()].into());
        assert_eq!(value_size(&tuple), Ok(24));
        assert_eq!(field_offset(&tuple, 1).unwrap().0, 8);
        assert_eq!(field_offset(&tuple, 2).unwrap().0, 16);
        let array = ir::Ty::Array {
            element: std::sync::Arc::new(ty),
            len: 3,
        };
        assert_eq!(value_size(&array), Ok(24));
        assert_eq!(array_offset(&array, 2).unwrap().0, 16);
        let object = emit_checked_fixture(&checked, &signatures, index).unwrap();
        assert!(
            object
                .code
                .windows(5)
                .any(|bytes| bytes == [0xb8, 0, 0, 0, 0]),
            "empty construction must explicitly initialize its private dummy word"
        );
    }

    #[test]
    fn unit_nominal_private_transport_preserves_nested_values_and_argument_effects() {
        let program = frontend::parse(
            r#"
            struct Marker;
            struct Packet { first: Marker, number: i32, last: Marker }
            fn make(n:i32)->Marker {
                if n==1 { println!("first"); } else { println!("last"); }
                Marker
            }
            fn number()->i32 { println!("number"); 42 }
            fn relay(a:Marker,n:i32,b:Marker)->Marker {
                if n!=42 { 1/0; }
                a
            }
            fn pass(value:Packet)->Packet { value }
            fn main() {
                let marker=relay(make(1),number(),make(2));
                let packet=pass(Packet{first:marker,number:42,last:Marker});
                if packet.number!=42 { 1/0; }
                let tuple=(packet.first,7,packet.last);
                if tuple.1!=7 { 1/0; }
                let values=[tuple.0,tuple.2];
                let Marker=values[0];
                let Marker=values[1];
                println!("unit");
            }
            "#,
        )
        .unwrap();
        let output = run(&program);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"first\nnumber\nlast\nunit\n");
    }

    #[test]
    fn tuple_nominal_transport_preserves_nested_fields_and_argument_effects() {
        let program = frontend::parse(
            "struct Marker;struct Pair(i32,i32);struct Packet(Pair,Marker,i32);fn left()->i32{println!(\"first\");2}fn right()->i32{println!(\"second\");40}fn swap(p:Pair)->Pair{let Pair(a,b)=p;Pair(b,a)}fn pass(p:Packet)->Packet{p}fn main(){let packet=pass(Packet(swap(Pair(left(),right())),Marker,7));let Packet(pair,marker,n)=packet;let Pair(a,b)=pair;let Marker=marker;if a==40{if b==2{if n==7{println!(\"tuple\");}}}}",
        )
        .unwrap();
        let output = run(&program);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"first\nsecond\ntuple\n");
    }

    #[test]
    fn nominal_aggregate_abi_handles_one_field_nested_unit_and_recursive_returns() {
        let p = frontend::parse(
            r#"
            struct One { value: i32 }
            struct Flags { ready: bool, unit: () }
            struct Pack { left: One, middle: (Flags, i32), right: One }
            fn one(value: i32) -> One { One { value: value } }
            fn descend(n: i32, item: One) -> One {
                if n == 0 { return item; }
                descend(n - 1, One { value: item.value + 1 })
            }
            fn relay(first: One, flag: bool, item: Pack) -> Pack {
                if first.value != 7 { 1 / 0; }
                if !flag { 1 / 0; }
                item
            }
            fn main() {
                let mut p = Pack {
                    right: one(3),
                    middle: (Flags { unit: (), ready: true }, 2),
                    left: descend(3, one(1)),
                };
                p.left.value = 9;
                let p = relay(one(7), true, p);
                if p.left.value != 9 { 1 / 0; }
                if !p.middle.0.ready { 1 / 0; }
                if p.middle.1 != 2 { 1 / 0; }
                if p.right.value != 3 { 1 / 0; }
                if one(42).value != 42 { 1 / 0; }
                println!("nominal");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"nominal\n");
        let env = frontend::signature_table(&p).unwrap();
        let index = authored_function(&p, "one");
        let checked = frontend::type_function_with_signatures(&p, index, &env).unwrap();
        assert_eq!(checked.result.field_count(), Some(1));
        assert_eq!(value_size(&checked.result).unwrap(), 8);
        let object = emit_checked_fixture(&checked, &env, index).unwrap();
        assert!(
            object
                .code
                .windows(5)
                .any(|bytes| bytes == [0x48, 0x89, 0xf8, 0xc9, 0xc3]),
            "one-field nominal return must still use the aggregate destination ABI"
        );
    }

    #[test]
    fn nominal_initializer_order_and_move_through_tuple_branch_and_loop() {
        let p = frontend::parse(
            r#"
            struct One { value: i32 }
            struct Pair { first: One, second: One }
            fn one(value: i32) -> One { One { value: value } }
            fn relay(item: (One, i32)) -> (One, i32) { item }
            fn main() {
                let p = Pair {
                    second: { println!("second"); one(2) },
                    first: { println!("first"); one(1) },
                };
                let first = p.first;
                let second = p.second;
                let item = if true { (first, 3) } else { (one(9), 8) };
                let item = relay(item);
                let result = loop { break Pair { first: item.0, second: second }; };
                if result.first.value != 1 { 1 / 0; }
                if result.second.value != 2 { 1 / 0; }
                println!("moves");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"second\nfirst\nmoves\n");
    }

    #[test]
    fn checked_and_program_conveniences_reject_use_after_nominal_move() {
        let p = frontend::parse(
            r#"
            struct One { value: i32 }
            fn take(item: One) -> i32 { item.value }
            fn main() {
                let item = One { value: 7 };
                take(item);
                take(item);
            }
        "#,
        )
        .unwrap();
        let env = frontend::signature_table(&p).unwrap();
        let index = authored_function(&p, "main");
        let checked = frontend::type_function_with_signatures(&p, index, &env).unwrap();
        let error = emit_checked_fixture(&checked, &env, index).unwrap_err();
        assert!(error.contains("initialization/move analysis"), "{error}");
        assert!(
            emit(&p)
                .unwrap_err()
                .contains("initialization/move analysis")
        );
    }

    #[test]
    fn let_patterns_selectively_move_fields_without_consuming_ignored_siblings() {
        let p = frontend::parse(
            r#"
            struct Leaf { value: i32 }
            struct Pair { left: Leaf, right: Leaf }
            fn main() {
                let pair = Pair { left: Leaf { value: 1 }, right: Leaf { value: 2 } };
                let Pair { left: first, .. } = pair;
                let remaining = pair.right;
                let tuple = (Leaf { value: 3 }, (Leaf { value: 4 }, 5));
                let (_, (selected, number)) = tuple;
                let ignored = tuple.0;
                if first.value != 1 { 1 / 0; }
                if remaining.value != 2 { 1 / 0; }
                if ignored.value != 3 { 1 / 0; }
                if selected.value != 4 { 1 / 0; }
                if number != 5 { 1 / 0; }
                let (mut scalar,) = (6,);
                scalar = scalar + 1;
                if scalar != 7 { 1 / 0; }
                println!("selected");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"selected\n");
    }

    #[test]
    fn let_pattern_wildcards_skip_moved_places_but_evaluate_nonplaces_once() {
        let p = frontend::parse(
            r#"
            struct Leaf { value: i32 }
            struct Pair { left: Leaf, right: Leaf }
            fn produce() -> (Leaf, i32) {
                println!("produce");
                (Leaf { value: 8 }, 9)
            }
            fn main() {
                let item = Leaf { value: 1 };
                let moved = item;
                let _ = item;
                let pair = Pair { left: Leaf { value: 2 }, right: Leaf { value: 3 } };
                let moved_pair = pair;
                let Pair { .. } = pair;
                let tuple = (Leaf { value: 4 }, Leaf { value: 5 });
                let moved_tuple = tuple;
                let (_, _) = tuple;
                let (made, number) = produce();
                if moved.value != 1 { 1 / 0; }
                if moved_pair.right.value != 3 { 1 / 0; }
                if moved_tuple.0.value != 4 { 1 / 0; }
                if made.value != 8 { 1 / 0; }
                if number != 9 { 1 / 0; }
                let _ = { println!("wildcard"); produce() };
                println!("done");
            }
        "#,
        )
        .unwrap();
        let output = run(&p);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, b"produce\nwildcard\nproduce\ndone\n");
    }

    #[test]
    fn let_pattern_consumed_selected_leaf_is_rejected_by_checked_convenience() {
        for source in [
            "struct Leaf{value:i32} fn main(){let t=(Leaf{value:1},Leaf{value:2});let (selected,_)=t;let again=t.0;}",
            "struct Leaf{value:i32} struct Pair{left:Leaf,right:Leaf} fn main(){let p=Pair{left:Leaf{value:1},right:Leaf{value:2}};let Pair{left:selected,..}=p;let again=p.left;}",
        ] {
            let p = frontend::parse(source).unwrap();
            let env = frontend::signature_table(&p).unwrap();
            let index = authored_function(&p, "main");
            let checked = frontend::type_function_with_signatures(&p, index, &env).unwrap();
            let error = emit_checked_fixture(&checked, &env, index).unwrap_err();
            assert!(error.contains("initialization/move analysis"), "{error}");
            assert!(
                emit(&p)
                    .unwrap_err()
                    .contains("initialization/move analysis")
            );
        }
    }
}
