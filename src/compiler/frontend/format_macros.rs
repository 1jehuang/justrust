//! Buffered format macros: `format!`, `write!` and `writeln!`.
//!
//! The parser produces `Expr::FormatInto`. The checker lowers it to a block
//! that owns a private `String` buffer:
//!
//! ```text
//! { let mut $buf = String::new();
//!   <arg_i as std::fmt::DisplayInto>::__jr_fmt_into(&arg_i, &mut $buf, piece_i, "", spec..);
//!   ... trailing piece pushed with push_str ...
//!   $buf }                                   // format!
//!   dst.write_str($buf.as_str())             // write!/writeln!
//! ```
//!
//! Arguments are evaluated once each, in source order, before any output is
//! produced (format_args semantics). A missing Display/Debug impl is the
//! ordinary E0277 trait-selection error.
use super::format_spec::{FormatSpec, FormatTrait};
use super::*;

const ABSENT: u128 = u64::MAX as u128;

/// The placeholder calls of one buffered format macro. Built by the parser
/// inside a block that owns the buffer; `rest[i]` are the fabricated source
/// arguments after the receiver for placeholder `i`:
/// `[out, prefix, "", fill, align, width, precision, flags]`, where `out` is
/// the block's `&mut String` local.
#[derive(Clone, Debug)]
pub struct SourceFormatInto {
    pub arguments: Vec<Expr>,
    pub specs: Vec<FormatSpec>,
    pub rest: Vec<Vec<Expr>>,
}

impl Parser<'_, '_> {
    fn fmt_ident(
        &mut self,
        text: &str,
        site: super::super::expansion_span::SpanId,
    ) -> Result<AstIdent, ParseError> {
        self.names
            .intern_root(text, site)
            .map_err(native_adapter::name_error)
    }
    fn fmt_path(
        &mut self,
        site: super::super::expansion_span::SpanId,
        segments: &[&str],
    ) -> Result<AstPath, ParseError> {
        let mut idents = Vec::new();
        for segment in segments {
            idents.push(self.fmt_ident(segment, site)?);
        }
        AstPath::new(self.names, PathAnchor::Relative, &idents).map_err(native_adapter::name_error)
    }
    fn fmt_var(&mut self, ident: AstIdent) -> Result<Expr, ParseError> {
        self.reference_lexical(ident)?;
        Ok(Expr::Var(
            AstPath::new(self.names, PathAnchor::Relative, &[ident])
                .map_err(native_adapter::name_error)?,
        ))
    }
    fn fmt_method(
        &mut self,
        receiver: Expr,
        name: &str,
        site: super::super::expansion_span::SpanId,
        args: Vec<Expr>,
    ) -> Result<Expr, ParseError> {
        Ok(Expr::MethodCall {
            receiver: Box::new(receiver),
            name: self.fmt_ident(name, site)?,
            type_args: Vec::new(),
            args,
        })
    }

    pub(super) fn format_into_expression(&mut self, spelling: &str) -> Result<Expr, ParseError> {
        let token_site = self.token().span;
        let site = panic_callsite(self.names.session(), token_site)
            .map_err(native_adapter::transport_error)?;
        self.ident()?;
        self.expect("!")?;
        let close = if self.eat("(") {
            ")"
        } else if self.eat("[") {
            "]"
        } else if self.eat("{") {
            "}"
        } else {
            return Err(self.error("expected a format macro delimiter"));
        };
        self.names
            .budget()
            .charge(super::super::expansion_budget::Counter::Work, 8)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        let destination = if spelling == "format" {
            None
        } else {
            Some(self.nested_expr()?)
        };
        let (mut pieces, arguments, specs) = if destination.is_some() && !self.eat(",") {
            if spelling != "writeln" {
                return Err(self.error("expected `,` after the write! destination"));
            }
            self.expect(close)?;
            (vec![String::new()], Vec::new(), Vec::new())
        } else {
            if self.is(close) {
                return Err(self.error("requires at least a format string argument"));
            }
            let buffer;
            let token = if self.token().is_opaque_literal() {
                buffer = self
                    .input
                    .opaque_literal_buffer(&self.token(), self.names)?;
                TokenCursor::new(&buffer).current()
            } else {
                self.token()
            };
            if !token.string {
                return Err(self.error("format argument must be a string literal"));
            }
            let text = self.string_literal_value(token.text)?;
            self.cursor.bump();
            let (pieces, holders) =
                format_spec::parse_format(&text).map_err(|message| self.error(&message))?;
            let (arguments, _modes, specs) = self.format_arguments(close, &holders)?;
            if holders.is_empty() && !arguments.is_empty() {
                return Err(self.error("argument never used"));
            }
            (pieces, arguments, specs)
        };
        if spelling == "writeln" {
            if let Some(last) = pieces.last_mut() {
                last.push('\n');
            }
        }
        self.names
            .retain_parser_text(
                pieces.iter().map(String::len).sum::<usize>()
                    + arguments.len() * 9 * std::mem::size_of::<Expr>(),
            )
            .map_err(native_adapter::name_error)?;
        // A literal-only format!: `String::from("text")`.
        if arguments.is_empty() && destination.is_none() {
            let from = self.fmt_path(site, &["std", "string", "String", "from"])?;
            return Ok(Expr::Call {
                name: from,
                args: vec![Expr::Str(pieces.concat())],
            });
        }
        let buf = self.fmt_ident("__jr_fmt_buf", site)?;
        let out = self.fmt_ident("__jr_fmt_out", site)?;
        let frontier = self.enter_lexical()?;
        let built = (|| {
            let new = self.fmt_path(site, &["std", "string", "String", "new"])?;
            let mut statements = vec![Stmt::Let {
                name: BindingName::Named(buf),
                mutable: true,
                ty: Type::Infer,
                value: Expr::Call {
                    name: new,
                    args: Vec::new(),
                },
            }];
            self.bind_lexical(buf)?;
            let tail_piece = pieces.pop().unwrap_or_default();
            if !arguments.is_empty() {
                let frontier = self.enter_lexical()?;
                let inner = (|| {
                    let borrow = Expr::Borrow {
                        kind: ir::BorrowKind::Mut,
                        place: Place {
                            local: LocalName::Named(buf),
                            fields: Vec::new(),
                        },
                    };
                    self.reference_lexical(buf)?;
                    let mut inner = vec![Stmt::Let {
                        name: BindingName::Named(out),
                        mutable: false,
                        ty: Type::Infer,
                        value: borrow,
                    }];
                    self.bind_lexical(out)?;
                    let mut rest = Vec::new();
                    for (prefix, spec) in pieces.iter().zip(&specs) {
                        let mut row = vec![
                            self.fmt_var(out)?,
                            Expr::Str(prefix.clone()),
                            Expr::Str(String::new()),
                        ];
                        row.extend(spec_arguments(spec));
                        rest.push(row);
                    }
                    inner.push(Stmt::Expr(Expr::FormatInto(
                        self.names
                            .try_parser_box(SourceFormatInto {
                                arguments,
                                specs,
                                rest,
                            })
                            .map_err(native_adapter::name_error)?,
                    )));
                    Ok::<_, ParseError>(inner)
                })();
                self.restore_frontier(frontier);
                statements.push(Stmt::Expr(Expr::Block(Block {
                    statements: inner?,
                    tail: None,
                })));
            }
            if !tail_piece.is_empty() {
                let var = self.fmt_var(buf)?;
                let push = self.fmt_method(var, "push_str", site, vec![Expr::Str(tail_piece)])?;
                statements.push(Stmt::Expr(push));
            }
            let var = self.fmt_var(buf)?;
            Ok::<_, ParseError>(Block {
                statements,
                tail: Some(Box::new(var)),
            })
        })();
        self.restore_frontier(frontier);
        let block = Expr::Block(built?);
        let expression = match destination {
            None => {
                let wrap = self.fmt_path(site, &["std", "fmt", "__jr_format"])?;
                Expr::Call {
                    name: wrap,
                    args: vec![block],
                }
            }
            Some(destination) => {
                self.fmt_method(destination, "__jr_write_buf", site, vec![block])?
            }
        };
        validate_depth(DepthNode::Expr(&expression)).map_err(|e| self.depth_error(&e))?;
        Ok(expression)
    }
}
impl Checker<'_> {
    /// Each argument is evaluated in source order and borrowed (a place is
    /// borrowed in place, a value is owned by a temporary), then one
    /// `*Into` trait call per placeholder appends to the macro buffer.
    pub(super) fn check_format_into(
        &mut self,
        source: &SourceFormatInto,
    ) -> Result<(ir::ExprKind, Flow), String> {
        if source.specs.len() != source.arguments.len()
            || source.rest.len() != source.arguments.len()
        {
            return Err("format argument count mismatch".into());
        }
        let mut statements = Vec::new();
        let mut completes = true;
        let mut borrows = Vec::new();
        for (index, argument) in source.arguments.iter().enumerate() {
            let place = static_source_place(argument)
                .ok()
                .filter(|_| place_expression(argument));
            let borrow = if let Some(place) = &place {
                let (place, _, _) = self.checked_place(place)?;
                self.borrow_expression(ir::BorrowKind::Shared, place)?
            } else {
                let value = self.expr_expected(argument, None)?;
                completes &= self.flow(value).completes;
                let ty = self.exprs[value.0].ty.clone();
                let owner = self.temporary(&format!("$fmt_into{index}"), ty, false)?;
                statements.push(ir::Stmt::Let {
                    local: owner,
                    value,
                });
                self.borrow_expression(
                    ir::BorrowKind::Shared,
                    ir::Place {
                        local: owner,
                        projection: Vec::new(),
                    },
                )?
            };
            let ty = self.exprs[borrow.0].ty.clone();
            let local = self.temporary(&format!("$fmt_into_ref{index}"), ty.clone(), false)?;
            statements.push(ir::Stmt::Let {
                local,
                value: borrow,
            });
            borrows.push((local, ty));
        }
        for (index, (local, ty)) in borrows.into_iter().enumerate() {
            let mut borrow = self.into_lit(ty.clone(), ir::ExprKind::Local(local))?;
            let ir::Ty::Ref { referent, .. } = ty else {
                return Err("contract: format operand is not a shared borrow".into());
            };
            let mut referent = referent.as_ref().clone();
            while let ir::Ty::Ref { referent: inner, .. } = &referent {
                let inner = inner.as_ref().clone();
                borrow = self.into_lit(referent.clone(), ir::ExprKind::Deref { value: borrow })?;
                referent = inner;
            }
            let (trait_name, method) =
                into_trait(&source.specs[index], matches!(referent, ir::Ty::Str))?;
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
                let wanted = if source.specs[index].format == FormatTrait::Debug {
                    "Debug"
                } else {
                    "Display"
                };
                return Err(format!(
                    "E0277: `{referent:?}` doesn't implement `std::fmt::{wanted}`"
                ));
            };
            let call = self.finish_checked_call(
                target,
                ident,
                signature,
                &source.rest[index],
                Arc::from([]),
                Some((borrow, None)),
            )?;
            if let Some(cap) = self.canonical {
                let pointer = &source.arguments[index] as *const Expr as usize | 2;
                cap.record_call_at(pointer, call, &self.exprs[call.0].kind)?;
            }
            statements.push(ir::Stmt::Expr(call));
        }
        Ok((
            ir::ExprKind::Block(ir::Block {
                statements,
                tail: None,
            }),
            Flow {
                ty: ir::Ty::Unit,
                completes,
            },
        ))
    }

    fn into_lit(&mut self, ty: ir::Ty, kind: ir::ExprKind) -> Result<ir::ExprId, String> {
        self.charge_node(std::mem::size_of::<ir::TypedExpr>())?;
        let id = ir::ExprId(self.exprs.len());
        self.exprs.push(ir::TypedExpr {
            ty,
            completion: ir::Completion::Returns,
            kind,
        });
        Ok(id)
    }
}

/// Spec arguments shared by every `*Into` call: fill, align, width,
/// precision, flags.
pub(super) fn spec_arguments(spec: &FormatSpec) -> [Expr; 5] {
    let int = |magnitude: u128, ty: ir::IntTy| {
        Expr::Integer(IntegerLiteral {
            magnitude,
            suffix: Some(ty),
        })
    };
    [
        Expr::Char(spec.fill),
        int(u128::from(spec.align), ir::IntTy::U8),
        int(spec.width.map_or(ABSENT, |w| w as u128), ir::IntTy::Usize),
        int(
            spec.precision.map_or(ABSENT, |p| p as u128),
            ir::IntTy::Usize,
        ),
        int(u128::from(spec.flags), ir::IntTy::U32),
    ]
}

pub(super) fn into_trait(spec: &FormatSpec, unsized_str: bool) -> Result<(&'static str, &'static str), String> {
    Ok(match (spec.format, unsized_str) {
        (FormatTrait::Display, false) => ("DisplayInto", "__jr_fmt_into"),
        (FormatTrait::Debug, false) => ("DebugInto", "__jr_fmt_into"),
        (FormatTrait::Display, true) => ("StrDisplayInto", "__jr_fmt_into_str"),
        (FormatTrait::Debug, true) => ("StrDebugInto", "__jr_fmt_into_str"),
        (other, _) => return Err(format!("unsupported: {other:?} format trait placeholders")),
    })
}
