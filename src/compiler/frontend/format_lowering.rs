//! Library-routed format placeholders.
//!
//! `println!`-family placeholders whose argument type has no native scalar
//! formatter, or that carry a spec (width, precision, fill, `#`, `+`, `0`,
//! radix), are printed by the Rust-source `std::fmt` library:
//! `<T as std::fmt::DisplayArg>::__jr_fmt(&arg, fd, fill, align, width,
//! precision, flags)` (or `DebugArg`). The trait is blanket-implemented for
//! every `T: Display` / `T: Debug`, so a missing user impl is the ordinary
//! E0277 trait error. Every argument is evaluated and borrowed in source
//! order first (format_args semantics); the output then runs as one Block of
//! native literal/scalar segments interleaved with these calls.
use super::format_spec::{FormatSpec, FormatTrait};
use super::*;

const ABSENT: u128 = u64::MAX as u128;

impl Checker<'_> {
    /// Whether the native scalar formatter prints `ty` under `spec` exactly.
    pub(super) fn native_format(spec: &FormatSpec, ty: &ir::Ty) -> bool {
        match spec.native_mode() {
            Some(mode) => ir::validate_format_ty(mode, ty, None).is_ok(),
            None => false,
        }
    }

    fn format_lit(&mut self, ty: ir::Ty, kind: ir::ExprKind) -> Result<ir::ExprId, String> {
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty,
            completion: ir::Completion::Returns,
            kind,
        });
        Ok(id)
    }

    /// One library formatting call for an already evaluated shared borrow.
    pub(super) fn library_format_call(
        &mut self,
        borrow: ir::ExprId,
        referent: &ir::Ty,
        spec: &FormatSpec,
        stream: ir::OutputStream,
    ) -> Result<ir::ExprId, String> {
        // `&T` formats exactly as `T`: peel reference layers by copying the
        // inner reference out (`*r` of a `&&T` is a Copy `&T`).
        let (mut borrow, mut referent) = (borrow, referent.clone());
        while let ir::Ty::Ref {
            referent: inner, ..
        } = &referent
        {
            let inner = inner.as_ref().clone();
            borrow = self.format_lit(referent.clone(), ir::ExprKind::Deref { value: borrow })?;
            referent = inner;
        }
        let referent = &referent;
        let unsized_str = matches!(referent, ir::Ty::Str);
        let (trait_name, method) = match (spec.format, unsized_str) {
            (FormatTrait::Display, false) => ("DisplayArg", "__jr_fmt"),
            (FormatTrait::Debug, false) => ("DebugArg", "__jr_fmt"),
            (FormatTrait::Display, true) => ("StrDisplayArg", "__jr_fmt_str"),
            (FormatTrait::Debug, true) => ("StrDebugArg", "__jr_fmt_str"),
            other => {
                return Err(format!("unsupported: {other:?} format trait placeholders"));
            }
        };
        let plan = self
            .signatures
            .plan
            .clone()
            .ok_or("format call requires registry")?;
        let (declaration, ident) = self.prelude_trait_method("fmt", trait_name, method)?;
        let trait_ref = ir::TraitRef {
            declaration,
            self_ty: referent.clone(),
            args: Arc::from([]),
        };
        let Some((target, signature, _)) =
            plan.select_trait(&trait_ref, &ident, &[], self.facts(), false, &plan.budget)?
        else {
            let wanted = if spec.format == FormatTrait::Display {
                "Display"
            } else {
                "Debug"
            };
            return Err(format!(
                "E0277: `{}` doesn't implement `std::fmt::{wanted}`",
                rust_ty_name(referent)
            ));
        };
        let int = |magnitude: u128, ty: ir::IntTy| {
            Expr::Integer(IntegerLiteral {
                magnitude,
                suffix: Some(ty),
            })
        };
        let rest = [
            int(
                match stream {
                    ir::OutputStream::Stdout => 1,
                    ir::OutputStream::Stderr | ir::OutputStream::PanicStderr => 2,
                },
                ir::IntTy::Usize,
            ),
            Expr::Char(spec.fill),
            int(u128::from(spec.align), ir::IntTy::U8),
            int(spec.width.map_or(ABSENT, |w| w as u128), ir::IntTy::Usize),
            int(
                spec.precision.map_or(ABSENT, |p| p as u128),
                ir::IntTy::Usize,
            ),
            int(u128::from(spec.flags), ir::IntTy::U32),
        ];
        self.finish_checked_call(
            target,
            ident,
            signature,
            &rest,
            Arc::from([]),
            Some((borrow, None)),
        )
    }

    /// Prelude trait `std::<module>::<name>` and its method `method`.
    pub(in crate::compiler) fn prelude_trait_method(
        &self,
        module: &str,
        name: &str,
        method: &str,
    ) -> Result<(ir::DeclId, AstIdent), String> {
        let plan = self.signatures.plan()?;
        let names = plan.program.names.as_ref();
        let suffix = format!("::{module}::{name}");
        for id in &plan.registry.trait_ids {
            let info = plan.registry.declaration(*id)?;
            if !info.name.ends_with(&suffix) || !plan.program.in_prelude(info.path.module) {
                continue;
            }
            let source = plan.registry.source_trait(*id)?;
            for item in &source.items {
                let spelling = names
                    .spelling(item.signature.name)
                    .map_err(native_adapter::name_diagnostic)?;
                if spelling == method {
                    return Ok((*id, item.signature.name));
                }
            }
        }
        Err(format!(
            "unsupported: formatting this argument needs std::{module}::{name}, which is not available"
        ))
    }
}

impl Checker<'_> {
    /// Check one output macro. Every argument is evaluated once, in source
    /// order. When each placeholder is a plain `{}`/`{:?}` over a native
    /// scalar type this is the native `Format` operation unchanged.
    /// Otherwise arguments are first snapshotted (native scalars by copy,
    /// others by shared borrow) and the output becomes a Block of native
    /// segments interleaved with library formatting calls.
    pub(super) fn check_format(
        &mut self,
        pieces: &[String],
        arguments: &[Expr],
        modes: &[ir::FormatMode],
        specs: &[FormatSpec],
        stream: ir::OutputStream,
        newline: bool,
    ) -> Result<(ir::ExprKind, Flow), String> {
        if pieces.len() != arguments.len() + 1
            || modes.len() != arguments.len()
            || specs.len() != arguments.len()
        {
            return Err("format argument count mismatch".into());
        }
        let mut completes = true;
        // (value id, native, borrow-or-value id for the native Format)
        let mut evaluated: Vec<(ir::ExprId, bool, ir::ExprId)> = Vec::new();
        for (argument, spec) in arguments.iter().zip(specs) {
            let checked_index_reference =
                if matches!(argument, Expr::Index { .. } | Expr::DynamicIndex { .. }) {
                    self.format_indexed_place_borrow(argument)?
                } else {
                    None
                };
            let id = if let Some(reference) = checked_index_reference {
                let ir::Ty::Ref { referent, .. } = self.exprs[reference.0].ty.clone() else {
                    return Err("indexed format operand must be a checked reference".into());
                };
                let id = self.format_lit(
                    referent.as_ref().clone(),
                    ir::ExprKind::Deref { value: reference },
                )?;
                self.exprs[id.0].completion = self.exprs[reference.0].completion;
                id
            } else {
                self.expr_expected(argument, None)?
            };
            let flow = self.flow(id);
            completes &= flow.completes;
            let native = Self::native_format(spec, &flow.ty);
            if spec.native_mode().is_some() && !native {
                // Plain placeholder: keep the precise native diagnostic for
                // types that std::fmt cannot print either.
                if !self.library_formattable(&flow.ty) {
                    let mode = spec.native_mode().unwrap();
                    ir::validate_format_ty(mode, &flow.ty, Some(&self.signatures.plan()?.budget))?;
                }
            }
            let place = static_source_place(argument)
                .ok()
                .filter(|_| place_expression(argument));
            let indexed_reference = checked_index_reference.or_else(|| {
                if matches!(argument, Expr::Index { .. } | Expr::DynamicIndex { .. }) {
                    match self.exprs[id.0].kind {
                        ir::ExprKind::Deref { value } => Some(value),
                        _ => None,
                    }
                } else {
                    None
                }
            });
            let operand = if let Some(reference) = indexed_reference {
                reference
            } else if let Some(place) = &place {
                let (place, _, _) = self.checked_place(place)?;
                self.borrow_expression(ir::BorrowKind::Shared, place)?
            } else if native
                && matches!(
                    argument,
                    Expr::DynamicIndex { .. }
                        | Expr::Deref { .. }
                        | Expr::Field { .. }
                        | Expr::Index { .. }
                )
            {
                return Err(
                    "unsupported: dynamic or temporary-rooted format place arguments".into(),
                );
            } else {
                id
            };
            evaluated.push((id, native, operand));
        }
        for piece in pieces {
            self.charge_node(piece.len())?;
        }
        self.charge_node(
            evaluated.len() * 3 * std::mem::size_of::<ir::ExprId>()
                + pieces.len() * std::mem::size_of::<String>()
                + modes.len() * std::mem::size_of::<ir::FormatMode>(),
        )?;
        let flow = Flow {
            ty: ir::Ty::Unit,
            completes,
        };
        if evaluated.iter().all(|(_, native, _)| *native) {
            return Ok((
                ir::ExprKind::Format {
                    pieces: pieces.to_vec(),
                    arguments: evaluated
                        .iter()
                        .map(|(value, _, operand)| {
                            // Native Format has special in-place handling for an
                            // explicit Borrow, not arbitrary reference-producing
                            // calls. An overloaded Index call must be dereferenced.
                            if ir::native_format_borrow(&self.exprs[operand.0]) {
                                *operand
                            } else {
                                *value
                            }
                        })
                        .collect(),
                    modes: modes.to_vec(),
                    stream,
                    newline,
                },
                flow,
            ));
        }
        // Snapshot every argument in source order before any output.
        let mut statements = Vec::new();
        let mut reads: Vec<(ir::LocalId, ir::Ty)> = Vec::new();
        for (index, (value, native, operand)) in evaluated.iter().enumerate() {
            let (init, ty) = if *native {
                // Copy scalar snapshot (the place borrow taken for the
                // native path is dereferenced, preserving evaluation order).
                let ty = self.exprs[value.0].ty.clone();
                let init = if operand != value {
                    self.format_lit(ty.clone(), ir::ExprKind::Deref { value: *operand })?
                } else {
                    *value
                };
                (init, ty)
            } else if operand != value
                || matches!(self.exprs[operand.0].kind, ir::ExprKind::Borrow { .. })
            {
                (*operand, self.exprs[operand.0].ty.clone())
            } else {
                // Temporary value: own it for the statement, lend it out.
                let ty = self.exprs[value.0].ty.clone();
                let owner = self.temporary(&format!("$fmt_value{index}"), ty.clone(), false)?;
                statements.push(ir::Stmt::Let {
                    local: owner,
                    value: *value,
                });
                let borrow = self.borrow_expression(
                    ir::BorrowKind::Shared,
                    ir::Place {
                        local: owner,
                        projection: Vec::new(),
                    },
                )?;
                (borrow, self.exprs[borrow.0].ty.clone())
            };
            let local = self.temporary(&format!("$fmt_arg{index}"), ty.clone(), false)?;
            statements.push(ir::Stmt::Let { local, value: init });
            reads.push((local, ty));
        }
        let mut run_pieces = vec![pieces[0].clone()];
        let mut run_args = Vec::new();
        let mut run_modes = Vec::new();
        for (index, (_, native, _)) in evaluated.iter().enumerate() {
            let (local, ty) = reads[index].clone();
            let read = self.format_lit(ty.clone(), ir::ExprKind::Local(local))?;
            if *native {
                run_args.push(read);
                run_modes.push(modes[index]);
                run_pieces.push(pieces[index + 1].clone());
                continue;
            }
            self.flush_format_run(
                &mut statements,
                &mut run_pieces,
                &mut run_args,
                &mut run_modes,
                stream,
                false,
            )?;
            let ir::Ty::Ref { referent, .. } = &ty else {
                return Err("contract: library format operand is not a shared borrow".into());
            };
            let referent = referent.as_ref().clone();
            let call = self.library_format_call(read, &referent, &specs[index], stream)?;
            if let Some(cap) = self.canonical {
                let pointer = &arguments[index] as *const Expr as usize | 2;
                cap.record_call_at(pointer, call, &self.exprs[call.0].kind)?;
            }
            statements.push(ir::Stmt::Expr(call));
            run_pieces = vec![pieces[index + 1].clone()];
        }
        self.flush_format_run(
            &mut statements,
            &mut run_pieces,
            &mut run_args,
            &mut run_modes,
            stream,
            newline,
        )?;
        Ok((
            ir::ExprKind::Block(ir::Block {
                statements,
                tail: None,
            }),
            flow,
        ))
    }

    fn flush_format_run(
        &mut self,
        statements: &mut Vec<ir::Stmt>,
        pieces: &mut Vec<String>,
        args: &mut Vec<ir::ExprId>,
        modes: &mut Vec<ir::FormatMode>,
        stream: ir::OutputStream,
        newline: bool,
    ) -> Result<(), String> {
        let kind = if args.is_empty() {
            let text = pieces.concat();
            if text.is_empty() && !newline {
                pieces.clear();
                return Ok(());
            }
            ir::ExprKind::Output(ir::LiteralOutput {
                text,
                stream,
                newline,
            })
        } else {
            ir::ExprKind::Format {
                pieces: std::mem::take(pieces),
                arguments: std::mem::take(args),
                modes: std::mem::take(modes),
                stream,
                newline,
            }
        };
        pieces.clear();
        args.clear();
        modes.clear();
        let id = self.format_lit(ir::Ty::Unit, kind)?;
        statements.push(ir::Stmt::Expr(id));
        Ok(())
    }

    /// Types the library formatter may handle (nominal values, references,
    /// str, floats, scalars with specs). Final acceptance is trait selection.
    fn library_formattable(&self, ty: &ir::Ty) -> bool {
        !matches!(ty, ir::Ty::Never | ir::Ty::Tuple(_) | ir::Ty::Array { .. })
    }
}

/// Source-like spelling of a type for diagnostics.
pub(super) fn rust_ty_name(ty: &ir::Ty) -> String {
    let strip = |name: &str| -> String {
        let name = name.strip_prefix("crate::").unwrap_or(name);
        let std = crate::compiler::source::source_prelude::PRELUDE_MODULE;
        match name.strip_prefix(std) {
            Some(rest) => format!("std{rest}"),
            None => name.to_owned(),
        }
    };
    let args = |args: &[ir::Ty]| -> String {
        if args.is_empty() {
            String::new()
        } else {
            let parts: Vec<String> = args.iter().map(rust_ty_name).collect();
            format!("<{}>", parts.join(", "))
        }
    };
    match ty {
        ir::Ty::Struct(t) => format!("{}{}", strip(&t.def().name), args(&t.def().type_args)),
        ir::Ty::Enum(t) => format!("{}{}", strip(&t.def().name), args(&t.def().type_args)),
        ir::Ty::Ref { kind, referent } => format!(
            "&{}{}",
            if *kind == ir::BorrowKind::Mut { "mut " } else { "" },
            rust_ty_name(referent)
        ),
        ir::Ty::Slice { element } => format!("[{}]", rust_ty_name(element)),
        ir::Ty::Array { element, len } => format!("[{}; {len}]", rust_ty_name(element)),
        ir::Ty::Tuple(fields) => {
            let parts: Vec<String> = fields.iter().map(rust_ty_name).collect();
            format!("({})", parts.join(", "))
        }
        ir::Ty::Str => "str".into(),
        ir::Ty::Unit => "()".into(),
        ir::Ty::Bool => "bool".into(),
        ir::Ty::Char => "char".into(),
        other => format!("{other:?}"),
    }
}
