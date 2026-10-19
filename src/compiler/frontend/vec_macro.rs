//! Builtin `vec!` expansion into ordinary source calls on the prelude Vec:
//! `vec![]` -> `std::vec::Vec::new()`,
//! `vec![e; n]` -> `std::vec::from_elem(e, n)`,
//! `vec![a, b, c]` -> `{ let mut v = std::vec::__jr_vec_seed(a, 3); v.push(b); v.push(c); v }`.
//! Elements are evaluated once each, in source order, as stock does.
use super::*;

impl Parser<'_, '_> {
    fn vec_path(
        &mut self,
        site: super::super::expansion_span::SpanId,
        segments: &[&str],
    ) -> Result<AstPath, ParseError> {
        let mut idents = Vec::new();
        for segment in segments {
            idents.push(
                self.names
                    .intern_root(segment, site)
                    .map_err(native_adapter::name_error)?,
            );
        }
        AstPath::new(self.names, PathAnchor::Relative, &idents).map_err(native_adapter::name_error)
    }

    pub(super) fn vec_expression(&mut self) -> Result<Expr, ParseError> {
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
            return Err(self.error("expected a vec! delimiter"));
        };
        self.names
            .budget()
            .charge(super::super::expansion_budget::Counter::Work, 4)
            .map_err(|error| native_adapter::transport_error(error.into()))?;
        if self.eat(close) {
            let name = self.vec_path(site, &["std", "vec", "Vec", "new"])?;
            return Ok(Expr::Call {
                name,
                args: Vec::new(),
            });
        }
        let first = self.nested_expr()?;
        if self.eat(";") {
            let count = self.nested_expr()?;
            self.expect(close)?;
            let name = self.vec_path(site, &["std", "vec", "from_elem"])?;
            return Ok(Expr::Call {
                name,
                args: vec![first, count],
            });
        }
        let mut rest = Vec::new();
        while self.eat(",") && !self.is(close) {
            self.names
                .budget()
                .charge(super::super::expansion_budget::Counter::Work, 2)
                .map_err(|error| native_adapter::transport_error(error.into()))?;
            rest.push(self.nested_expr()?);
        }
        self.expect(close)?;
        let capacity = rest.len() as u64 + 1;
        let seed = self.vec_path(site, &["std", "vec", "__jr_vec_seed"])?;
        let seeded = Expr::Call {
            name: seed,
            args: vec![
                first,
                Expr::Integer(IntegerLiteral {
                    magnitude: u128::from(capacity),
                    suffix: Some(ir::IntTy::Usize),
                }),
            ],
        };
        if rest.is_empty() {
            return Ok(seeded);
        }
        // A hygienic temporary: `$vec` cannot be written by user source.
        let temp = self
            .names
            .intern_root("__jr_vec_tmp", site)
            .map_err(native_adapter::name_error)?;
        let push = self
            .names
            .intern_root("push", site)
            .map_err(native_adapter::name_error)?;
        let frontier = self.enter_lexical()?;
        let built = (|| {
            self.bind_lexical(temp)?;
            self.reference_lexical(temp)?;
            let var = || AstPath::new(self.names, PathAnchor::Relative, &[temp]);
            let mut statements = vec![Stmt::Let {
                name: BindingName::Named(temp),
                mutable: true,
                ty: Type::Infer,
                value: seeded,
            }];
            for value in rest {
                statements.push(Stmt::Expr(Expr::MethodCall {
                    receiver: Box::new(Expr::Var(var().map_err(native_adapter::name_error)?)),
                    name: push,
                    type_args: Vec::new(),
                    args: vec![value],
                }));
            }
            Ok(Block {
                statements,
                tail: Some(Box::new(Expr::Var(
                    var().map_err(native_adapter::name_error)?,
                ))),
            })
        })();
        self.restore_frontier(frontier);
        let expression = Expr::Block(built?);
        validate_depth(DepthNode::Expr(&expression)).map_err(|e| self.depth_error(&e))?;
        Ok(expression)
    }
}
