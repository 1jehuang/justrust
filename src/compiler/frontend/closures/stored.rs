//! Stored closure syntax stays attached to its private anonymous declaration.
//! Parameter/result inference and capture classification belong to its parent
//! body checker, not the whole-program signatures producer.
use super::*;

#[derive(Clone, Debug)]
pub(in crate::compiler::frontend) struct StoredClosureSyntax {
    pub site: u32,
    pub params: Vec<(BindingName, Type)>,
    pub param_mut: Vec<bool>,
    pub result: Type,
    pub body: Block,
    pub lexical_scope: Option<super::super::super::semantic_scope::ScopeId>,
}

impl PlanCtx {
    pub(in crate::compiler::frontend) fn stored_closure_syntax(
        &self,
        name: &AstPath,
    ) -> Result<Option<&StoredClosureSyntax>, String> {
        let key = name_scope::path_key(self.program.names.as_ref(), name, Namespace::Type)?;
        Ok(self
            .program
            .closure_sources
            .get(&key)
            .and_then(|source| source.stored.as_ref()))
    }
}

impl Parser<'_, '_> {
    pub(in crate::compiler::frontend) fn stored_closure(&mut self) -> Result<Expr, ParseError> {
        if self.lexical.closure_depth >= 16 {
            return Err(self.limit("closure nesting depth limit 16"));
        }
        let origin = self.token().span;
        let by_move = self.eat("move");
        let empty = self.eat("||");
        if !empty {
            self.expect("|")?;
        }
        let mut params = Vec::new();
        let mut param_mut = Vec::new();
        if !empty {
            while !self.is("|") {
                if params.len() >= 64 {
                    return Err(self.limit("closure parameter limit 64"));
                }
                let mutable = self.eat("mut");
                let name = self.binding()?;
                let ty = if self.eat(":") {
                    self.ty()?
                } else {
                    Type::Infer
                };
                self.reserve_parser_items(&mut params, 1)?;
                self.reserve_parser_items(&mut param_mut, 1)?;
                params.push((name, ty));
                param_mut.push(mutable);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect("|")?;
        }
        let result = if self.eat("->") {
            if self.eat("!") {
                Type::Never
            } else {
                self.ty()?
            }
        } else {
            Type::Infer
        };
        let serial = self.lexical.closure_serial;
        self.lexical.closure_serial = serial
            .checked_add(1)
            .ok_or_else(|| self.limit("closure identity exhausted"))?;
        let ident = self
            .names
            .anonymous_closure(origin, serial)
            .map_err(native_adapter::name_error)?;
        let name = self.closure_path(ident)?;
        let parent = self.lexical.closure_parent.replace(ClosureParent {
            name: name.clone(),
            params: params.clone(),
        });
        let previous = self.lexical.function.take();
        self.lexical.closure_depth += 1;
        let body = if self.is("{") {
            self.function_body(&params)
        } else {
            self.nested_expr().map(|value| {
                (
                    Block {
                        statements: Vec::new(),
                        tail: Some(Box::new(value)),
                    },
                    None,
                )
            })
        };
        self.lexical.closure_depth -= 1;
        self.lexical.function = previous;
        self.lexical.closure_parent = parent.clone();
        let (body, lexical_scope) = body?;
        let site =
            u32::try_from(serial).map_err(|_| self.limit("closure site identity overflow"))?;
        let mut slots = Vec::new();
        for field in 0..5 {
            slots.push(
                self.names
                    .anonymous_closure_environment(origin, serial, field)
                    .map_err(native_adapter::name_error)?,
            );
        }
        // The callable declaration describes the adapter ABI, never guessed
        // parameter/capture types. Parent checking supplies its exact actuals.
        let function = Function {
            lexical_scope: None,
            module: self.module,
            constness: Constness::Runtime,
            unsafety: false,
            static_mut: false,
            visibility: ir::Visibility::Restricted(self.module),
            bounds: Vec::new(),
            name: name.clone(),
            regions: RegionDecls::default(),
            type_params: slots[..3].to_vec(),
            params: vec![
                (BindingName::Named(slots[3]), Type::Param(slots[0])),
                (BindingName::Named(slots[4]), Type::Param(slots[1])),
            ],
            param_mut: vec![false, false],
            result: Type::Param(slots[2]),
            impl_params: 0,
            body: Block {
                statements: Vec::new(),
                tail: None,
            },
        };
        let mut functions = std::mem::take(&mut self.lexical.closure_functions);
        self.reserve_parser_items(&mut functions, 1)?;
        functions.push(function);
        self.lexical.closure_functions = functions;
        let mut sources = std::mem::take(&mut self.lexical.closure_sources);
        self.reserve_parser_items(&mut sources, 1)?;
        sources.push(ClosureSource {
            name: name.clone(),
            environments: Vec::new(),
            parent,
            owner_source: None,
            by_move,
            captures: Vec::new(),
            stored: Some(StoredClosureSyntax {
                site,
                params,
                param_mut,
                result,
                body,
                lexical_scope,
            }),
        });
        self.lexical.closure_sources = sources;
        Ok(Expr::Var(name))
    }
}
