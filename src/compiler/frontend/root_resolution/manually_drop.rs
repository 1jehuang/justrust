//! Canonical catalog wrapper source admission, never a source nominal shim.
use super::*;

impl Resolver<'_> {
    fn is_manually_drop_path(&mut self, path: &AstPath, module: ModuleId) -> Result<bool, String> {
        path.check(self.names())
            .map_err(native_adapter::name_diagnostic)?;
        if path.anchor() == PathAnchor::SelfType {
            return Ok(false);
        }
        for end in 1..path.len() {
            self.alias_budget.charge_work(1)?;
            let prefix = path
                .semantic_slice(self.names(), 0..end, path.anchor())
                .map_err(native_adapter::name_diagnostic)?;
            let Some(owner) = self.path(&prefix, module, true, false)? else {
                return Ok(false);
            };
            if owner.kind != ResolvedKind::Module {
                return Ok(false);
            }
        }
        Ok(self
            .path(path, module, true, false)?
            .is_some_and(|item| matches!(item.path, ResolvedPath::ManuallyDrop)))
    }

    pub(super) fn manually_drop_type(
        &mut self,
        ty: &Type,
        module: ModuleId,
    ) -> Result<Option<Box<SourceManuallyDrop>>, String> {
        let path = match ty {
            Type::Apply(path) => {
                let occurrence = path.path(self.names())?;
                if !self.is_manually_drop_path(&occurrence, module)? {
                    return Ok(None);
                }
                source_clone::clone_path(path, &self.alias_budget)?;
                path.clone()
            }
            Type::Named(path) if self.is_manually_drop_path(path, module)? => {
                self.alias_source_path(path)?
            }
            _ => return Ok(None),
        };
        let occurrence = path.path(self.names())?;
        if !self.is_manually_drop_path(&occurrence, module)? {
            return Ok(None);
        }
        let last = path.segments.last().ok_or("empty ManuallyDrop type path")?;
        if path.segments[..path.segments.len() - 1]
            .iter()
            .any(|segment| {
                !segment.args.is_empty()
                    || !segment.region_args.is_empty()
                    || !segment.associated_types.is_empty()
            })
        {
            return Err("generic arguments are not allowed on ManuallyDrop modules".into());
        }
        if !last.region_args.is_empty() {
            return Err("unsupported: ManuallyDrop lifetime actuals".into());
        }
        if !last.associated_types.is_empty() {
            return Err("ManuallyDrop has no associated type arguments".into());
        }
        if last.args.len() != 1 {
            return Err("E0107: ManuallyDrop requires exactly one type argument".into());
        }
        walk_depth(DepthNode::Type(&last.args[0]), Some(&self.alias_budget))?;
        let mut argument = last.args[0].clone();
        self.ty(&mut argument, module)?;
        self.alias_budget.retain_bytes(
            std::mem::size_of::<SourceCatalogType>()
                + std::mem::size_of::<GenericPath>()
                + 2 * std::mem::size_of::<usize>()
                + std::mem::size_of::<Type>(),
        )?;
        Ok(Some(Box::new(SourceCatalogType {
            kind: SourceCatalogKind::ManuallyDrop,
            original: Arc::new(path),
            occurrence,
            argument: Some(Box::new(argument)),
        })))
    }

    pub(super) fn resolve_manually_drop_call(
        &mut self,
        expr: &mut Expr,
        module: ModuleId,
        locals: &mut Vec<std::collections::HashSet<LocalKey>>,
    ) -> Result<bool, String> {
        let dedicated_call = matches!(expr, Expr::Call { .. })
            || matches!(expr, Expr::Apply { value, .. } if matches!(value.as_ref(), Expr::Call { .. }));
        let mut path = match expr {
            Expr::Apply { path, value }
                if matches!(value.as_ref(), Expr::Call { .. } | Expr::Var(_)) =>
            {
                source_clone::clone_path(path, &self.alias_budget)?;
                path.clone()
            }
            Expr::Call { name, .. } => self.alias_source_path(name)?,
            Expr::Var(path) if path.len() >= 2 => self.alias_source_path(path)?,
            _ => return Ok(false),
        };
        if path.segments.len() < 2 || path.anchor == PathAnchor::SelfType {
            return Ok(false);
        }
        let original = path.path(self.names())?;
        let owner = original
            .semantic_parent(self.names())
            .map_err(native_adapter::name_diagnostic)?;
        if !self.is_manually_drop_path(&owner, module)? {
            return Ok(false);
        }
        if !dedicated_call {
            return Err("unsupported: first-class ManuallyDrop operations require a canonical executable provider".into());
        }
        let member = path
            .segments
            .last()
            .ok_or("missing ManuallyDrop operation")?;
        if !member.args.is_empty()
            || !member.region_args.is_empty()
            || !member.associated_types.is_empty()
        {
            return Err("E0107: ManuallyDrop operation takes no method arguments".into());
        }
        let operation = if self.program.symbols.matches(
            self.names(),
            member.name,
            BuiltinAtom::ManuallyDropNew,
        )? {
            ManuallyDropOperation::New
        } else if self.program.symbols.matches(
            self.names(),
            member.name,
            BuiltinAtom::ManuallyDropIntoInner,
        )? {
            ManuallyDropOperation::IntoInner
        } else if self.program.symbols.matches(
            self.names(),
            member.name,
            BuiltinAtom::ManuallyDropDrop,
        )? {
            ManuallyDropOperation::Drop
        } else {
            return Err(
                "unsupported: canonical ManuallyDrop operation has no executable provider".into(),
            );
        };
        let at = path.segments.len() - 2;
        for segment in &path.segments[..at] {
            if !segment.args.is_empty()
                || !segment.region_args.is_empty()
                || !segment.associated_types.is_empty()
            {
                return Err("generic arguments are not allowed on ManuallyDrop modules".into());
            }
        }
        let owner_segment = &path.segments[at];
        if !owner_segment.region_args.is_empty() {
            return Err("unsupported: ManuallyDrop lifetime actuals".into());
        }
        if !owner_segment.associated_types.is_empty() || owner_segment.args.len() > 1 {
            return Err("E0107: ManuallyDrop requires one type argument".into());
        }
        let argument = if let Some(argument) = owner_segment.args.first() {
            walk_depth(DepthNode::Type(argument), Some(&self.alias_budget))?;
            let mut actual = argument.clone();
            self.ty(&mut actual, module)?;
            self.alias_budget
                .retain_bytes(std::mem::size_of::<Type>())?;
            Some(Box::new(actual))
        } else {
            None
        };
        let value = match &mut *expr {
            Expr::Apply { value, .. } => value.as_mut(),
            other => other,
        };
        let Expr::Call { args, .. } = value else {
            return Err("ManuallyDrop operation lost original call".into());
        };
        if args.len() != 1 {
            return Err("E0061: ManuallyDrop operation requires exactly one value".into());
        }
        self.alias_budget.retain_bytes(
            std::mem::size_of::<SourceManuallyDropCall>()
                + std::mem::size_of::<SourceCatalogType>()
                + std::mem::size_of::<Type>()
                + std::mem::size_of::<Expr>()
                + std::mem::size_of::<GenericPath>()
                + 2 * std::mem::size_of::<usize>(),
        )?;
        let mut value = args
            .pop()
            .ok_or("missing ManuallyDrop operation argument")?;
        self.expr(&mut value, module, locals)?;
        // The owner arguments remain the original syntax. Only the separate
        // mapped argument is specialized or inferred by active checking.
        path.resolved_use = Some(original.clone());
        *expr = Expr::ManuallyDrop(Box::new(SourceManuallyDropCall {
            source: Box::new(SourceCatalogType {
                kind: SourceCatalogKind::ManuallyDrop,
                original: Arc::new(path),
                occurrence: original,
                argument,
            }),
            operation,
            value: Box::new(value),
        }));
        Ok(true)
    }
}
