//! Immediate closures are separate callable declarations, never substituted bodies.
use super::*;
pub(in crate::compiler) mod graph_facts;

#[derive(Clone, Debug)]
pub(super) struct ClosureParent {
    pub name: AstPath,
    pub params: Vec<(BindingName, Type)>,
}

#[derive(Clone, Debug)]
pub(super) struct SourceCapture {
    pub parameter: usize,
    pub name: AstIdent,
    pub ty: Type,
}

#[derive(Clone, Debug)]
pub(super) struct ClosureSource {
    pub name: AstPath,
    pub environment: AstIdent,
    pub parent: Option<ClosureParent>,
    pub by_move: bool,
    pub captures: Vec<SourceCapture>,
}

pub(super) fn anonymous_path(path: &AstPath, names: &NameStore) -> Result<bool, String> {
    path.check(names).map_err(native_adapter::name_diagnostic)?;
    if path.anchor() != PathAnchor::Crate {
        return Ok(false);
    }
    path.segments().last().map_or(Ok(false), |name| {
        names
            .is_anonymous_closure(*name)
            .map_err(native_adapter::name_diagnostic)
    })
}

impl Parser<'_, '_> {
    pub(super) fn closure_path(&mut self, ident: AstIdent) -> Result<AstPath, ParseError> {
        let mut segments = vec![ident];
        let mut module = self.module;
        while let Some(parent) = self.modules.parent(module).map_err(module_parse_error)? {
            let name = self
                .modules
                .name(module)
                .map_err(module_parse_error)?
                .ok_or_else(|| self.error("closure outside a source module"))?;
            self.reserve_parser_items(&mut segments, 1)?;
            segments.push(name);
            module = parent;
        }
        segments.reverse();
        AstPath::new(self.names, PathAnchor::Crate, &segments).map_err(native_adapter::name_error)
    }
    /// The opening grouping parenthesis has already been consumed. This first
    /// increment has an empty environment and explicit scalar result typing.
    /// Stored closures and inferred signatures require genuine environment types.
    pub(super) fn immediate_closure(&mut self) -> Result<Expr, ParseError> {
        // A closure retains its complete signature while parsing a nested
        // callable. Bound that heavier recursive frame before entering it.
        // This is live nesting depth, not a compilation-wide closure budget.
        if self.lexical.closure_depth >= 16 {
            return Err(self.limit("closure nesting depth limit 16"));
        }
        if self.expansion.is_some() || self.item_expansion.is_some() {
            return Err(self.unsupported("closures in macro expansions are unsupported"));
        }
        if !self.type_params.is_empty() {
            return Err(self.unsupported("closures in generic bodies are unsupported"));
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
                if !self.eat(":") {
                    return Err(self.unsupported("closure parameter inference is unsupported"));
                }
                let ty = self.ty()?;
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
        if !self.eat("->") {
            return Err(self.unsupported("closure return inference is unsupported"));
        }
        let result = self.ty()?;
        if !matches!(
            result,
            Type::Unit | Type::Bool | Type::Char | Type::Int(_) | Type::Float(_)
        ) {
            return Err(self.unsupported("non-scalar closure return types are unsupported"));
        }
        let serial = self.lexical.closure_serial;
        self.lexical.closure_serial = serial
            .checked_add(1)
            .ok_or_else(|| self.limit("closure identity exhausted"))?;
        let ident = self
            .names
            .anonymous_closure(origin, serial)
            .map_err(native_adapter::name_error)?;
        let name = self.closure_path(ident)?;
        let environment = self
            .names
            .anonymous_closure_environment(origin, serial)
            .map_err(native_adapter::name_error)?;
        let parent = self.lexical.closure_parent.replace(ClosureParent {
            name: name.clone(),
            params: params.clone(),
        });
        // A closure's body cannot inherit its parent's return/loop/binding or
        // macro frontier. The ordinary independent checker owns all its locals.
        let previous = self.lexical.function.take();
        self.lexical.closure_depth += 1;
        let body = self.function_body(&params);
        self.lexical.closure_depth -= 1;
        self.lexical.function = previous;
        self.lexical.closure_parent = parent.clone();
        let (body, lexical_scope) = body?;
        self.expect(")")?;
        if !self.eat("(") {
            return Err(self.unsupported("stored closure values are unsupported"));
        }
        let mut args = Vec::new();
        while !self.is(")") {
            if args.len() >= 64 {
                return Err(self.limit("closure argument limit 64"));
            }
            self.reserve_parser_items(&mut args, 1)?;
            args.push(self.nested_expr()?);
            if !self.eat(",") {
                break;
            }
        }
        self.expect(")")?;
        let function = Function {
            lexical_scope,
            module: self.module,
            constness: Constness::Runtime,
            visibility: ir::Visibility::Restricted(self.module),
            bounds: Vec::new(),
            name: name.clone(),
            regions: RegionDecls::default(),
            type_params: Vec::new(),
            params,
            param_mut,
            result,
            impl_params: 0,
            body,
        };
        let mut functions = std::mem::take(&mut self.lexical.closure_functions);
        self.reserve_parser_items(&mut functions, 1)?;
        functions.push(function);
        self.lexical.closure_functions = functions;
        let mut sources = std::mem::take(&mut self.lexical.closure_sources);
        self.reserve_parser_items(&mut sources, 1)?;
        sources.push(ClosureSource {
            name: name.clone(),
            environment,
            parent,
            by_move,
            captures: Vec::new(),
        });
        self.lexical.closure_sources = sources;
        Ok(Expr::Call { name, args })
    }
}
