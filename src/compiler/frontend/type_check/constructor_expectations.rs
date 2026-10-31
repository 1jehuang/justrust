//! Constructor expectation provenance, keyed by canonical nominal declarations.
use super::*;

impl Checker<'_> {
    pub(super) fn constructor_field_schema(
        &self,
        declaration: ir::DeclId,
        variant: Option<usize>,
        field: usize,
        expectation: Expectation<'_>,
    ) -> Result<InferTerm, String> {
        let plan = self.signatures.plan()?;
        plan.budget.charge_work(1)?;
        let (scope, nominal) = plan.nominal_schema(declaration, &plan.budget)?;
        let fields = match (&nominal, variant) {
            (ir::Ty::Struct(ty), None) => &ty.def().fields,
            (ir::Ty::Enum(ty), Some(index)) => {
                &ty.def()
                    .variants
                    .get(index)
                    .ok_or("unknown constructor variant schema")?
                    .fields
            }
            _ => return Err("constructor source schema kind mismatch".into()),
        };
        let fields = fields.read(&plan.budget)?;
        let field_index = field;
        let field = fields
            .get(field_index)
            .ok_or("unknown constructor field schema")?;
        let expected = match expectation {
            Expectation::HasType(ty) => Some(ty),
            _ => None,
        };
        if let Some(realized) = self.realized_constructor_expectation(declaration, expected)? {
            let concrete_fields = match (&realized, variant) {
                (ir::Ty::Struct(ty), None) => &ty.def().fields,
                (ir::Ty::Enum(ty), Some(index)) => {
                    &ty.def()
                        .variants
                        .get(index)
                        .ok_or("unknown realized constructor variant")?
                        .fields
                }
                _ => return Err("realized constructor source kind mismatch".into()),
            };
            let concrete_fields = concrete_fields.read(&plan.budget)?;
            if concrete_fields.len() != fields.len() {
                return Err("invalid metadata: realized constructor field topology drift".into());
            }
            let concrete = concrete_fields
                .get(field_index)
                .ok_or("unknown realized constructor field")?;
            return self.infer_type_term(&concrete.ty, None, 0);
        }
        let term = self.infer_type_term(&field.ty, Some(scope.binder()), 0)?;
        let arguments = self.constructor_source_arguments(declaration, expectation, 0)?;
        if let Some(arguments) = &arguments
            && arguments.len() != scope.arity()
        {
            return Err("constructor source argument arity mismatch".into());
        }
        self.replace_constructor_slots(&term, arguments.as_deref(), 0)
    }

    fn constructor_source_arguments(
        &self,
        declaration: ir::DeclId,
        expectation: Expectation<'_>,
        depth: usize,
    ) -> Result<Option<Vec<InferTerm>>, String> {
        self.infer_work(depth)?;
        let term = match expectation {
            Expectation::Application(application) => {
                let plan = self.signatures.plan()?;
                let arity = plan.param_env(declaration)?.arity();
                if application.arguments.len() < arity {
                    plan.registry
                        .nominal_source_arity(declaration, application.arguments.len())?;
                    if application
                        .arguments
                        .iter()
                        .any(partial_annotation::has_nested_infer)
                    {
                        return Err("unsupported: omitted nominal defaults after explicit source placeholders require solved-source default substitution".into());
                    }
                    // An explicit partial application fixes omitted defaults.
                    // Neither outer expectations nor field evidence may supply
                    // replacement actuals for those declaration-owned slots.
                    plan.budget.retain_bytes(
                        application
                            .arguments
                            .len()
                            .checked_mul(std::mem::size_of::<ir::Ty>())
                            .ok_or("constructor source default actual storage overflow")?,
                    )?;
                    let mut actuals = Vec::new();
                    actuals.try_reserve_exact(application.arguments.len())
                        .map_err(|_| "infrastructure: constructor source default actual allocation failed")?;
                    for source in application.arguments {
                        actuals.push(self.checked_type_actual(source)?);
                    }
                    let actuals = plan.complete_nominal_source_arguments(
                        declaration,
                        actuals,
                        self.facts(),
                        &plan.budget,
                    )?;
                    self.infer_storage(actuals.len())?;
                    let mut arguments = Vec::new();
                    arguments.try_reserve_exact(actuals.len()).map_err(
                        |_| "infrastructure: constructor source default term allocation failed",
                    )?;
                    for actual in &actuals {
                        arguments.push(self.infer_type_term(actual, None, depth + 1)?);
                    }
                    return Ok(Some(arguments));
                }
                let inherited =
                    self.constructor_source_arguments(declaration, application.outer, depth + 1)?;
                self.infer_storage(application.arguments.len())?;
                let mut arguments = Vec::new();
                arguments
                    .try_reserve_exact(application.arguments.len())
                    .map_err(|_| "infrastructure: constructor source term allocation failed")?;
                for (index, source) in application.arguments.iter().enumerate() {
                    let written = self.partial_type_term(source, depth + 1)?;
                    arguments.push(self.merge_constructor_context(
                        &written,
                        inherited.as_ref().and_then(|args| args.get(index)),
                        depth + 1,
                    )?);
                }
                return Ok(Some(arguments));
            }
            Expectation::HasType(ty) => self.infer_type_term(ty, None, depth + 1)?,
            Expectation::SourceSchema(_, term) => term.clone(),
            _ => return Ok(None),
        };
        match term.as_ref() {
            InferKind::Nominal(owner, arguments) if *owner == declaration => {
                self.infer_storage(arguments.len())?;
                Ok(Some(arguments.to_vec()))
            }
            _ => Ok(None),
        }
    }

    fn replace_constructor_slots(
        &self,
        term: &InferTerm,
        arguments: Option<&[InferTerm]>,
        depth: usize,
    ) -> Result<InferTerm, String> {
        self.infer_work(depth)?;
        let kind = match term.as_ref() {
            InferKind::Slot(index) => {
                return match arguments.and_then(|args| args.get(*index)) {
                    Some(argument) => Ok(argument.clone()),
                    None => self.infer_node(InferKind::Unknown(false)),
                };
            }
            InferKind::Ref(kind, inner) => InferKind::Ref(
                *kind,
                self.replace_constructor_slots(inner, arguments, depth + 1)?,
            ),
            InferKind::RawPtr(kind, inner) => InferKind::RawPtr(
                *kind,
                self.replace_constructor_slots(inner, arguments, depth + 1)?,
            ),
            InferKind::Array(inner, len) => InferKind::Array(
                self.replace_constructor_slots(inner, arguments, depth + 1)?,
                *len,
            ),
            InferKind::Slice(inner) => {
                InferKind::Slice(self.replace_constructor_slots(inner, arguments, depth + 1)?)
            }
            InferKind::PhantomData(declaration, argument) => InferKind::PhantomData(
                *declaration,
                self.replace_constructor_slots(argument, arguments, depth + 1)?,
            ),
            InferKind::ManuallyDrop(declaration, argument) => InferKind::ManuallyDrop(
                *declaration,
                self.replace_constructor_slots(argument, arguments, depth + 1)?,
            ),
            InferKind::Cell(declaration, argument) => InferKind::Cell(
                *declaration,
                self.replace_constructor_slots(argument, arguments, depth + 1)?,
            ),
            InferKind::FnPointer(inputs, output) => {
                let budget = &self.signatures.plan()?.budget;
                let mut mapped = ir::FnPointerTy::storage(inputs.len(), budget)?;
                for t in inputs.iter() {
                    mapped.push(self.replace_constructor_slots(t, arguments, depth + 1)?);
                }
                InferKind::FnPointer(
                    ir::FnPointerTy::slice(mapped, budget)?,
                    self.replace_constructor_slots(output, arguments, depth + 1)?,
                )
            }
            InferKind::Tuple(fields) => {
                self.infer_storage(fields.len())?;
                InferKind::Tuple(
                    fields
                        .iter()
                        .map(|field| self.replace_constructor_slots(field, arguments, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                )
            }
            InferKind::Nominal(owner, fields) => {
                self.infer_storage(fields.len())?;
                InferKind::Nominal(
                    *owner,
                    fields
                        .iter()
                        .map(|field| self.replace_constructor_slots(field, arguments, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                )
            }
            _ => return Ok(term.clone()),
        };
        self.infer_node(kind)
    }

    fn merge_constructor_context(
        &self,
        written: &InferTerm,
        inherited: Option<&InferTerm>,
        depth: usize,
    ) -> Result<InferTerm, String> {
        self.infer_work(depth)?;
        let Some(inherited) = inherited else {
            return Ok(written.clone());
        };
        match (written.as_ref(), inherited.as_ref()) {
            (InferKind::Unknown(_), _) => Ok(inherited.clone()),
            (InferKind::Nominal(owner, fields), InferKind::Nominal(other, values))
                if owner == other && fields.len() == values.len() =>
            {
                self.infer_storage(fields.len())?;
                let fields = fields
                    .iter()
                    .zip(values.iter())
                    .map(|(written, inherited)| {
                        self.merge_constructor_context(written, Some(inherited), depth + 1)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.infer_node(InferKind::Nominal(*owner, fields.into()))
            }
            (InferKind::Tuple(fields), InferKind::Tuple(values))
                if fields.len() == values.len() =>
            {
                self.infer_storage(fields.len())?;
                let fields = fields
                    .iter()
                    .zip(values.iter())
                    .map(|(written, inherited)| {
                        self.merge_constructor_context(written, Some(inherited), depth + 1)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.infer_node(InferKind::Tuple(fields.into()))
            }
            (InferKind::Array(inner, len), InferKind::Array(value, actual)) if len == actual => {
                let inner = self.merge_constructor_context(inner, Some(value), depth + 1)?;
                self.infer_node(InferKind::Array(inner, *len))
            }
            (InferKind::Slice(inner), InferKind::Slice(value)) => {
                let inner = self.merge_constructor_context(inner, Some(value), depth + 1)?;
                self.infer_node(InferKind::Slice(inner))
            }
            // Reference shape is already firmly written, regardless of holes
            // below the referent. No reference-value move occurs inside it.
            _ => Ok(written.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_partial_constructor_defaults_use_source_slots_not_outer_or_field_inference() {
        let source = parse("struct Pair<A=i32,B=A,C=B>{a:A,b:B,c:C}fn main(){let fixed=Pair::<i32>{a:1,b:2,c:4};let partial=Pair::<_>{a:1,b:2,c:4};let full=Pair::<_,_,_>{a:1,b:2,c:4};}").unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&source, &budget).unwrap();
        let main = plan.function_decl(0).unwrap();
        let signatures = plan.environment(Some(main)).unwrap();
        let checker = Checker::new(&signatures, ir::Ty::Unit, source.functions[0].module);
        let declaration = plan.nominal_declaration(0).unwrap();
        let source_arguments = |index: usize| {
            let Stmt::Let {
                value: Expr::Apply { path, .. },
                ..
            } = &source.functions[0].body.statements[index]
            else {
                panic!("missing original explicit constructor source")
            };
            path.application().unwrap().1
        };
        let outer = plan
            .intern_nominal(
                declaration,
                &[ir::Ty::Int(ir::IntTy::I32), ir::Ty::Bool, ir::Ty::Bool],
                checker.facts(),
                &budget,
            )
            .unwrap();
        let application = ApplicationExpectation {
            outer: Expectation::HasType(&outer),
            arguments: source_arguments(0),
        };
        let arguments = checker
            .constructor_source_arguments(declaration, Expectation::Application(&application), 0)
            .unwrap()
            .unwrap();
        assert_eq!(arguments.len(), 3);
        assert!(
            arguments
                .iter()
                .all(|term| matches!(term.as_ref(), InferKind::Rigid(ir::Ty::Int(ir::IntTy::I32))))
        );
        assert!(
            checker
                .constructor_source_arguments(declaration, Expectation::None, 0)
                .unwrap()
                .is_none()
        );
        let partial = ApplicationExpectation {
            outer: Expectation::None,
            arguments: source_arguments(1),
        };
        let error = checker
            .constructor_source_arguments(declaration, Expectation::Application(&partial), 0)
            .unwrap_err();
        assert!(error.contains("unsupported"), "{error}");
        let full = ApplicationExpectation {
            outer: Expectation::None,
            arguments: source_arguments(2),
        };
        let arguments = checker
            .constructor_source_arguments(declaration, Expectation::Application(&full), 0)
            .unwrap()
            .unwrap();
        assert_eq!(arguments.len(), 3);
        assert!(
            arguments
                .iter()
                .all(|term| matches!(term.as_ref(), InferKind::Unknown(_)))
        );
    }
}
