//! Direct Linux x86-64 backend. No external compiler, assembler, or linker.
//!
//! Private ABI: arguments are evaluated left-to-right and pushed as eight-byte
//! stack words. The callee reads them in reverse order above rbp. eax carries
//! bool results (unit is zero), while rax carries normalized 64-bit integers.
//! Signed integers are sign-extended and unsigned integers zero-extended from
//! their declared width. References remain a separate thin-pointer class.
//! All registers are caller-saved. Frames use
//! stable rbp-relative slots, independent of temporary pushes. At most 4096
//! parameters/arguments are accepted. This is not a platform interoperability ABI.
//! Aggregate layout is private: each scalar/unit leaf occupies eight bytes,
//! tuples and nominal structs concatenate their fields. Payload enums contain
//! an eight-byte tag followed by the largest variant's concatenated fields.
//! Any non-unit variant shape makes the entire enum use aggregate transport,
//! including unit variants of that enum and zero-field tuple/struct variants.
//! Arrays are always indirect and concatenate element storage. Empty arrays
//! own one initialized eight-byte dummy word, including when nested as elements.
//! Aggregate arguments carry snapshot pointers. Aggregate results use a
//! caller-owned destination, passed last at rbp+16, even for a one-field aggregate.
use super::cfg;
use super::frontend::{self, BinaryOp, Program, UnaryOp};
use super::ir::{self, LocalId};
use super::ownership;
use std::collections::HashMap;

const MAX_ARGS: usize = 4096;
const MAX_FRAME_BYTES: i32 = 1024 * 1024;
const BASE: u64 = 0x400000;
const PAGE: usize = 4096;

fn indirect(ty: &ir::Ty) -> bool {
    ty.field_count().is_some()
        || matches!(ty, ir::Ty::Array { .. })
        || matches!(ty, ir::Ty::Enum(ty) if ty.has_payload())
}

fn value_size(ty: &ir::Ty) -> Result<i32, String> {
    if let ir::Ty::Array { element, len } = ty {
        if *len > ir::MAX_ARRAY_LEN {
            return Err("array length exceeds private backend limit".into());
        }
        if *len == 0 {
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
        None => Ok(8),
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
    function: &'a cfg::Function,
    locals: Vec<i32>,
    aggregate_scratch: i32,
    returns: Vec<usize>,
    traps: Vec<usize>,
    edges: Vec<(usize, cfg::BlockId)>,
}
impl<'a> Emitter<'a> {
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
    fn normalize_integer(&mut self, ty: ir::IntTy) {
        if ty.bits() < 64 {
            let shift = (64 - ty.bits()) as u8;
            self.bytes(&[0x48, 0xc1, 0xe0, shift]);
            self.bytes(&[0x48, 0xc1, if ty.is_signed() { 0xf8 } else { 0xe8 }, shift]);
        }
    }
    fn check_integer_range(&mut self, ty: ir::IntTy) {
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
        let mut ty: &'a ir::Ty = &self.function.locals[place.local.0].ty;
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
        if indirect(ty) {
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
        if indirect(ty) {
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
                self.word(value.bits);
                self.normalize_integer(value.ty);
            }
            cfg::Operand::Constant(value) => self.immediate(match value {
                cfg::Constant::Int(_) => {
                    unreachable!("integer constants use typed full-width immediates")
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
        if !indirect(ty) {
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
        let ty = self.function.place_ty(array)?;
        let ir::Ty::Array { element, len } = ty else {
            return Err("runtime index requires array storage".into());
        };
        if self.function.operand_ty(index)? != ir::Ty::Int(ir::IntTy::Usize) {
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
            if indirect(element) {
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
        match op {
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
    fn binary_noninteger(&mut self, op: BinaryOp) -> Result<(), String> {
        let cc = match op {
            BinaryOp::Eq => 0x94,
            BinaryOp::Ne => 0x95,
            _ => return Err("noninteger binary operator is unsupported".into()),
        };
        self.bytes(&[0x48, 0x39, 0xc8, 0x0f, cc, 0xc0, 0x0f, 0xb6, 0xc0]);
        Ok(())
    }
    fn println(&mut self, text: &str) -> Result<(), String> {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(b'\n');
        self.bytes(&[0x48, 0xbe]);
        let at = self.position();
        self.bytes(&[0; 8]);
        let length = u64::try_from(bytes.len()).map_err(|_| "literal too large")?;
        self.object.strings.push((at, bytes));
        self.bytes(&[0x48, 0xba]);
        self.bytes(&length.to_le_bytes());
        let retry = self.position();
        self.bytes(&[0xb8, 1, 0, 0, 0, 0xbf, 1, 0, 0, 0, 0x0f, 0x05]);
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
    fn rvalue(&mut self, value: &cfg::Rvalue) -> Result<(), String> {
        match value {
            cfg::Rvalue::Use(operand) => self.operand(operand)?,
            cfg::Rvalue::Borrow(loan) => {
                let site = self
                    .function
                    .loan_sites
                    .get(loan.0)
                    .ok_or("invalid checked loan site")?;
                self.place_address(&site.place)?;
            }
            cfg::Rvalue::IndexCopy { array, index } => {
                let element = self.index_address(array, index)?;
                if !indirect(element) {
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
                let ty = self.function.operand_ty(value)?;
                if ty == ir::Ty::Never {
                    // The operand diverged, so this dead CFG operation has no
                    // runtime value to load and no concrete integer width.
                    return Ok(());
                }
                self.operand(value)?;
                match (op, ty) {
                    (UnaryOp::Neg, ir::Ty::Int(integer)) if integer.is_signed() => {
                        self.bytes(&[0x48, 0xf7, 0xd8]);
                        self.trap_if(0x80);
                        self.check_integer_range(integer);
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
                let ty = self.function.operand_ty(left)?;
                if ty == ir::Ty::Never || self.function.operand_ty(right)? == ir::Ty::Never {
                    // Operand effects already occurred in earlier CFG steps.
                    // Divergence leaves no value to load or arithmetic to run.
                    return Ok(());
                }
                self.operand(left)?;
                self.bytes(&[0x49, 0x89, 0xc0]); // r8 = normalized left, full width
                self.operand(right)?;
                self.bytes(&[0x48, 0x89, 0xc1, 0x4c, 0x89, 0xc0]);
                if let ir::Ty::Int(integer) = ty {
                    self.binary_integer(*op, integer)?;
                } else {
                    self.binary_noninteger(*op)?;
                }
            }
            cfg::Rvalue::Aggregate { ty, fields } => {
                // Evaluate the whole aggregate into scratch, so destination
                // projections cannot overwrite a later Copy operand's source.
                if matches!(ty, ir::Ty::Array { len: 0, .. }) {
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
                let ty = self.function.place_ty(place)?;
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
                name,
                signature,
                args,
                destination,
                next,
            } => {
                if args.len() > MAX_ARGS {
                    return Err(format!("at most {MAX_ARGS} arguments are supported"));
                }
                for operand in args {
                    self.operand(operand)?;
                    self.bytes(&[0x50]);
                }
                let aggregate = indirect(&signature.result);
                if aggregate {
                    self.place_address(destination)?;
                    self.bytes(&[0x50]);
                }
                let at = self.jump(&[0xe8]);
                self.object.calls.push((at, name.clone()));
                let words = args.len() + usize::from(aggregate);
                if words != 0 {
                    self.bytes(&[0x48, 0x81, 0xc4]);
                    self.i32((words * 8) as i32);
                }
                if !aggregate {
                    self.store_place(destination)?;
                }
                self.edge(&[0xe9], *next);
            }
            cfg::Terminator::Return => {
                self.load_place(&ir::Place {
                    local: self.function.return_local,
                    projection: Vec::new(),
                })?;
                let at = self.jump(&[0xe9]);
                self.returns.push(at);
            }
            cfg::Terminator::Unreachable => self.bytes(&[0x0f, 0x0b]),
        }
        Ok(())
    }
}

/// Trusted, unchecked code-generation entry point for an immutable CFG.
/// The caller must obtain a successful `ownership::analyze` result for this
/// exact CFG and leave it unchanged afterward. No structural or ownership
/// validation is repeated here. Graph callers satisfy this through fact 109,
/// while standalone checked-body callers should use `emit_checked_function`.
/// Reference calls additionally require whole-program noescape and exact callee
/// signature evidence. Reference returns require the exact canonical RefContract
/// and successful read-phase return-provenance/region verification as part of
/// that analysis. Mutable returns additionally consume the unique handle with
/// Move. The pointer load below is physical transport, not semantic Copy.
/// Projected returns require exact external-root plus typed suffix membership
/// and candidate-to-actual-origin correlation, never layout offsets in summaries.
/// Local analysis alone cannot establish the whole-program call contract.
/// Every local and lowering temporary has stable invocation-owned frame storage.
pub fn emit_cfg_function(function: &cfg::Function) -> Result<Object, String> {
    if function.params.len() > MAX_ARGS {
        return Err(format!("at most {MAX_ARGS} parameters are supported"));
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
    let aggregate_result = indirect(&function.result);
    let result_size = value_size(&function.result)?;
    let mut emitter = Emitter {
        object: Object {
            code: Vec::new(),
            calls: Vec::new(),
            strings: Vec::new(),
        },
        function,
        locals,
        aggregate_scratch,
        returns: Vec::new(),
        traps: Vec::new(),
        edges: Vec::new(),
    };
    emitter.bytes(&[0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec]);
    emitter.i32(frame_bytes);
    for (index, local) in function.params.iter().enumerate() {
        let offset =
            16 + usize::from(aggregate_result) * 8 + (function.params.len() - index - 1) * 8;
        emitter.bytes(&[0x48, 0x8b, 0x85]);
        emitter.i32(offset as i32);
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
    let trap = emitter.position();
    emitter.bytes(&[
        0xbf, 101, 0, 0, 0, 0xb8, 60, 0, 0, 0, 0x0f, 0x05, 0x0f, 0x0b,
    ]);
    for at in std::mem::take(&mut emitter.traps) {
        emitter.patch(at, trap)?;
    }
    Ok(emitter.object)
}

/// Compatibility entry point for direct checked-body callers, lowering and
/// analyzing once. Graph compilation owns both facts and calls the
/// raw emit_cfg_function without lowering or analyzing again.
pub fn emit_checked_function(function: &ir::CheckedFunction) -> Result<Object, String> {
    let lowered = cfg::lower(function)?;
    if lowered.blocks.iter().any(|block| {
        matches!(&block.terminator.value, cfg::Terminator::Call { signature, .. }
            if signature.params.iter().any(|ty| matches!(ty, ir::Ty::Ref { .. }))
                || matches!(signature.result, ir::Ty::Ref { .. }))
    }) {
        return Err(
            "reference calls require whole-program noescape and callee-signature evidence".into(),
        );
    }
    let analysis = ownership::analyze(&lowered);
    if !analysis.passed() {
        return Err(format!(
            "{}: initialization/move analysis (complete={}): {:?}",
            lowered.name, analysis.complete, analysis.diagnostics
        ));
    }
    emit_cfg_function(&lowered)
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

/// Compatibility entry point for AST callers. Build signatures once and lower
/// each body once. Analyze every body before any raw emission, establishing the
/// whole-program noescape evidence required for reference calls.
pub fn emit(program: &Program) -> Result<Vec<u8>, String> {
    let signatures = frontend::signature_table(program)?;
    let functions = program
        .functions
        .iter()
        .enumerate()
        .map(|(index, f)| {
            let checked = frontend::type_function_with_signatures(program, index, &signatures)?;
            let lowered = cfg::lower(&checked)?;
            let analysis = ownership::analyze(&lowered);
            if !analysis.passed() {
                return Err(format!(
                    "{}: initialization/move analysis (complete={}): {:?}",
                    lowered.name, analysis.complete, analysis.diagnostics
                ));
            }
            Ok((f.name.clone(), lowered))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let objects = functions
        .iter()
        .map(|(name, function)| Ok((name.clone(), emit_cfg_function(function)?)))
        .collect::<Result<Vec<_>, String>>()?;
    link(&objects)
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::super::frontend::{Block, Expr, Function, Stmt, Type};
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::process::{Command, Output, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn block(statements: Vec<Stmt>, tail: Option<Expr>) -> Block {
        Block {
            statements,
            tail: tail.map(Box::new),
        }
    }
    fn function(name: &str, params: Vec<(String, Type)>, result: Type, body: Block) -> Function {
        Function {
            name: name.into(),
            regions: Default::default(),
            params,
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
    fn var(name: &str) -> Expr {
        Expr::Var(name.into())
    }
    fn call(name: &str, args: Vec<Expr>) -> Expr {
        Expr::Call {
            name: name.into(),
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
    fn program(statements: Vec<Stmt>, mut others: Vec<Function>) -> Program {
        others.push(function(
            "main",
            vec![],
            Type::Unit,
            block(statements, None),
        ));
        Program {
            functions: others,
            structs: vec![],
            enums: vec![],
        }
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
        assert!(emit_checked_function(&target).is_ok());
        let main = frontend::type_function_with_signatures(&p, 1, &env).unwrap();
        assert!(
            emit_checked_function(&main)
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
        assert!(emit_checked_function(&identity).is_ok());
        let main = frontend::type_function_with_signatures(&p, 1, &env).unwrap();
        assert!(
            emit_checked_function(&main)
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
        assert!(emit_checked_function(&identity).is_ok());
        let main = frontend::type_function_with_signatures(&p, 1, &env).unwrap();
        assert!(
            emit_checked_function(&main)
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
        assert!(emit_checked_function(&read).is_ok());
        let main = frontend::type_function_with_signatures(&p, 1, &env).unwrap();
        assert!(
            emit_checked_function(&main)
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
        let lowered = cfg::lower(&checked).unwrap();
        assert!(!ownership::analyze(&lowered).passed());
        assert!(emit_checked_function(&checked).is_err());
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
        let function = cfg::Function {
            ref_contract: Default::default(),
            loan_sites: vec![],
            name: "main".into(),
            params: vec![],
            result: ir::Ty::Unit,
            locals: vec![],
            blocks: vec![],
            entry: cfg::BlockId(0),
            return_local: LocalId(0),
        };
        // The initialized dummy is a private ABI invariant, not observable
        // through source indexing because an empty array has no valid index.
        let mut emitter = Emitter {
            object: Object {
                code: vec![],
                calls: vec![],
                strings: vec![],
            },
            function: &function,
            locals: vec![],
            aggregate_scratch: -8,
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
        assert!(indirect(&empty));
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
        let object = emit_checked_function(&checked).unwrap();
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
        let ty = ir::EnumTy::new(ir::EnumDef {
            id: ir::DefId {
                snapshot: ir::SnapshotId::allocate().unwrap(),
                index: 0,
            },
            name: "Singleton".into(),
            module: String::new(),
            public: false,
            variants: vec![ir::VariantDef {
                name: "Only".into(),
                shape: ir::VariantShape::Tuple,
                fields: vec![ir::FieldDef {
                    name: "0".into(),
                    ty: ir::Ty::Int(ir::IntTy::I32),
                    public: true,
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
            let mut branch = Emitter {
                object: Object {
                    code: vec![],
                    calls: vec![],
                    strings: vec![],
                },
                function: &function,
                locals: vec![],
                aggregate_scratch: 0,
                returns: vec![],
                traps: vec![],
                edges: vec![],
            };
            branch
                .terminator(&function.blocks[0].terminator.value)
                .unwrap();
            assert_eq!(branch.object.code, [0xe9, 0, 0, 0, 0]);
            assert_eq!(branch.edges, [(1, cfg::BlockId(1))]);
            let object = emit_cfg_function(&function).unwrap();
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
        let field = |name: &str| ir::FieldDef {
            name: name.into(),
            ty: ir::Ty::Int(ir::IntTy::I32),
            public: true,
        };
        let ty = ir::EnumTy::new(ir::EnumDef {
            id: ir::DefId {
                snapshot: ir::SnapshotId::allocate().unwrap(),
                index: 0,
            },
            name: "Backing".into(),
            module: String::new(),
            public: false,
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
        assert!(indirect(&ir::Ty::Enum(ty.clone())));
        assert_eq!(value_size(&ir::Ty::Enum(ty.clone())).unwrap(), 32);
        let function = cfg::Function {
            ref_contract: Default::default(),
            loan_sites: vec![],
            name: "main".into(),
            params: vec![],
            result: ir::Ty::Unit,
            locals: vec![],
            blocks: vec![],
            entry: cfg::BlockId(0),
            return_local: LocalId(0),
        };
        // This test exercises the private constructor emitter directly so it
        // can inspect inactive physical storage, which source Rust cannot read.
        let mut emitter = Emitter {
            object: Object {
                code: vec![],
                calls: vec![],
                strings: vec![],
            },
            function: &function,
            locals: vec![],
            aggregate_scratch: -32,
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
        let factorial = function(
            "factorial",
            vec![("n".into(), Type::Int(ir::IntTy::I32))],
            Type::Int(ir::IntTy::I32),
            block(
                vec![],
                Some(Expr::If {
                    condition: Box::new(binary(BinaryOp::Le, var("n"), Expr::i32(1))),
                    then_block: block(vec![Stmt::Return(Some(Expr::i32(1)))], None),
                    else_block: block(
                        vec![],
                        Some(binary(
                            BinaryOp::Mul,
                            var("n"),
                            call(
                                "factorial",
                                vec![binary(BinaryOp::Sub, var("n"), Expr::i32(1))],
                            ),
                        )),
                    ),
                }),
            ),
        );
        let pick = function(
            "pick",
            (0..12)
                .map(|i| (format!("p{i}"), Type::Int(ir::IntTy::I32)))
                .collect(),
            Type::Int(ir::IntTy::I32),
            block(vec![], Some(binary(BinaryOp::Sub, var("p0"), var("p11")))),
        );
        let local = |name: &str, n| Stmt::Let {
            name: name.into(),
            mutable: true,
            ty: Type::Int(ir::IntTy::I32),
            value: Expr::i32(n),
        };
        let statements = vec![
            local("x", 3),
            equals(
                binary(
                    BinaryOp::Add,
                    Expr::i32(2),
                    Expr::Block(block(vec![local("x", 9)], Some(var("x")))),
                ),
                11,
            ),
            equals(var("x"), 3),
            Stmt::While {
                condition: binary(BinaryOp::Lt, var("x"), Expr::i32(6)),
                body: block(
                    vec![
                        Stmt::Assign {
                            name: "x".into(),
                            value: binary(BinaryOp::Add, var("x"), Expr::i32(1)),
                        },
                        local("temporary", 100),
                    ],
                    None,
                ),
            },
            equals(var("x"), 6),
            equals(call("factorial", vec![Expr::i32(6)]), 720),
            equals(call("pick", (0..12).map(Expr::i32).collect()), -11),
        ];
        let output = run(&program(statements, vec![factorial, pick]));
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
    fn argument_bound_and_early_return_with_temporaries() {
        let pick = function(
            "pick",
            (0..MAX_ARGS)
                .map(|i| (format!("p{i}"), Type::Int(ir::IntTy::I32)))
                .collect(),
            Type::Int(ir::IntTy::I32),
            block(vec![], Some(binary(BinaryOp::Sub, var("p0"), var("p4095")))),
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
        );
        let p = program(
            vec![
                equals(
                    call("pick", (0..MAX_ARGS as i32).map(Expr::i32).collect()),
                    -4095,
                ),
                equals(
                    binary(BinaryOp::Add, Expr::i32(1), call("early", vec![])),
                    8,
                ),
            ],
            vec![pick.clone(), early],
        );
        assert_eq!(run(&p).status.code(), Some(0));
        let mut too_many = pick;
        too_many
            .params
            .push(("extra".into(), Type::Int(ir::IntTy::I32)));
        assert!(
            emit(&program(vec![], vec![too_many]))
                .unwrap_err()
                .contains("4096")
        );
        assert!(
            emit(&program(
                vec![Stmt::Expr(call("pick", vec![Expr::i32(0); MAX_ARGS + 1]))],
                vec![p.functions[0].clone()]
            ))
            .unwrap_err()
            .contains("4096")
        );
    }

    #[test]
    fn unit_arguments_nested_calls_and_left_to_right_effects() {
        let unit = function(
            "unit",
            vec![],
            Type::Unit,
            block(vec![Stmt::Expr(Expr::Println("unit".into()))], None),
        );
        let identity = function(
            "identity",
            vec![("n".into(), Type::Int(ir::IntTy::I32))],
            Type::Int(ir::IntTy::I32),
            block(vec![], Some(var("n"))),
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
                    binary(BinaryOp::Mul, var("a"), Expr::i32(10)),
                    var("b"),
                )),
            ),
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
                )),
            ),
        );
        let effect = |label: &str, value| {
            call(
                "identity",
                vec![Expr::Block(block(
                    vec![Stmt::Expr(Expr::Println(label.into()))],
                    Some(Expr::i32(value)),
                ))],
            )
        };
        let p = program(
            vec![
                equals(
                    binary(
                        BinaryOp::Add,
                        call(
                            "combine",
                            vec![effect("first", 2), call("unit", vec![]), effect("last", 3)],
                        ),
                        call("identity", vec![Expr::i32(4)]),
                    ),
                    27,
                ),
                equals(
                    binary(BinaryOp::Mul, Expr::i32(2), call("early", vec![])),
                    18,
                ),
            ],
            vec![unit, identity, combine, early],
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
        let object = emit_checked_function(&checked).unwrap();
        assert!(link(&[("main".into(), object.clone()), ("main".into(), object)]).is_err());
        assert!(emit(&program(vec![Stmt::Expr(call("missing", vec![]))], vec![])).is_err());
    }

    #[test]
    fn checked_emission_uses_local_ids_not_display_names() {
        let p = program(
            vec![equals(call("shadow", vec![Expr::i32(17)]), 19)],
            vec![function(
                "shadow",
                vec![("x".into(), Type::Int(ir::IntTy::I32))],
                Type::Int(ir::IntTy::I32),
                block(
                    vec![Stmt::Let {
                        name: "x".into(),
                        mutable: false,
                        ty: Type::Int(ir::IntTy::I32),
                        value: binary(BinaryOp::Add, var("x"), Expr::i32(2)),
                    }],
                    Some(var("x")),
                ),
            )],
        );
        assert_eq!(run(&p).status.code(), Some(0));
        let signatures = frontend::signature_table(&p).unwrap();
        let mut checked = frontend::type_function_with_signatures(&p, 0, &signatures).unwrap();
        let original = emit_checked_function(&checked).unwrap();
        for local in &mut checked.locals {
            local.name = "not_a_source_binding".into();
        }
        let renamed = emit_checked_function(&checked).unwrap();
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
        let index = p.functions.iter().position(|f| f.name == "make").unwrap();
        let checked = frontend::type_function_with_signatures(&p, index, &signatures).unwrap();
        let object = emit_checked_function(&checked).unwrap();
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
        assert!(
            emit_cfg_function(&cfg::lower(&checked).unwrap())
                .unwrap_err()
                .contains("limit")
        );
    }

    #[test]
    fn cfg_entry_edges_and_aggregate_assignment_snapshot_reads() {
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
        let object = emit_cfg_function(&function).unwrap();
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
        assert_eq!(emit_cfg_function(&function).unwrap().code, object.code);
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
        let index = p.functions.iter().position(|f| f.name == "one").unwrap();
        let checked = frontend::type_function_with_signatures(&p, index, &env).unwrap();
        assert_eq!(checked.result.field_count(), Some(1));
        assert_eq!(value_size(&checked.result).unwrap(), 8);
        let object = emit_checked_function(&checked).unwrap();
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
        let index = p.functions.iter().position(|f| f.name == "main").unwrap();
        let checked = frontend::type_function_with_signatures(&p, index, &env).unwrap();
        let error = emit_checked_function(&checked).unwrap_err();
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
            let index = p.functions.iter().position(|f| f.name == "main").unwrap();
            let checked = frontend::type_function_with_signatures(&p, index, &env).unwrap();
            let error = emit_checked_function(&checked).unwrap_err();
            assert!(error.contains("initialization/move analysis"), "{error}");
            assert!(
                emit(&p)
                    .unwrap_err()
                    .contains("initialization/move analysis")
            );
        }
    }
}
