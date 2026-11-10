//! Deferred opaque metadata substitution for body-local inference. The retained
//! descriptor supplies identity only. Its children remain ordinary solver terms
//! until caller arguments resolve, then normal checking authenticates the result.
use super::*;

impl Solver<'_, '_, '_> {
    /// A same-spelling method is not invocation authority. Only conclude
    /// absence when every retained bound is a non-callable source trait with
    /// no inherited bounds. Other descriptors remain on ordinary selection.
    pub(super) fn opaque_excludes_callable_bounds(
        &self,
        descriptor: &ir::OpaqueTy,
    ) -> Result<bool, String> {
        let plan = self.checker.signatures.plan()?;
        let world = plan.trait_world()?;
        for bound in descriptor.bounds() {
            self.checker.infer_work(0)?;
            if world
                .closure_trait_kind(bound.declaration())
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
                .is_some()
            {
                return Ok(false);
            }
            let declaration = plan.declaration(bound.declaration())?;
            if declaration.kind != ir::DeclKind::Trait
                || declaration.source_index == usize::MAX
                || !plan
                    .registry
                    .source_trait(bound.declaration())?
                    .supertraits
                    .is_empty()
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
    /// Use only the declaration-owned callable bound's input/output terms.
    /// Unrelated owner actuals (notably inferred closure representations) need
    /// not be materialized to constrain a call. Checked lowering still proves
    /// the complete opaque descriptor and canonical invocation signature.
    pub(super) fn opaque_callable_application(
        &mut self,
        descriptor: &Arc<ir::OpaqueTy>,
        children: &[Term],
        args: &[Term],
    ) -> Result<Option<Term>, String> {
        let plan = self.checker.signatures.plan()?;
        let world = plan.trait_world()?;
        if descriptor.types().count() != children.len() {
            return Err("opaque inference child count changed".into());
        }
        for mode in [
            ir::ClosureKind::Fn,
            ir::ClosureKind::FnMut,
            ir::ClosureKind::FnOnce,
        ] {
            let mut offset = descriptor.owner_args().len();
            for bound in descriptor.bounds() {
                self.checker.infer_work(0)?;
                let argument_types = bound
                    .args()
                    .iter()
                    .filter(|argument| matches!(argument, ir::OpaqueBoundArg::Type(_)))
                    .count();
                let next = offset
                    .checked_add(argument_types)
                    .and_then(|offset| offset.checked_add(bound.equalities().len()))
                    .ok_or("opaque bound inference index overflow")?;
                if next > children.len() {
                    return Err("opaque bound inference child count changed".into());
                }
                if world
                    .closure_trait_kind(bound.declaration())
                    .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
                    == Some(mode)
                    && bound.args().len() == 1
                    && argument_types == 1
                {
                    let mut output = None;
                    for (index, (item, _)) in bound.equalities().iter().enumerate() {
                        self.checker.infer_work(0)?;
                        let declaration = plan.declaration(*item)?;
                        let callable_output = declaration.kind == ir::DeclKind::AssociatedType
                            && declaration.owner == Some(bound.declaration())
                            && world
                                .closure_trait_kind(bound.declaration())
                                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
                                == Some(mode)
                            && {
                                let owner = plan.declaration(bound.declaration())?;
                                let source = plan.registry.source_trait(bound.declaration())?;
                                let associated = source
                                    .associated_types
                                    .get(declaration.source_index)
                                    .ok_or("callable output source member is missing")?;
                                plan.registry
                                    .trait_associated_types
                                    .get(owner.source_index)
                                    .and_then(|items| items.get(declaration.source_index))
                                    == Some(item)
                                    && plan
                                        .program
                                        .names
                                        .spelling(associated.name)
                                        .map_err(native_adapter::name_diagnostic)?
                                        == "Output"
                            };
                        if callable_output {
                            if output.is_some() {
                                return Err("opaque callable output equality is duplicated".into());
                            }
                            output = Some(children[offset + argument_types + index]);
                        }
                    }
                    if let Some(output) = output {
                        self.checker.infer_storage(args.len())?;
                        let actual = if args.is_empty() {
                            self.fixed(ir::Ty::Unit)?
                        } else {
                            self.node(Domain::Tuple(args.to_vec()))?
                        };
                        self.coerce(actual, children[offset])?;
                        return Ok(Some(output));
                    }
                }
                offset = next;
            }
        }
        Ok(None)
    }

    /// A defining body that returns a closure for its own `impl Fn..` result:
    /// the declared callable bound supplies the closure's argument tuple and
    /// `Output`, exactly as a generic callee's bound would (`let mut c = 0;
    /// move || { c += 1; c }` under `impl FnMut() -> u32`). A hint only: the
    /// checker still proves every declared bound for the hidden type.
    /// Returns whether a returned closure was found and constrained.
    pub(super) fn opaque_return_callable_bounds(&mut self) -> Result<bool, String> {
        let Some(term) = self.opaque_return else {
            return Ok(false);
        };
        let Domain::Callable(callable) = self.domain(term) else {
            return Ok(false);
        };
        let ir::Ty::Opaque(opaque) = self.checker.result.clone() else {
            return Ok(false);
        };
        let plan = self.checker.signatures.plan()?;
        let world = plan.trait_world()?;
        for bound in opaque.bounds() {
            self.checker.infer_work(0)?;
            if world
                .closure_trait_kind(bound.declaration())
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
                .is_none()
            {
                continue;
            }
            let [ir::OpaqueBoundArg::Type(arguments)] = bound.args() else {
                continue;
            };
            let arguments = self.fixed(arguments.clone())?;
            self.checker.infer_storage(callable.params.len())?;
            let tuple = self.node(Domain::Tuple(callable.params.clone()))?;
            self.unify(arguments, tuple)?;
            // The callable traits declare one associated type, `Output`.
            for (_, output) in bound.equalities() {
                self.checker.infer_work(0)?;
                let output = self.fixed(output.clone())?;
                self.coerce(callable.result, output)?;
            }
        }
        Ok(true)
    }

    /// A closure unsized to a callable trait object (`Box<dyn Fn(i32) -> i32>`,
    /// `&dyn FnMut() -> u32`): the object's written argument tuple and
    /// `Output` constrain the closure's parameters and result. A hint only:
    /// the checker proves the unsizing and the associated binding.
    pub(super) fn dyn_callable_hint(&mut self, actual: Term, object: Term) -> Result<(), String> {
        let Domain::Callable(callable) = self.domain(actual) else {
            return Ok(());
        };
        let Domain::Fixed(ir::Ty::Dyn(object)) = self.domain(object) else {
            return Ok(());
        };
        let plan = self.checker.signatures.plan()?;
        if plan
            .trait_world()?
            .closure_trait_kind(object.principal)
            .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
            .is_none()
        {
            return Ok(());
        }
        let [arguments] = object.args.as_ref() else {
            return Ok(());
        };
        let arguments = self.fixed(arguments.clone())?;
        self.checker.infer_storage(callable.params.len())?;
        let tuple = self.node(Domain::Tuple(callable.params.clone()))?;
        self.unify(arguments, tuple)?;
        // The callable traits declare one associated type, `Output`.
        for (_, output) in object.associated_types.iter() {
            self.checker.infer_work(0)?;
            let output = self.fixed(output.clone())?;
            self.coerce(callable.result, output)?;
        }
        Ok(())
    }

    /// Whether a cast from `source` to `target` is an unsizing coercion site:
    /// the target points to a trait object and the operand is already the same
    /// kind of pointer. Any other operand is left to the checker, which
    /// reports the non-primitive cast.
    pub(super) fn dyn_pointer_cast(&self, source: Term, target: Term) -> bool {
        match (self.domain(source), self.domain(target)) {
            (Domain::Ref(..), Domain::Ref(_, pointee)) => {
                matches!(self.domain(pointee), Domain::Fixed(ir::Ty::Dyn(_)))
            }
            (Domain::Nominal(have, _), Domain::Nominal(want, arguments)) => {
                have == want
                    && arguments.len() == 1
                    && matches!(self.domain(arguments[0]), Domain::Fixed(ir::Ty::Dyn(_)))
            }
            _ => false,
        }
    }

    pub(super) fn opaque_identity_matches(
        &self,
        left: &ir::OpaqueTy,
        right: &ir::OpaqueTy,
        depth: usize,
    ) -> Result<bool, String> {
        for _ in left
            .captured_regions()
            .iter()
            .chain(right.captured_regions())
        {
            self.checker.infer_work(depth)?;
        }
        for bound in left.bounds().iter().chain(right.bounds()) {
            self.checker.infer_work(depth)?;
            for _ in bound.args().iter() {
                self.checker.infer_work(depth)?;
            }
            for _ in bound.equalities() {
                self.checker.infer_work(depth)?;
            }
        }
        left.try_eq(right, |_, _| {
            self.checker.infer_work(depth)?;
            Ok(true)
        })
    }

    pub(super) fn opaque_schema(
        &mut self,
        descriptor: &Arc<ir::OpaqueTy>,
        params: &HashMap<ir::ParamId, Term>,
        depth: usize,
    ) -> Result<Term, String> {
        self.checker.infer_storage(descriptor.types().count())?;
        let children = descriptor
            .types()
            .map(|ty| self.schema(ty, params, depth + 1))
            .collect::<Result<Vec<_>, _>>()?;
        self.node(Domain::Opaque(descriptor.clone(), children))
    }

    pub(super) fn resolve_opaque(
        &self,
        descriptor: &Arc<ir::OpaqueTy>,
        children: &[Term],
        depth: usize,
    ) -> Result<Option<ir::Ty>, String> {
        self.checker.infer_storage(children.len())?;
        let mut actuals = Vec::with_capacity(children.len());
        for child in children {
            let Some(actual) = self.resolve(*child, depth + 1)? else {
                return Ok(None);
            };
            actuals.push(actual);
        }
        let mut actuals = actuals.into_iter();
        let mapped = descriptor.try_map(|_| {
            self.checker.infer_work(depth + 1)?;
            actuals
                .next()
                .ok_or_else(|| "opaque inference child count changed".to_string())
        })?;
        if actuals.next().is_some() {
            return Err("opaque inference retained extra children".into());
        }
        Ok(Some(ir::Ty::Opaque(Arc::new(mapped))))
    }
}
