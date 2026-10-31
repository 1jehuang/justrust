//! Stable, authority-free fingerprint of stack-emitter inputs.
//!
//! Eligibility is deliberately closed: proof-dependent operations must reach
//! the backend rather than finding a cache entry. This is not validation.
use super::{
    cfg,
    frontend::{BinaryOp, UnaryOp},
    ir, ssa,
};
use crate::persistent_cache::KeyBuilder;

enum Failure {
    Unsupported,
    Resource(String),
}
type Result<T> = std::result::Result<T, Failure>;
impl From<String> for Failure {
    fn from(value: String) -> Self {
        Self::Resource(value)
    }
}

struct Encoder<'a, 'b> {
    key: KeyBuilder,
    meter: &'a ssa::meter::Meter<'b>,
    function: &'a cfg::Function,
    resolve: &'a dyn Fn(&ir::CallTarget) -> std::result::Result<String, String>,
}
impl Encoder<'_, '_> {
    fn bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.meter.work(ssa::meter::add(bytes.len(), 8)?)?;
        self.key.field(bytes);
        Ok(())
    }
    fn n(&mut self, n: usize) -> Result<()> {
        self.bytes(&(n as u64).to_le_bytes())
    }
    fn flag(&mut self, flag: bool) -> Result<()> {
        self.bytes(&[u8::from(flag)])
    }
    fn kind(&mut self, kind: ir::BorrowKind) -> Result<()> {
        self.n(match kind {
            ir::BorrowKind::Shared => 0,
            ir::BorrowKind::Mut => 1,
        })
    }
    fn int(&mut self, ty: ir::IntTy) -> Result<()> {
        use ir::IntTy::*;
        self.n(match ty {
            I8 => 0,
            I16 => 1,
            I32 => 2,
            I64 => 3,
            Isize => 4,
            U8 => 5,
            U16 => 6,
            U32 => 7,
            U64 => 8,
            Usize => 9,
            I128 => 10,
            U128 => 11,
        })
    }
    fn float(&mut self, ty: ir::FloatTy) -> Result<()> {
        self.n(match ty {
            ir::FloatTy::F32 => 0,
            ir::FloatTy::F64 => 1,
        })
    }
    fn ty(&mut self, ty: &ir::Ty, depth: usize) -> Result<()> {
        if depth > 128 {
            return Err(Failure::Unsupported);
        }
        self.meter.work(1)?;
        use ir::Ty::*;
        match ty {
            Int(t) => {
                self.n(0)?;
                self.int(*t)
            }
            Char => self.n(1),
            Str => self.n(2),
            Float(t) => {
                self.n(3)?;
                self.float(*t)
            }
            Bool => self.n(4),
            Unit => self.n(5),
            Never => self.n(6),
            Ref { kind, referent } => {
                self.n(7)?;
                self.kind(*kind)?;
                self.ty(referent, depth + 1)
            }
            RawPtr { kind, pointee } => {
                self.n(8)?;
                self.kind(*kind)?;
                self.ty(pointee, depth + 1)
            }
            Tuple(fields) => {
                self.n(9)?;
                self.n(fields.len())?;
                for f in fields.iter() {
                    self.ty(f, depth + 1)?;
                }
                Ok(())
            }
            Array { element, len } => {
                self.n(10)?;
                self.n(*len)?;
                self.ty(element, depth + 1)
            }
            Slice { element } => {
                self.n(11)?;
                self.ty(element, depth + 1)
            }
            PhantomData(_) | Opaque(_) | Closure(_) | FnPointer(_) | FnItem(_)
            | AssociatedType(_) | Param(_) | Struct(_) | Enum(_) | Dyn(_) => {
                Err(Failure::Unsupported)
            }
        }
    }
    fn place(&mut self, p: &ir::Place) -> Result<()> {
        self.n(p.local.0)?;
        self.n(p.projection.len())?;
        for projection in &p.projection {
            match projection {
                ir::Projection::Field(i) => {
                    self.n(0)?;
                    self.n(*i)?;
                }
                ir::Projection::ConstantIndex(i) => {
                    self.n(1)?;
                    self.n(*i)?;
                }
                ir::Projection::DynamicIndex(i) => {
                    self.n(2)?;
                    self.n(i.0)?;
                }
                ir::Projection::Deref => self.n(3)?,
                ir::Projection::Downcast(_) => return Err(Failure::Unsupported),
            }
        }
        Ok(())
    }
    fn constant(&mut self, c: &cfg::Constant) -> Result<()> {
        match c {
            cfg::Constant::Int(v) => {
                self.n(0)?;
                self.int(v.ty)?;
                self.bytes(&v.bits.to_le_bytes())
            }
            cfg::Constant::Char(v) => {
                self.n(1)?;
                self.bytes(&v.value.to_le_bytes())
            }
            cfg::Constant::Float(v) => {
                self.n(2)?;
                self.float(v.ty)?;
                self.bytes(&v.bits.to_le_bytes())
            }
            cfg::Constant::Bool(v) => {
                self.n(3)?;
                self.flag(*v)
            }
            cfg::Constant::Unit => self.n(4),
            cfg::Constant::PhantomData(_)
            | cfg::Constant::AssociatedConst(_)
            | cfg::Constant::Enum { .. } => Err(Failure::Unsupported),
        }
    }
    fn operand(&mut self, o: &cfg::Operand) -> Result<()> {
        match o {
            cfg::Operand::Constant(c) => {
                self.n(0)?;
                self.constant(c)
            }
            cfg::Operand::Copy(p) => {
                self.n(1)?;
                self.place(p)
            }
            cfg::Operand::Move(p) => {
                self.n(2)?;
                self.place(p)
            }
        }
    }
    fn operands(&mut self, args: &[cfg::Operand]) -> Result<()> {
        self.n(args.len())?;
        for arg in args {
            self.operand(arg)?;
        }
        Ok(())
    }
    fn binary(&mut self, op: BinaryOp) -> Result<()> {
        use BinaryOp::*;
        self.n(match op {
            Add => 0,
            Sub => 1,
            Mul => 2,
            Div => 3,
            Rem => 4,
            Eq => 5,
            Ne => 6,
            Lt => 7,
            Le => 8,
            Gt => 9,
            Ge => 10,
            And => 11,
            Or => 12,
            BitAnd => 13,
            BitOr => 14,
            BitXor => 15,
            Shl => 16,
            Shr => 17,
        })
    }
    fn stream(&mut self, stream: ir::OutputStream) -> Result<()> {
        self.n(match stream {
            ir::OutputStream::Stdout => 0,
            ir::OutputStream::Stderr => 1,
            ir::OutputStream::PanicStderr => 2,
        })
    }
    fn rvalue(&mut self, v: &cfg::Rvalue) -> Result<()> {
        use cfg::Rvalue::*;
        match v {
            TypeAlignment(_)
            | OpaquePack { .. }
            | Intrinsic { .. }
            | EnumVariant { .. }
            | Discriminant(_)
            | RefToDyn { .. } => Err(Failure::Unsupported),
            NativeSyscall(args) => {
                self.n(0)?;
                self.operands(args)
            }
            RawSlice { kind, args } => {
                self.n(1)?;
                self.kind(*kind)?;
                self.operands(args)
            }
            SliceDataPointer { slice } => {
                self.n(2)?;
                self.operand(slice)
            }
            CharToInt { value, to } => {
                self.n(3)?;
                self.operand(value)?;
                self.int(*to)
            }
            U8ToChar { value } => {
                self.n(4)?;
                self.operand(value)
            }
            PtrCast { value, to } => {
                self.n(5)?;
                self.operand(value)?;
                self.ty(to, 0)
            }
            LeadingZeros { value } => {
                self.n(6)?;
                self.operand(value)
            }
            IntCast { value, to } => {
                self.n(7)?;
                self.operand(value)?;
                self.int(*to)
            }
            FloatCast { value, to } => {
                self.n(8)?;
                self.operand(value)?;
                match to {
                    ir::NumTy::Int(t) => {
                        self.n(0)?;
                        self.int(*t)
                    }
                    ir::NumTy::Float(t) => {
                        self.n(1)?;
                        self.float(*t)
                    }
                }
            }
            Borrow(id) => {
                self.n(9)?;
                let site = self
                    .function
                    .loan_sites
                    .get(id.0)
                    .ok_or(Failure::Unsupported)?;
                // Only the materialized address is consumed by the emitter.
                self.place(&site.place)
            }
            IndexCopy { array, index } => {
                self.n(10)?;
                self.place(array)?;
                self.operand(index)
            }
            Repeat { ty, value } => {
                self.n(11)?;
                self.ty(ty, 0)?;
                self.operand(value)
            }
            Use(o) => {
                self.n(12)?;
                self.operand(o)
            }
            Unary { op, value } => {
                self.n(13)?;
                self.n(match op {
                    UnaryOp::Neg => 0,
                    UnaryOp::Not => 1,
                })?;
                self.operand(value)
            }
            Binary { op, left, right } => {
                self.n(14)?;
                self.binary(*op)?;
                self.operand(left)?;
                self.operand(right)
            }
            Aggregate { ty, fields } => {
                self.n(15)?;
                self.ty(ty, 0)?;
                self.operands(fields)
            }
            Output(o) => {
                self.n(16)?;
                self.bytes(o.text.as_bytes())?;
                self.stream(o.stream)?;
                self.flag(o.newline)
            }
            Str(s) => {
                self.n(17)?;
                self.bytes(s.as_bytes())
            }
            ByteStr(s) => {
                self.n(18)?;
                self.bytes(s)
            }
            ByteRepeat { byte, len } => {
                self.n(19)?;
                self.bytes(&[*byte])?;
                self.n(*len)
            }
            PromotedScalar(s) => {
                self.n(20)?;
                match s {
                    ir::PromotedScalar::Int(v) => self.constant(&cfg::Constant::Int(*v)),
                    ir::PromotedScalar::Bool(v) => self.constant(&cfg::Constant::Bool(*v)),
                    ir::PromotedScalar::Char(v) => self.constant(&cfg::Constant::Char(*v)),
                }
            }
            Println(s) => {
                self.n(21)?;
                self.bytes(s.as_bytes())
            }
            Format {
                pieces,
                arguments,
                modes,
                stream,
                newline,
            } => {
                self.n(22)?;
                self.n(pieces.len())?;
                for p in pieces {
                    self.bytes(p.as_bytes())?;
                }
                self.operands(arguments)?;
                self.n(modes.len())?;
                for m in modes {
                    self.n(match m {
                        ir::FormatMode::Display => 0,
                        ir::FormatMode::Debug => 1,
                    })?;
                }
                self.stream(*stream)?;
                self.flag(*newline)
            }
            ArrayToSlice { reference } => {
                self.n(23)?;
                self.operand(reference)
            }
            SliceLen { slice } => {
                self.n(24)?;
                self.operand(slice)
            }
            SliceIndex { slice, index } => {
                self.n(25)?;
                self.operand(slice)?;
                self.operand(index)
            }
            Subslice {
                slice,
                start,
                end,
                inclusive,
            } => {
                self.n(26)?;
                self.operand(slice)?;
                self.operand(start)?;
                self.flag(end.is_some())?;
                if let Some(end) = end {
                    self.operand(end)?;
                }
                self.flag(*inclusive)
            }
        }
    }
    fn statement(&mut self, s: &cfg::Statement) -> Result<()> {
        use cfg::Statement::*;
        match s {
            IndexAssignOp {
                array,
                index,
                value,
                op,
            } => {
                self.n(0)?;
                self.place(array)?;
                self.operand(index)?;
                self.operand(value)?;
                self.binary(*op)
            }
            IndexAssign {
                array,
                index,
                value,
            } => {
                self.n(1)?;
                self.place(array)?;
                self.operand(index)?;
                self.operand(value)
            }
            IndexInspect { array, index } => {
                self.n(2)?;
                self.place(array)?;
                self.operand(index)
            }
            Inspect(p) => {
                self.n(3)?;
                self.place(p)
            }
            StorageLive(l) => {
                self.n(4)?;
                self.n(l.0)
            }
            StorageDead(l) => {
                self.n(5)?;
                self.n(l.0)
            }
            Assign { destination, value } => {
                self.n(6)?;
                self.place(destination)?;
                self.rvalue(value)
            }
        }
    }
    fn terminator(&mut self, t: &cfg::Terminator) -> Result<()> {
        use cfg::Terminator::*;
        match t {
            Drop { .. } | SwitchVariant { .. } => Err(Failure::Unsupported),
            Goto(b) => {
                self.n(0)?;
                self.n(b.0)
            }
            Switch {
                condition,
                then_block,
                else_block,
            } => {
                self.n(1)?;
                self.operand(condition)?;
                self.n(then_block.0)?;
                self.n(else_block.0)
            }
            Call {
                target,
                name: _,
                signature,
                args,
                destination,
                next,
            } => {
                self.n(2)?;
                let target = match target {
                    ir::Callee::Direct(t) => t,
                    ir::Callee::TraitItem(_) | ir::Callee::Virtual(_) | ir::Callee::Untupled(_) => {
                        return Err(Failure::Unsupported);
                    }
                };
                self.n(target.type_args.len())?;
                for ty in target.type_args.iter() {
                    self.ty(ty, 0)?;
                }
                self.meter.work(1)?;
                let symbol = (self.resolve)(target)?;
                self.bytes(symbol.as_bytes())?;
                let ir::Signature {
                    ref_contract: _, // Current graph authority, not machine bytes.
                    params,
                    result,
                } = signature;
                self.n(params.len())?;
                for p in params {
                    self.ty(p, 0)?;
                }
                self.ty(result, 0)?;
                self.operands(args)?;
                self.place(destination)?;
                self.n(next.0)
            }
            Return => self.n(3),
            Panic(p) => {
                self.n(4)?;
                self.bytes(p.message.as_bytes())?;
                self.bytes(p.location.as_bytes())
            }
            Unreachable => self.n(5),
        }
    }
    fn function(&mut self) -> Result<()> {
        // No `..`: adding a CFG field must require an input-contract decision.
        // Proofs remain current graph authority. Loans are only read by the
        // Borrow operation, which this encoder rejects. Names are diagnostics.
        let cfg::Function {
            runtime_sites,
            lowering_proof: _,
            body_substitution: _,
            call_evidence: _,
            ref_contract: _,
            loan_sites: _,
            name: _,
            params,
            result,
            locals,
            blocks,
            entry,
            return_local,
        } = self.function;
        self.ty(result, 0)?;
        self.n(locals.len())?;
        for local in locals {
            let ir::LocalDecl {
                name: _,
                ty,
                mutable: _, // Checked before backend input publication.
            } = local;
            self.ty(ty, 0)?;
        }
        self.n(params.len())?;
        for p in params {
            self.n(p.0)?;
        }
        self.n(return_local.0)?;
        self.n(entry.0)?;
        self.n(blocks.len())?;
        for (bi, block) in blocks.iter().enumerate() {
            let cfg::BasicBlock {
                statements,
                terminator,
            } = block;
            self.n(statements.len())?;
            for (si, statement) in statements.iter().enumerate() {
                // CfgRuntimeSites is indexed by physical operation, NOT origin.
                self.meter.work(usize::BITS as usize)?;
                let location = runtime_sites.get(cfg::BlockId(bi), si);
                self.flag(location.is_some())?;
                if let Some(l) = location {
                    self.bytes(l.path.as_bytes())?;
                    self.n(l.line)?;
                    self.n(l.column)?;
                }
                let cfg::Located { value, origin: _ } = statement;
                self.statement(value)?;
            }
            let cfg::Located { value, origin: _ } = terminator;
            self.terminator(value)?;
        }
        Ok(())
    }
}

pub(super) fn key(
    function: &cfg::Function,
    resolve: &dyn Fn(&ir::CallTarget) -> std::result::Result<String, String>,
    overflow_checks: bool,
    unwind: bool,
    compiler: &[u8; 32],
    meter: &ssa::meter::Meter<'_>,
) -> std::result::Result<Option<[u8; 32]>, String> {
    meter.work(128)?;
    let mut encoder = Encoder {
        key: KeyBuilder::new(b"native-stack-cfg-v1", compiler),
        meter,
        function,
        resolve,
    };
    let result = (|| {
        encoder.flag(overflow_checks)?;
        encoder.flag(unwind)?;
        encoder.function()
    })();
    match result {
        Ok(()) => Ok(Some(encoder.key.finish())),
        Err(Failure::Unsupported) => Ok(None),
        Err(Failure::Resource(e)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> cfg::Function {
        cfg::Function {
            runtime_sites: Default::default(),
            lowering_proof: None,
            body_substitution: None,
            call_evidence: [].into(),
            ref_contract: Default::default(),
            loan_sites: vec![],
            name: "fixture".into(),
            params: vec![],
            result: ir::Ty::Unit,
            locals: vec![ir::LocalDecl {
                name: "return".into(),
                ty: ir::Ty::Unit,
                mutable: true,
            }],
            blocks: vec![cfg::BasicBlock {
                statements: vec![],
                terminator: cfg::Located {
                    value: cfg::Terminator::Return,
                    origin: None,
                },
            }],
            entry: cfg::BlockId(0),
            return_local: ir::LocalId(0),
        }
    }

    fn fingerprint(function: &cfg::Function) -> Option<[u8; 32]> {
        let budget = ir::PlanBudget::default();
        key(
            function,
            &|_| panic!("call-free fixture"),
            true,
            false,
            &[7; 32],
            &ssa::meter::Meter::new(&budget),
        )
        .unwrap()
    }

    #[test]
    fn code_inputs_change_keys_but_diagnostic_names_do_not() {
        let original = fixture();
        let expected = fingerprint(&original).unwrap();
        let mut renamed = original.clone();
        renamed.name = "other".into();
        renamed.locals[0].name = "other".into();
        renamed.blocks[0].terminator.origin = Some(ir::ExprId(42));
        assert_eq!(fingerprint(&renamed), Some(expected));
        let mut changed = original.clone();
        changed.locals[0].ty = ir::Ty::Bool;
        assert_ne!(fingerprint(&changed), Some(expected));
        changed = original.clone();
        changed.blocks[0].terminator.value = cfg::Terminator::Unreachable;
        assert_ne!(fingerprint(&changed), Some(expected));
        let budget = ir::PlanBudget::default();
        let meter = ssa::meter::Meter::new(&budget);
        assert_ne!(
            key(
                &original,
                &|_| unreachable!(),
                false,
                false,
                &[7; 32],
                &meter
            )
            .unwrap(),
            Some(expected)
        );
        assert_ne!(
            key(&original, &|_| unreachable!(), true, true, &[7; 32], &meter).unwrap(),
            Some(expected)
        );
        assert_ne!(
            key(
                &original,
                &|_| unreachable!(),
                true,
                false,
                &[8; 32],
                &meter
            )
            .unwrap(),
            Some(expected)
        );
    }

    #[test]
    fn literals_are_length_delimited_and_backend_only_variants_fail_closed() {
        let mut function = fixture();
        function.blocks[0].statements.push(cfg::Located {
            value: cfg::Statement::Assign {
                destination: ir::Place {
                    local: ir::LocalId(0),
                    projection: vec![],
                },
                value: cfg::Rvalue::Println("abc".into()),
            },
            origin: None,
        });
        let first = fingerprint(&function).unwrap();
        let cfg::Statement::Assign { value, .. } = &mut function.blocks[0].statements[0].value
        else {
            unreachable!()
        };
        *value = cfg::Rvalue::Println("abc\0".into());
        assert_ne!(fingerprint(&function), Some(first));
        let cfg::Statement::Assign { value, .. } = &mut function.blocks[0].statements[0].value
        else {
            unreachable!()
        };
        *value = cfg::Rvalue::Borrow(cfg::LoanId(99));
        assert_eq!(fingerprint(&function), None);
    }

    #[test]
    fn recursive_structural_types_are_eligible_and_nominal_free() {
        let mut function = fixture();
        function.locals[0].ty = ir::Ty::Tuple(
            vec![
                ir::Ty::Array {
                    element: ir::Ty::Int(ir::IntTy::U32).into(),
                    len: 3,
                },
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: ir::Ty::Slice {
                        element: ir::Ty::Bool.into(),
                    }
                    .into(),
                },
                ir::Ty::RawPtr {
                    kind: ir::BorrowKind::Mut,
                    pointee: ir::Ty::Char.into(),
                },
            ]
            .into(),
        );
        assert!(fingerprint(&function).is_some());
    }
}
