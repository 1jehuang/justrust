//! Source-only demand descriptions. These are not type or region permissions.
use super::NominalShape as Shape;
use super::*;

impl PlanCtx {
    pub(in crate::compiler) fn nominal_source_shape(
        &self,
        declaration: ir::DeclId,
        ty: &Type,
    ) -> Result<Option<Shape>, String> {
        let info = self.registry.declaration(declaration)?;
        let bindings = generic_bindings(&self.registry, &info.env, &self.budget)?;
        self.nominal_source_shape_inner(ty, &bindings, 0)
            .map(Some)
            .map_err(|error| {
                format!(
                    "{error}; source recipe callable={} ({declaration:?})",
                    info.name
                )
            })
    }

    fn nominal_source_shape_inner(
        &self,
        ty: &Type,
        bindings: &HashMap<GenericBindingKey, ir::Ty>,
        depth: usize,
    ) -> Result<Shape, String> {
        self.budget.check_depth(depth)?;
        self.budget.charge_work(1)?;
        let child = |ty: &Type| self.nominal_source_shape_inner(ty, bindings, depth + 1);
        Ok(match ty {
            Type::Int(n) => Shape::Leaf(ir::Ty::Int(*n)),
            Type::Float(n) => Shape::Leaf(ir::Ty::Float(*n)),
            Type::Bool => Shape::Leaf(ir::Ty::Bool), Type::Unit => Shape::Leaf(ir::Ty::Unit),
            Type::Never => Shape::Leaf(ir::Ty::Never), Type::Char => Shape::Leaf(ir::Ty::Char),
            Type::Str => Shape::Leaf(ir::Ty::Str),
            Type::Param(name) => Shape::Leaf(bindings.get(&self.registry.parameter_key(*name)?)
                .cloned().ok_or("source nominal recipe has a foreign generic parameter")?),
            Type::SelfType => Shape::Leaf(bindings.get(&GenericBindingKey::SelfType)
                .cloned().ok_or("unsupported: source nominal recipe needs exact impl Self metadata")?),
            Type::Ref {kind, referent, ..} => Shape::Ref(*kind, Arc::new(child(referent)?)),
            Type::RawPtr {kind, pointee} => Shape::RawPtr(*kind, Arc::new(child(pointee)?)),
            Type::Array(element,len) => Shape::Array(Arc::new(child(element)?),*len),
            Type::Slice(element) => Shape::Slice(Arc::new(child(element)?)),
            Type::Tuple(types) => {
                let mut fields=Vec::new(); fields.try_reserve_exact(types.len()).map_err(|_|"source recipe tuple allocation failed")?;
                self.budget.retain_bytes(types.len().checked_mul(std::mem::size_of::<Shape>()).ok_or("source recipe tuple size overflow")?)?;
                for ty in types { fields.push(child(ty)?); }
                Shape::Tuple(fields.into())
            }
            Type::Named(path) => {
                let declaration=*self.registry.nominals.get(&name_scope::path_key(self.registry.names.as_ref(),path,Namespace::Type)?)
                    .ok_or("unsupported: source recipe path is not an authenticated nominal declaration")?;
                let info=self.registry.declaration(declaration)?;
                if info.param_env.arity()!=0{return Err("source nominal recipe omitted generic arguments".into());}
                Shape::Nominal(declaration,Arc::from([]))
            }
            Type::Apply(path) => {
                if path.segments.iter().any(|s|!s.associated_types.is_empty()){
                    return Err("unsupported: associated equalities in nominal source recipe".into());
                }
                let (at,segment)=path.segments.iter().enumerate().find(|(_,s)|!s.args.is_empty()||!s.region_args.is_empty())
                    .ok_or("nominal source recipe lacks its written argument segment")?;
                if at+1!=path.segments.len(){return Err("unsupported: source recipe generic associated type".into());}
                let declaration=*self.registry.nominals.get(&name_scope::path_key(self.registry.names.as_ref(),&path.path(self.registry.names.as_ref())?,Namespace::Type)?)
                    .ok_or("unsupported: source recipe generic path is not a nominal declaration")?;
                let info=self.registry.declaration(declaration)?;
                if info.param_env.arity()!=segment.args.len(){return Err("nominal source recipe argument arity changed".into());}
                let mut args=Vec::new();args.try_reserve_exact(segment.args.len()).map_err(|_|"nominal recipe actual allocation failed")?;
                self.budget.retain_bytes(segment.args.len().checked_mul(std::mem::size_of::<Shape>()).ok_or("nominal recipe actual size overflow")?)?;
                for (index, ty) in segment.args.iter().enumerate() {
                    args.push(child(ty).map_err(|error| format!("{error}; nominal={declaration:?} argument={index}"))?);
                }
                Shape::Nominal(declaration,args.into())
            }
            Type::Projection(_) => return Err("unsupported: source projection recipe needs retained occurrence lowering capability".into()),
            Type::Dyn(path) => {
                // A closed principal is a rigid leaf: its actuals and associated
                // assignments are lowered through the canonical dynamic metadata
                // path and admitted only when no parameter or inference remains.
                let lowered = {
                    let mut state = self.scratch();
                    state.source_dynamic_type(
                        &self.registry,
                        path,
                        bindings,
                        ir::FactsCtx::Concrete,
                        &self.budget,
                    )?
                };
                match lowered {
                    ir::Ty::Dyn(object) if object.is_closed_leaf() => Shape::Leaf(ir::Ty::Dyn(object)),
                    _ => return Err("unsupported: generic dynamic source nominal recipe".into()),
                }
            }
            Type::Opaque(_) => return Err("unsupported: opaque source nominal recipe".into()),
            Type::Infer => return Err("unsupported: unresolved source type recipe requires solver-owned inference".into()),
        })
    }
}
