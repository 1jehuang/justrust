//! WebAssembly's 128-bit vectors in code generation: the `simd128` and relaxed-simd features.
//!
//! A SIMD value lives in memory as everywhere else. Where the function may use `simd128` (its
//! crate is built with the feature, or the function enables it with `#[target_feature]`), a
//! vector operation on lanes that fill whole vectors of 16 bytes loads each 16 bytes as a
//! Cranelift vector, does the one operation on it, and stores it: [`super::wasm`] lowers that
//! to a `v128.load`, the instruction of the operation, and a `v128.store`. So do the
//! `llvm.wasm.*` intrinsics `core::arch::wasm32` declares, and the vector `llvm.*` ones it
//! declares for wasm. What has no vector instruction here (a shift by a vector of amounts that
//! is not one amount, a lane cast wasm has no instruction for) is done a lane at a time, as on
//! a function without the feature, where no vector type is made at all.
//!
//! An instruction Cranelift has no opcode for is a call of its name in
//! [`super::wasm::INSTRUCTION`]. A relaxed instruction is emitted as itself, so it behaves as
//! the engine that runs it chooses, the way rustc's does.

use cranelift_codegen::ir::{
    self, AbiParam, Endianness, InstBuilder, MemFlagsData, Type, Value,
    condcodes::{FloatCC, IntCC},
    types,
};
use cranelift_frontend::FunctionBuilder;
use hir_def::{CallableDefId, attrs::AttrFlags, signatures::FunctionSignature};
use hir_ty::{
    mir::{LocalId, Operand, OperandKind, Rvalue, StatementKind, TerminatorKind},
    next_solver::{ConstKind, Ty, TyKind},
};
use rustc_type_ir::inherent::IntoKind;

use super::wasm::{self, Instruction};
use super::{AbiScalar, CodegenError, FunctionCx, MEM, Result, return_slot};

/// The lanes of a SIMD type whose bytes are a whole number of 16-byte vectors, or 8 bytes.
#[derive(Clone, Copy)]
struct Lanes {
    lane: AbiScalar,
    count: u64,
}

impl Lanes {
    fn bytes(self) -> u64 {
        u64::from(self.lane.ty.bytes()) * self.count
    }

    /// The Cranelift type of 16 bytes of these lanes.
    fn vector(self) -> Type {
        self.lane.ty.by(16 / self.lane.ty.bytes()).expect("a lane of at most 16 bytes")
    }

    /// How many vectors of 16 bytes they are.
    fn chunks(self) -> Option<i32> {
        let bytes = self.bytes();
        (bytes % 16 == 0 && bytes != 0).then(|| (bytes / 16) as i32)
    }
}

/// How a vector of another type is read as these bytes: as they are in memory.
const LITTLE: MemFlagsData = MemFlagsData::new().with_endianness(Endianness::Little);

impl<'db> FunctionCx<'_, 'db> {
    /// Whether the function is compiled with the wasm feature `feature`: by its crate, or by
    /// its own `#[target_feature(enable = ...)]`.
    fn wasm_feature(&self, feature: &str) -> bool {
        if !self.cg.target.is_wasm() {
            return false;
        }
        let atom = cfg::CfgAtom::KeyValue {
            key: intern::sym::target_feature,
            value: intern::Symbol::intern(feature),
        };
        if self.env.krate.cfg_options(self.cg.db).check_atom(&atom) {
            return true;
        }
        self.body.and_then(|body| body.owner.as_function()).is_some_and(|func| {
            AttrFlags::target_features(self.cg.db, func).iter().any(|it| it.as_str() == feature)
        })
    }

    /// Whether the function may use wasm's `v128`: `relaxed-simd` implies `simd128`, as rustc
    /// has it.
    pub(super) fn simd128(&self) -> bool {
        self.wasm_feature("simd128") || self.wasm_feature("relaxed-simd")
    }

    /// Whether code generation may make a vector type: always but on wasm without `simd128`.
    pub(super) fn makes_vectors(&self) -> bool {
        !self.cg.target.is_wasm() || self.simd128()
    }

    fn simd_shape(&self, ty: Ty<'db>) -> Result<Option<Lanes>> {
        let (lane, count) = self.simd_lanes(ty)?;
        if lane.ty.bits() > 64 || u64::from(lane.ty.bytes()) * count > 64 * 16 {
            return Ok(None);
        }
        Ok(Some(Lanes { lane, count }))
    }

    fn vbitcast(&mut self, ty: Type, value: Value) -> Value {
        if self.builder.func.dfg.value_type(value) == ty {
            return value;
        }
        self.builder.ins().bitcast(ty, LITTLE, value)
    }

    /// Loads the 8 bytes at `addr + offset` as the low half of a vector of type `ty`.
    fn load_half(&mut self, ty: Type, addr: Value, offset: i32) -> Value {
        let low = self.builder.ins().load(types::I64, MEM, addr, offset);
        let vector = self.builder.ins().scalar_to_vector(types::I64X2, low);
        self.vbitcast(ty, vector)
    }

    /// Stores the low 8 bytes of a vector at `addr + offset`.
    fn store_half(&mut self, value: Value, addr: Value, offset: i32) {
        let value = self.vbitcast(types::I64X2, value);
        let low = self.builder.ins().extractlane(value, 0);
        self.builder.ins().store(MEM, low, addr, offset);
    }

    /// Calls the wasm instruction `instruction`: its operands are `args`, and it leaves a value
    /// of `ret` when there is one.
    fn wasm_instruction(
        &mut self,
        instruction: Instruction,
        args: &[Value],
        ret: Option<Type>,
    ) -> Option<Value> {
        let mut sig = self.cg.make_signature();
        for &arg in args {
            sig.params.push(AbiParam::new(self.builder.func.dfg.value_type(arg)));
        }
        sig.returns.extend(ret.map(AbiParam::new));
        let signature = self.builder.import_signature(sig);
        let name = self.builder.func.declare_imported_user_function(ir::UserExternalName {
            namespace: wasm::INSTRUCTION,
            index: instruction.index(),
        });
        let callee = self.builder.import_function(ir::ExtFuncData {
            name: ir::ExternalName::user(name),
            signature,
            colocated: true,
            patchable: false,
        });
        let call = self.builder.ins().call(callee, args);
        self.builder.inst_results(call).first().copied()
    }

    /// The bytes of a constant operand, as `simd_shuffle` is given its indices.
    fn constant_bytes(&mut self, operand: &Operand) -> Result<Option<Vec<u8>>> {
        Ok(match &operand.kind {
            OperandKind::Constant { konst, ty } => match konst.as_ref().kind() {
                ConstKind::Value(value) => {
                    let blob = self.valtree_blob(ty.as_ref(), value.value)?;
                    (blob.relocs.is_empty() && blob.fn_relocs.is_empty()).then_some(blob.bytes)
                }
                ConstKind::Unevaluated(named) => Some(self.eval_const(named)?.memory.to_vec()),
                _ => None,
            },
            OperandKind::Allocation { allocation } => Some(allocation.as_ref().memory.to_vec()),
            OperandKind::Copy(place) | OperandKind::Move(place)
                if place.projection.as_slice().is_empty() =>
            {
                let Some(from) = self.only_assignment(place.local) else { return Ok(None) };
                self.constant_bytes(&from)?
            }
            _ => None,
        })
    }

    /// What a local is set to, when one statement of the body sets all of it and nothing else
    /// does: a const generic argument is moved to a local of its own first.
    fn only_assignment(&self, local: LocalId) -> Option<Operand> {
        let body = self.body?;
        let mut assigned = body.basic_blocks.iter().flat_map(|(_, block)| {
            block.statements.iter().filter_map(move |it| match &it.kind {
                StatementKind::Assign(to, rvalue) if to.local == local => Some((to, rvalue)),
                _ => None,
            })
        });
        let (Some((to, Rvalue::Use(from))), None) = (assigned.next(), assigned.next()) else {
            return None;
        };
        to.projection.as_slice().is_empty().then(|| from.clone())
    }

    /// Whether `operand` is a local that only one call sets, of `simd_splat` or of a function
    /// that only returns what `simd_splat` makes (`core::arch`'s `Simd::splat`): all its lanes
    /// one value, what `core::arch::wasm32` shifts each lane by.
    fn splatted(&mut self, operand: &Operand) -> Result<bool> {
        let (OperandKind::Copy(place) | OperandKind::Move(place)) = &operand.kind else {
            return Ok(false);
        };
        let Some(body) = self.body else { return Ok(false) };
        if !place.projection.as_slice().is_empty() {
            return Ok(false);
        }
        let mut callees = Vec::new();
        for (_, block) in body.basic_blocks.iter() {
            let assigns = block.statements.iter().any(
                |it| matches!(&it.kind, StatementKind::Assign(to, _) if to.local == place.local),
            );
            if assigns {
                return Ok(false);
            }
            if let Some(TerminatorKind::Call { func, destination, .. }) =
                block.terminator.as_ref().map(|it| &it.kind)
                && destination.local == place.local
            {
                if !destination.projection.as_slice().is_empty() {
                    return Ok(false);
                }
                callees.push(self.operand_ty(func)?);
            }
        }
        let [callee] = callees[..] else { return Ok(false) };
        let TyKind::FnDef(def, _) = callee.kind() else { return Ok(false) };
        let CallableDefId::FunctionId(callee) = def.0 else { return Ok(false) };
        if FunctionSignature::of(self.cg.db, callee).name.as_str() == "simd_splat" {
            return Ok(true);
        }
        Ok(self.only_splats(callee))
    }

    /// Whether the body of `func` makes its result with one call, of `simd_splat`, and does
    /// nothing else that is a call.
    fn only_splats(&self, func: hir_def::FunctionId) -> bool {
        let db = self.cg.db;
        let Ok(body) = db.mir_body(func.into()) else { return false };
        let mut calls = body.basic_blocks.iter().filter_map(|(_, block)| {
            match block.terminator.as_ref().map(|it| &it.kind) {
                Some(TerminatorKind::Call { func, destination, .. }) => Some((func, destination)),
                _ => None,
            }
        });
        let (Some((callee, destination)), None) = (calls.next(), calls.next()) else {
            return false;
        };
        if destination.local != return_slot() || !destination.projection.as_slice().is_empty() {
            return false;
        }
        let ty = match &callee.kind {
            OperandKind::Constant { ty, .. } => ty.as_ref(),
            OperandKind::Allocation { allocation } => allocation.as_ref().ty,
            _ => return false,
        };
        let TyKind::FnDef(def, _) = ty.kind() else { return false };
        let CallableDefId::FunctionId(callee) = def.0 else { return false };
        FunctionSignature::of(db, callee).name.as_str() == "simd_splat"
    }

    /// A `simd_*` intrinsic as vector instructions, when the function may use `simd128` and
    /// there is one for it. False when it is to be done a lane at a time.
    pub(super) fn wasm_simd_intrinsic(
        &mut self,
        name: &str,
        args: &[Operand],
        dst: Value,
        dst_ty: Ty<'db>,
    ) -> Result<bool> {
        if !self.cg.target.is_wasm() || !self.simd128() {
            return Ok(false);
        }
        match (name, args) {
            (
                "simd_add"
                | "simd_sub"
                | "simd_mul"
                | "simd_div"
                | "simd_and"
                | "simd_or"
                | "simd_xor"
                | "simd_eq"
                | "simd_ne"
                | "simd_lt"
                | "simd_le"
                | "simd_gt"
                | "simd_ge"
                | "simd_saturating_add"
                | "simd_saturating_sub"
                | "simd_shl"
                | "simd_shr",
                [l, r],
            ) => self.wasm_simd_binary(name, l, r, dst, dst_ty),
            (
                "simd_neg"
                | "simd_fabs"
                | "simd_fsqrt"
                | "simd_floor"
                | "simd_ceil"
                | "simd_trunc"
                | "simd_round_ties_even"
                | "simd_ctpop",
                [arg],
            ) => self.wasm_simd_unary(name, arg, dst, dst_ty),
            ("simd_select", [mask, on_true, on_false]) => {
                let mask_ty = self.operand_ty(mask)?;
                let (Some(mask_lanes), Some(lanes)) =
                    (self.simd_shape(mask_ty)?, self.simd_shape(dst_ty)?)
                else {
                    return Ok(false);
                };
                // A lane of a mask is all zeros or all ones, so the bits of the mask pick.
                let Some(chunks) = lanes.chunks() else { return Ok(false) };
                if mask_lanes.lane.ty.bits() != lanes.lane.ty.bits() {
                    return Ok(false);
                }
                let (mask, _) = self.operand(mask)?;
                let (on_true, _) = self.operand(on_true)?;
                let (on_false, _) = self.operand(on_false)?;
                let vector = lanes.vector();
                for chunk in 0..chunks {
                    let at = chunk * 16;
                    let [m, a, b] = [mask, on_true, on_false]
                        .map(|it| self.builder.ins().load(vector, MEM, it, at));
                    let value = self.builder.ins().bitselect(m, a, b);
                    self.builder.ins().store(MEM, value, dst, at);
                }
                Ok(true)
            }
            ("simd_splat", [arg]) => {
                let Some(lanes) = self.simd_shape(dst_ty)? else { return Ok(false) };
                let Some(chunks) = lanes.chunks() else { return Ok(false) };
                let (src, _) = self.operand(arg)?;
                let value = self.builder.ins().load(lanes.lane.ty, MEM, src, 0);
                let vector = self.builder.ins().splat(lanes.vector(), value);
                for chunk in 0..chunks {
                    self.builder.ins().store(MEM, vector, dst, chunk * 16);
                }
                Ok(true)
            }
            ("simd_shuffle", [first, second, indices]) => {
                self.wasm_simd_shuffle(first, second, indices, dst, dst_ty)
            }
            ("simd_cast" | "simd_as", [arg]) => self.wasm_simd_cast(arg, dst, dst_ty),
            ("simd_bitmask", [arg]) => {
                let ty = self.operand_ty(arg)?;
                let Some(lanes) = self.simd_shape(ty)? else { return Ok(false) };
                let Some(chunks) = lanes.chunks() else { return Ok(false) };
                let layout = self.layout(dst_ty)?;
                let hir_def::layout::BackendRepr::Scalar(_) = layout.backend_repr else {
                    return Ok(false);
                };
                if lanes.count > 64 || lanes.lane.ty.is_float() {
                    return Ok(false);
                }
                let to = self.scalar_of(dst_ty)?;
                let (src, _) = self.operand(arg)?;
                let vector = lanes.vector();
                // Bit `i` of the result is the top bit of lane `i`.
                let mut bits = self.builder.ins().iconst(types::I64, 0);
                for chunk in 0..chunks {
                    let value = self.builder.ins().load(vector, MEM, src, chunk * 16);
                    let top = self.builder.ins().vhigh_bits(types::I64, value);
                    let shift = i64::from(chunk) * i64::from(vector.lane_count());
                    let top = self.builder.ins().ishl_imm_u(top, shift);
                    bits = self.builder.ins().bor(bits, top);
                }
                let from = AbiScalar { ty: types::I64, signed: false };
                let value = self.int_cast(bits, from, to);
                self.builder.ins().store(MEM, value, dst, 0);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn wasm_simd_binary(
        &mut self,
        name: &str,
        l: &Operand,
        r: &Operand,
        dst: Value,
        dst_ty: Ty<'db>,
    ) -> Result<bool> {
        let ty = self.operand_ty(l)?;
        let (Some(lanes), Some(out)) = (self.simd_shape(ty)?, self.simd_shape(dst_ty)?) else {
            return Ok(false);
        };
        let Some(chunks) = lanes.chunks() else { return Ok(false) };
        if out.count != lanes.count || out.lane.ty.bits() != lanes.lane.ty.bits() {
            return Ok(false);
        }
        let (float, signed, bits) =
            (lanes.lane.ty.is_float(), lanes.lane.signed, lanes.lane.ty.bits());
        let supported = match name {
            "simd_div" => float,
            "simd_and" | "simd_or" | "simd_xor" => !float,
            "simd_saturating_add" | "simd_saturating_sub" => !float && bits <= 16,
            "simd_shl" | "simd_shr" => !float && self.splatted(r)?,
            _ => true,
        };
        if !supported {
            return Ok(false);
        }
        let (a, _) = self.operand(l)?;
        let (b, _) = self.operand(r)?;
        let vector = lanes.vector();
        // A shift moves each lane by the one amount all of them have.
        let amount = matches!(name, "simd_shl" | "simd_shr")
            .then(|| self.builder.ins().load(lanes.lane.ty, MEM, b, 0));
        let int_cc = |name: &str| match (name, signed) {
            ("simd_eq", _) => IntCC::Equal,
            ("simd_ne", _) => IntCC::NotEqual,
            ("simd_lt", true) => IntCC::SignedLessThan,
            ("simd_lt", false) => IntCC::UnsignedLessThan,
            ("simd_le", true) => IntCC::SignedLessThanOrEqual,
            ("simd_le", false) => IntCC::UnsignedLessThanOrEqual,
            ("simd_gt", true) => IntCC::SignedGreaterThan,
            ("simd_gt", false) => IntCC::UnsignedGreaterThan,
            ("simd_ge", true) => IntCC::SignedGreaterThanOrEqual,
            _ => IntCC::UnsignedGreaterThanOrEqual,
        };
        let float_cc = |name: &str| match name {
            "simd_eq" => FloatCC::Equal,
            "simd_ne" => FloatCC::NotEqual,
            "simd_lt" => FloatCC::LessThan,
            "simd_le" => FloatCC::LessThanOrEqual,
            "simd_gt" => FloatCC::GreaterThan,
            _ => FloatCC::GreaterThanOrEqual,
        };
        for chunk in 0..chunks {
            let at = chunk * 16;
            let x = self.builder.ins().load(vector, MEM, a, at);
            let ins = self.builder.ins();
            let value = match name {
                "simd_shl" => ins.ishl(x, amount.expect("an amount")),
                "simd_shr" if signed => ins.sshr(x, amount.expect("an amount")),
                "simd_shr" => ins.ushr(x, amount.expect("an amount")),
                _ => {
                    let y = self.builder.ins().load(vector, MEM, b, at);
                    let ins = self.builder.ins();
                    match name {
                        "simd_add" if float => ins.fadd(x, y),
                        "simd_add" => ins.iadd(x, y),
                        "simd_sub" if float => ins.fsub(x, y),
                        "simd_sub" => ins.isub(x, y),
                        "simd_mul" if float => ins.fmul(x, y),
                        "simd_mul" => ins.imul(x, y),
                        "simd_div" => ins.fdiv(x, y),
                        "simd_and" => ins.band(x, y),
                        "simd_or" => ins.bor(x, y),
                        "simd_xor" => ins.bxor(x, y),
                        "simd_saturating_add" if signed => ins.sadd_sat(x, y),
                        "simd_saturating_add" => ins.uadd_sat(x, y),
                        "simd_saturating_sub" if signed => ins.ssub_sat(x, y),
                        "simd_saturating_sub" => ins.usub_sat(x, y),
                        // A comparison: all ones in the lanes where it holds.
                        _ if float => ins.fcmp(float_cc(name), x, y),
                        _ => ins.icmp(int_cc(name), x, y),
                    }
                }
            };
            self.builder.ins().store(MEM, value, dst, at);
        }
        Ok(true)
    }

    fn wasm_simd_unary(
        &mut self,
        name: &str,
        arg: &Operand,
        dst: Value,
        dst_ty: Ty<'db>,
    ) -> Result<bool> {
        let ty = self.operand_ty(arg)?;
        let (Some(lanes), Some(out)) = (self.simd_shape(ty)?, self.simd_shape(dst_ty)?) else {
            return Ok(false);
        };
        let Some(chunks) = lanes.chunks() else { return Ok(false) };
        let float = lanes.lane.ty.is_float();
        let supported = match name {
            "simd_neg" => true,
            "simd_ctpop" => lanes.lane.ty == types::I8,
            _ => float,
        };
        if !supported || out.lane.ty != lanes.lane.ty || out.count != lanes.count {
            return Ok(false);
        }
        let (src, _) = self.operand(arg)?;
        let vector = lanes.vector();
        for chunk in 0..chunks {
            let at = chunk * 16;
            let x = self.builder.ins().load(vector, MEM, src, at);
            let ins = self.builder.ins();
            let value = match name {
                "simd_neg" if float => ins.fneg(x),
                "simd_neg" => ins.ineg(x),
                "simd_fabs" => ins.fabs(x),
                "simd_fsqrt" => ins.sqrt(x),
                "simd_floor" => ins.floor(x),
                "simd_ceil" => ins.ceil(x),
                "simd_trunc" => ins.trunc(x),
                "simd_round_ties_even" => ins.nearest(x),
                _ => ins.popcnt(x),
            };
            self.builder.ins().store(MEM, value, dst, at);
        }
        Ok(true)
    }

    /// `simd_shuffle` with constant indices: each 16 bytes of the result one `i8x16.shuffle`
    /// of at most two vectors of the operands.
    fn wasm_simd_shuffle(
        &mut self,
        first: &Operand,
        second: &Operand,
        indices: &Operand,
        dst: Value,
        dst_ty: Ty<'db>,
    ) -> Result<bool> {
        let ty = self.operand_ty(first)?;
        let (Some(lanes), Some(out)) = (self.simd_shape(ty)?, self.simd_shape(dst_ty)?) else {
            return Ok(false);
        };
        if lanes.lane.ty.bytes() != out.lane.ty.bytes() {
            return Ok(false);
        }
        let Some(bytes) = self.constant_bytes(indices)? else { return Ok(false) };
        let count = out.count as usize;
        if bytes.len() < count * 4 {
            return Ok(false);
        }
        let indices: Vec<u64> = bytes
            .chunks_exact(4)
            .take(count)
            .map(|it| u64::from(u32::from_le_bytes(it.try_into().expect("four bytes"))))
            .collect();
        // The operands in pieces a vector each: 16 bytes, or 8 that are the low half of one.
        let piece = match lanes.bytes() {
            8 => 8,
            bytes if bytes % 16 == 0 => 16,
            _ => return Ok(false),
        };
        let per_operand = lanes.bytes() / piece;
        let (out_bytes, out_piece) = match out.bytes() {
            8 => (8, 8),
            bytes if bytes % 16 == 0 => (bytes, 16),
            _ => return Ok(false),
        };
        let lane = u64::from(lanes.lane.ty.bytes());
        if indices.iter().any(|&it| it >= 2 * lanes.count) {
            return Ok(false);
        }
        // For each piece of the result, the pieces of the operands it takes bytes of and the
        // byte of those each of its bytes is.
        let mut plans = Vec::new();
        for start in (0..out_bytes).step_by(out_piece as usize) {
            let mut sources: Vec<u64> = Vec::new();
            let mut mask = [0u8; 16];
            for (at, byte) in mask.iter_mut().enumerate().take(out_piece as usize) {
                let at = start + at as u64;
                let from = indices[(at / lane) as usize] * lane + at % lane;
                let source = from / piece;
                let slot = match sources.iter().position(|&it| it == source) {
                    Some(slot) => slot,
                    None => {
                        sources.push(source);
                        sources.len() - 1
                    }
                };
                if slot > 1 {
                    return Ok(false);
                }
                *byte = (slot * 16) as u8 + (from % piece) as u8;
            }
            plans.push((start, sources, mask));
        }
        let (first, _) = self.operand(first)?;
        let (second, _) = self.operand(second)?;
        for (start, sources, mask) in plans {
            let mut loaded = Vec::new();
            for &source in &sources {
                let operand = if source < per_operand { first } else { second };
                let at = ((source % per_operand) * piece) as i32;
                loaded.push(if piece == 16 {
                    self.builder.ins().load(types::I8X16, MEM, operand, at)
                } else {
                    self.load_half(types::I8X16, operand, at)
                });
            }
            let (a, b) = (loaded[0], *loaded.get(1).unwrap_or(&loaded[0]));
            let mask = self.builder.func.dfg.immediates.push(mask.to_vec().into());
            let value = self.builder.ins().shuffle(a, b, mask);
            if out_piece == 16 {
                self.builder.ins().store(MEM, value, dst, start as i32);
            } else {
                self.store_half(value, dst, start as i32);
            }
        }
        Ok(true)
    }

    /// `simd_cast` and `simd_as` that one or two wasm instructions do for each vector.
    fn wasm_simd_cast(&mut self, arg: &Operand, dst: Value, dst_ty: Ty<'db>) -> Result<bool> {
        let ty = self.operand_ty(arg)?;
        let (Some(from), Some(to)) = (self.simd_shape(ty)?, self.simd_shape(dst_ty)?) else {
            return Ok(false);
        };
        if from.count != to.count {
            return Ok(false);
        }
        let (f, t) = (from.lane, to.lane);
        let (src, _) = self.operand(arg)?;
        let (from_bytes, to_bytes) = (from.bytes(), to.bytes());
        match (f.ty.is_float(), t.ty.is_float()) {
            // The same bits, or a sign that is read another way.
            (false, false) if f.ty == t.ty => {
                let size = self.layout(dst_ty)?.size;
                self.copy(dst, src, size);
            }
            // Twice as wide: each lane extended by the sign it has.
            (false, false) if t.ty.bits() == 2 * f.ty.bits() && from_bytes == 8 => {
                let ins = self.builder.ins();
                let value = match (f.ty, f.signed) {
                    (types::I8, true) => ins.sload8x8(MEM, src, 0),
                    (types::I8, false) => ins.uload8x8(MEM, src, 0),
                    (types::I16, true) => ins.sload16x4(MEM, src, 0),
                    (types::I16, false) => ins.uload16x4(MEM, src, 0),
                    (_, true) => ins.sload32x2(MEM, src, 0),
                    (_, false) => ins.uload32x2(MEM, src, 0),
                };
                self.builder.ins().store(MEM, value, dst, 0);
            }
            (false, false) if t.ty.bits() == 2 * f.ty.bits() && from_bytes % 16 == 0 => {
                let vector = from.vector();
                for chunk in 0..(from_bytes / 16) as i32 {
                    let x = self.builder.ins().load(vector, MEM, src, chunk * 16);
                    let (low, high) = if f.signed {
                        (self.builder.ins().swiden_low(x), self.builder.ins().swiden_high(x))
                    } else {
                        (self.builder.ins().uwiden_low(x), self.builder.ins().uwiden_high(x))
                    };
                    self.builder.ins().store(MEM, low, dst, chunk * 32);
                    self.builder.ins().store(MEM, high, dst, chunk * 32 + 16);
                }
            }
            // Half as wide: the low half of each lane.
            (false, false)
                if 2 * t.ty.bits() == f.ty.bits() && (to_bytes == 8 || to_bytes % 16 == 0) =>
            {
                let (fb, tb) = (f.ty.bytes() as u8, t.ty.bytes() as u8);
                let mut mask = [0u8; 16];
                for (at, byte) in mask.iter_mut().enumerate() {
                    let at = at as u8;
                    *byte = (at / tb) * fb + at % tb;
                }
                let mask = self.builder.func.dfg.immediates.push(mask.to_vec().into());
                if to_bytes == 8 {
                    let x = self.builder.ins().load(types::I8X16, MEM, src, 0);
                    let value = self.builder.ins().shuffle(x, x, mask);
                    self.store_half(value, dst, 0);
                } else {
                    for chunk in 0..(to_bytes / 16) as i32 {
                        let x = self.builder.ins().load(types::I8X16, MEM, src, chunk * 32);
                        let y = self.builder.ins().load(types::I8X16, MEM, src, chunk * 32 + 16);
                        let value = self.builder.ins().shuffle(x, y, mask);
                        self.builder.ins().store(MEM, value, dst, chunk * 16);
                    }
                }
            }
            (false, true) if (f.ty, t.ty) == (types::I32, types::F32) && from_bytes % 16 == 0 => {
                for chunk in 0..(from_bytes / 16) as i32 {
                    let x = self.builder.ins().load(types::I32X4, MEM, src, chunk * 16);
                    let ins = self.builder.ins();
                    let value = if f.signed {
                        ins.fcvt_from_sint(types::F32X4, x)
                    } else {
                        ins.fcvt_from_uint(types::F32X4, x)
                    };
                    self.builder.ins().store(MEM, value, dst, chunk * 16);
                }
            }
            // Each 8 bytes of the operand the low half of a vector, widened to a whole one.
            (false, true) if (f.ty, t.ty) == (types::I32, types::F64) && from_bytes % 8 == 0 => {
                let instruction = if f.signed {
                    Instruction::F64x2ConvertLowI32x4S
                } else {
                    Instruction::F64x2ConvertLowI32x4U
                };
                for half in 0..(from_bytes / 8) as i32 {
                    let x = self.load_half(types::I32X4, src, half * 8);
                    let value = self.wasm_instruction(instruction, &[x], Some(types::F64X2));
                    self.builder.ins().store(MEM, value.expect("a result"), dst, half * 16);
                }
            }
            // Saturated, as `simd_as` is and `simd_cast` may be.
            (true, false) if (f.ty, t.ty) == (types::F32, types::I32) && from_bytes % 16 == 0 => {
                for chunk in 0..(from_bytes / 16) as i32 {
                    let x = self.builder.ins().load(types::F32X4, MEM, src, chunk * 16);
                    let ins = self.builder.ins();
                    let value = if t.signed {
                        ins.fcvt_to_sint_sat(types::I32X4, x)
                    } else {
                        ins.fcvt_to_uint_sat(types::I32X4, x)
                    };
                    self.builder.ins().store(MEM, value, dst, chunk * 16);
                }
            }
            // Each 16 bytes of the operand narrowed to 8 of the result.
            (true, false) if (f.ty, t.ty) == (types::F64, types::I32) && from_bytes % 16 == 0 => {
                let instruction = if t.signed {
                    Instruction::I32x4TruncSatF64x2SZero
                } else {
                    Instruction::I32x4TruncSatF64x2UZero
                };
                for chunk in 0..(from_bytes / 16) as i32 {
                    let x = self.builder.ins().load(types::F64X2, MEM, src, chunk * 16);
                    let value = self.wasm_instruction(instruction, &[x], Some(types::I32X4));
                    self.store_half(value.expect("a result"), dst, chunk * 8);
                }
            }
            (true, true) if (f.ty, t.ty) == (types::F32, types::F64) && from_bytes % 8 == 0 => {
                for half in 0..(from_bytes / 8) as i32 {
                    let x = self.load_half(types::F32X4, src, half * 8);
                    let value = self.builder.ins().fvpromote_low(x);
                    self.builder.ins().store(MEM, value, dst, half * 16);
                }
            }
            (true, true) if (f.ty, t.ty) == (types::F64, types::F32) && from_bytes % 16 == 0 => {
                for chunk in 0..(from_bytes / 16) as i32 {
                    let x = self.builder.ins().load(types::F64X2, MEM, src, chunk * 16);
                    let value = self.builder.ins().fvdemote(x);
                    self.store_half(value, dst, chunk * 8);
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// An intrinsic `core::arch::wasm32` declares as `llvm.{name}`, as the wasm instruction it
    /// stands for. False for one that is not wasm's.
    pub(super) fn wasm_llvm_intrinsic(
        &mut self,
        name: &str,
        args: &[Operand],
        dst: Value,
    ) -> Result<bool> {
        if !self.cg.target.is_wasm() {
            return Ok(false);
        }
        let (scalars, instruction) = match name {
            "wasm.memory.size.i32" => (1, Instruction::MemorySize),
            "wasm.memory.grow.i32" => (1, Instruction::MemoryGrow),
            "wasm.memory.atomic.wait32" => (0, Instruction::MemoryAtomicWait32),
            "wasm.memory.atomic.wait64" => (0, Instruction::MemoryAtomicWait64),
            "wasm.memory.atomic.notify" => (0, Instruction::MemoryAtomicNotify),
            _ => return self.wasm_llvm_vector(name, args, dst),
        };
        // The memory instructions name the first memory, the only one there is.
        let (memory, args) = args.split_at(scalars);
        if let [memory] = memory
            && self.immediate(memory)? != Some(0)
        {
            unsupported!("`llvm.{name}` of a memory other than the first");
        }
        let operands =
            args.iter().map(|it| Ok(self.load_scalar(it)?.0)).collect::<Result<Vec<_>>>()?;
        let value = self.wasm_instruction(instruction, &operands, Some(types::I32));
        self.builder.ins().store(MEM, value.expect("a result"), dst, 0);
        Ok(true)
    }

    fn wasm_llvm_vector(&mut self, name: &str, args: &[Operand], dst: Value) -> Result<bool> {
        let Some((operands, result, how)) = vector_intrinsic(name) else { return Ok(false) };
        if operands.len() != args.len() {
            return Ok(false);
        }
        let mut values = Vec::with_capacity(args.len());
        for (arg, &ty) in args.iter().zip(operands) {
            let (addr, _) = self.operand(arg)?;
            values.push(self.builder.ins().load(ty, MEM, addr, 0));
        }
        let value = match how {
            How::Opcode(emit) => emit(&mut self.builder, &values),
            How::Instruction(instruction) => {
                self.wasm_instruction(instruction, &values, Some(result)).expect("a result")
            }
        };
        self.builder.ins().store(MEM, value, dst, 0);
        Ok(true)
    }
}

/// How a vector intrinsic is computed from its operands.
enum How {
    Opcode(fn(&mut FunctionBuilder<'_>, &[Value]) -> Value),
    Instruction(Instruction),
}

/// What the operands of the vector intrinsic `llvm.{name}` of `core::arch::wasm32` are, what its
/// result is, and how it is computed.
fn vector_intrinsic(name: &str) -> Option<(&'static [Type], Type, How)> {
    use Instruction as W;
    use types::{F32X4, F64X2, I8X16, I16X8, I32, I32X4, I64X2};
    let (stem, suffix) = name.rsplit_once('.')?;
    let lanes = match suffix {
        "v16i8" => I8X16,
        "v8i16" => I16X8,
        "v4i32" => I32X4,
        "v2i64" => I64X2,
        "v4f32" => F32X4,
        "v2f64" => F64X2,
        _ => return untyped_vector_intrinsic(name),
    };
    let one: &'static [Type] = match lanes {
        I8X16 => &[I8X16],
        I16X8 => &[I16X8],
        I32X4 => &[I32X4],
        I64X2 => &[I64X2],
        F32X4 => &[F32X4],
        _ => &[F64X2],
    };
    let two: &'static [Type] = match lanes {
        I8X16 => &[I8X16; 2],
        I16X8 => &[I16X8; 2],
        F32X4 => &[F32X4; 2],
        F64X2 => &[F64X2; 2],
        _ => &[],
    };
    let three: &'static [Type] = match lanes {
        I8X16 => &[I8X16; 3],
        I16X8 => &[I16X8; 3],
        I32X4 => &[I32X4; 3],
        I64X2 => &[I64X2; 3],
        F32X4 => &[F32X4; 3],
        _ => &[F64X2; 3],
    };
    let float = lanes.lane_type().is_float();
    let f32 = lanes == F32X4;
    Some(match stem {
        "wasm.anytrue" if !float => (
            one,
            I32,
            How::Opcode(|b, v| {
                let any = b.ins().vany_true(v[0]);
                b.ins().uextend(I32, any)
            }),
        ),
        "wasm.alltrue" if !float => (
            one,
            I32,
            How::Opcode(|b, v| {
                let all = b.ins().vall_true(v[0]);
                b.ins().uextend(I32, all)
            }),
        ),
        "wasm.bitmask" if !float => (one, I32, How::Opcode(|b, v| b.ins().vhigh_bits(I32, v[0]))),
        "wasm.bitselect" if lanes == I8X16 => {
            (three, lanes, How::Opcode(|b, v| b.ins().bitselect(v[2], v[0], v[1])))
        }
        "wasm.avgr.unsigned" if matches!(lanes, I8X16 | I16X8) => {
            (two, lanes, How::Opcode(|b, v| b.ins().avg_round(v[0], v[1])))
        }
        // Named by the result: each lane the sum of two of the narrower lanes of the operand.
        "wasm.extadd.pairwise.signed" | "wasm.extadd.pairwise.unsigned"
            if matches!(lanes, I16X8 | I32X4) =>
        {
            let signed = stem.ends_with(".signed");
            let (operand, instruction): (&'static [Type], _) = match (lanes, signed) {
                (I16X8, true) => (&[I8X16], W::I16x8ExtaddPairwiseI8x16S),
                (I16X8, false) => (&[I8X16], W::I16x8ExtaddPairwiseI8x16U),
                (_, true) => (&[I16X8], W::I32x4ExtaddPairwiseI16x8S),
                (_, false) => (&[I16X8], W::I32x4ExtaddPairwiseI16x8U),
            };
            (operand, lanes, How::Instruction(instruction))
        }
        "nearbyint" if float => (one, lanes, How::Opcode(|b, v| b.ins().nearest(v[0]))),
        // `fmin` and `fmax` are wasm's: a NaN if either is one, and -0 below +0.
        "minimum" if float => (two, lanes, How::Opcode(|b, v| b.ins().fmin(v[0], v[1]))),
        "maximum" if float => (two, lanes, How::Opcode(|b, v| b.ins().fmax(v[0], v[1]))),
        "wasm.relaxed.madd" if float => (
            three,
            lanes,
            How::Instruction(if f32 { W::F32x4RelaxedMadd } else { W::F64x2RelaxedMadd }),
        ),
        "wasm.relaxed.nmadd" if float => (
            three,
            lanes,
            How::Instruction(if f32 { W::F32x4RelaxedNmadd } else { W::F64x2RelaxedNmadd }),
        ),
        "wasm.relaxed.min" if float => (
            two,
            lanes,
            How::Instruction(if f32 { W::F32x4RelaxedMin } else { W::F64x2RelaxedMin }),
        ),
        "wasm.relaxed.max" if float => (
            two,
            lanes,
            How::Instruction(if f32 { W::F32x4RelaxedMax } else { W::F64x2RelaxedMax }),
        ),
        "wasm.relaxed.laneselect" if !float => {
            let instruction = match lanes {
                I8X16 => W::I8x16RelaxedLaneselect,
                I16X8 => W::I16x8RelaxedLaneselect,
                I32X4 => W::I32x4RelaxedLaneselect,
                _ => W::I64x2RelaxedLaneselect,
            };
            (three, lanes, How::Instruction(instruction))
        }
        _ => return None,
    })
}

/// The vector intrinsics whose names say nothing of their lanes.
fn untyped_vector_intrinsic(name: &str) -> Option<(&'static [Type], Type, How)> {
    use Instruction as W;
    use types::{F32X4, F64X2, I8X16, I16X8, I32X4};
    Some(match name {
        "wasm.swizzle" => (&[I8X16; 2], I8X16, How::Opcode(|b, v| b.ins().swizzle(v[0], v[1]))),
        "wasm.q15mulr.sat.signed" => {
            (&[I16X8; 2], I16X8, How::Opcode(|b, v| b.ins().sqmul_round_sat(v[0], v[1])))
        }
        "wasm.dot" => (&[I16X8; 2], I32X4, How::Instruction(W::I32x4DotI16x8S)),
        "wasm.relaxed.swizzle" => (&[I8X16; 2], I8X16, How::Instruction(W::I8x16RelaxedSwizzle)),
        "wasm.relaxed.trunc.signed" => {
            (&[F32X4], I32X4, How::Instruction(W::I32x4RelaxedTruncF32x4S))
        }
        "wasm.relaxed.trunc.unsigned" => {
            (&[F32X4], I32X4, How::Instruction(W::I32x4RelaxedTruncF32x4U))
        }
        "wasm.relaxed.trunc.signed.zero" => {
            (&[F64X2], I32X4, How::Instruction(W::I32x4RelaxedTruncF64x2SZero))
        }
        "wasm.relaxed.trunc.unsigned.zero" => {
            (&[F64X2], I32X4, How::Instruction(W::I32x4RelaxedTruncF64x2UZero))
        }
        "wasm.relaxed.q15mulr.signed" => {
            (&[I16X8; 2], I16X8, How::Instruction(W::I16x8RelaxedQ15mulrS))
        }
        "wasm.relaxed.dot.i8x16.i7x16.signed" => {
            (&[I8X16; 2], I16X8, How::Instruction(W::I16x8RelaxedDotI8x16I7x16S))
        }
        "wasm.relaxed.dot.i8x16.i7x16.add.signed" => {
            (&[I8X16, I8X16, I32X4], I32X4, How::Instruction(W::I32x4RelaxedDotI8x16I7x16AddS))
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_vector_intrinsic_of_core_arch() {
        for name in [
            "minimum.v4f32",
            "maximum.v2f64",
            "nearbyint.v4f32",
            "wasm.bitmask.v16i8",
            "wasm.alltrue.v2i64",
            "wasm.anytrue.v16i8",
            "wasm.bitselect.v16i8",
            "wasm.avgr.unsigned.v8i16",
            "wasm.extadd.pairwise.signed.v8i16",
            "wasm.extadd.pairwise.unsigned.v4i32",
            "wasm.q15mulr.sat.signed",
            "wasm.dot",
            "wasm.swizzle",
            "wasm.relaxed.swizzle",
            "wasm.relaxed.madd.v4f32",
            "wasm.relaxed.nmadd.v2f64",
            "wasm.relaxed.min.v4f32",
            "wasm.relaxed.max.v2f64",
            "wasm.relaxed.laneselect.v2i64",
            "wasm.relaxed.trunc.signed",
            "wasm.relaxed.trunc.unsigned.zero",
            "wasm.relaxed.q15mulr.signed",
            "wasm.relaxed.dot.i8x16.i7x16.signed",
            "wasm.relaxed.dot.i8x16.i7x16.add.signed",
        ] {
            assert!(vector_intrinsic(name).is_some(), "{name}");
        }
    }
}
