//! Exact selected-to-canonical target transport for opaque physical dispatch.
use super::*;

#[derive(Clone, Debug)]
pub(super) struct Transport {
    selected: ir::CallTarget,
    execution: ir::CallTarget,
    engine: crate::engine::EngineAuthority,
}

impl Transport {
    pub(super) async fn from_selection(
        selection: &traits::ResolvedTraitCall,
        facts: &Facts<'_>,
        types: &frontend::PlanCtx,
        caller: &traits::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<Self, Error> {
        let ir::Callee::Direct(selected) = selection.callee() else {
            return Err(Error::new(
                "opaque execution transport requires a direct selection",
            ));
        };
        let words = instances::graph_key::encode(selected, budget).map_err(Error::new)?;
        let execution = facts.decode_instance(&words, types, budget).await?;
        let transport = Self {
            selected: selected.clone(),
            execution,
            engine: facts.cx.authority(),
        };
        transport.replay(selection, caller, facts.cx.authority(), budget)?;
        Ok(transport)
    }

    pub(super) fn selected(&self) -> &ir::CallTarget {
        &self.selected
    }

    pub(super) fn execution(&self) -> &ir::CallTarget {
        &self.execution
    }

    pub(super) fn replay(
        &self,
        selection: &traits::ResolvedTraitCall,
        caller: &traits::ValidatedFacts,
        engine: crate::engine::EngineAuthority,
        budget: &ir::PlanBudget,
    ) -> Result<(), Error> {
        if selection.callee() != &ir::Callee::Direct(self.selected.clone()) {
            return Err(Error::new(
                "opaque execution transport changed the exact selection",
            ));
        }
        self.replay_arguments(caller, engine, budget)
    }

    fn replay_arguments(
        &self,
        caller: &traits::ValidatedFacts,
        engine: crate::engine::EngineAuthority,
        budget: &ir::PlanBudget,
    ) -> Result<(), Error> {
        if self.engine != engine
            || caller.phase() != traits::FactsPhase::Closed
            || !caller.budget().same_authority(budget)
        {
            return Err(Error::new(
                "opaque execution transport changed engine, phase or budget",
            ));
        }
        if self.selected.declaration != self.execution.declaration
            || self.selected.type_args.len() != self.execution.type_args.len()
            || instances::graph_key::encode(&self.selected, budget).map_err(Error::new)?
                != instances::graph_key::encode(&self.execution, budget).map_err(Error::new)?
        {
            return Err(Error::new(
                "opaque execution transport changed canonical target key",
            ));
        }
        let mut scratch = traits::TraitScratch::default();
        for (selected, execution) in self
            .selected
            .type_args
            .iter()
            .zip(self.execution.type_args.iter())
        {
            current_engine_type(selected, caller, engine, budget, &mut scratch, 0)?;
            current_engine_type(execution, caller, engine, budget, &mut scratch, 0)?;
            if !source_replay::replay_types_equal(selected, execution, budget)
                .map_err(|e| Error::new(e.to_string()))?
            {
                return Err(Error::new(
                    "opaque execution transport changed ordered actual authority",
                ));
            }
        }
        Ok(())
    }
}

fn current_engine_type(
    ty: &ir::Ty,
    caller: &traits::ValidatedFacts,
    engine: crate::engine::EngineAuthority,
    budget: &ir::PlanBudget,
    scratch: &mut traits::TraitScratch,
    depth: usize,
) -> Result<(), Error> {
    budget.check_depth(depth).map_err(Error::new)?;
    budget.charge_work(1).map_err(Error::new)?;
    let next = depth
        .checked_add(1)
        .ok_or_else(|| Error::new("opaque execution type depth overflow"))?;
    macro_rules! child {
        ($ty:expr) => {
            current_engine_type($ty, caller, engine, budget, scratch, next)
        };
    }
    match ty {
        ir::Ty::Struct(n) => {
            if n.id().index.is_graph() {
                let read = n.realization().ok_or_else(|| {
                    Error::new("opaque execution graph actual omitted typed read")
                })?;
                read.retain_for_substitution_with_scratch(
                    ty,
                    engine,
                    ir::FactsCtx::from_validated(caller),
                    budget,
                    scratch,
                )
                .map_err(Error::new)?;
                if let Some(height) = read.component_metadata_height() {
                    budget
                        .check_depth(depth.checked_add(height).ok_or_else(|| {
                            Error::new("opaque execution component depth overflow")
                        })?)
                        .map_err(Error::new)?;
                    return Ok(());
                }
            }
            if n.is_back_edge() {
                return Err(Error::new(
                    "unsupported: recursive opaque execution source actual",
                ));
            }
            for ty in n.def().type_args.iter() {
                child!(ty)?;
            }
            for field in n.def().fields.read(budget).map_err(Error::new)?.iter() {
                child!(&field.ty)?;
            }
        }
        ir::Ty::Enum(n) => {
            if n.id().index.is_graph() {
                let read = n.realization().ok_or_else(|| {
                    Error::new("opaque execution graph actual omitted typed read")
                })?;
                read.retain_for_substitution_with_scratch(
                    ty,
                    engine,
                    ir::FactsCtx::from_validated(caller),
                    budget,
                    scratch,
                )
                .map_err(Error::new)?;
                if let Some(height) = read.component_metadata_height() {
                    budget
                        .check_depth(depth.checked_add(height).ok_or_else(|| {
                            Error::new("opaque execution component depth overflow")
                        })?)
                        .map_err(Error::new)?;
                    return Ok(());
                }
            }
            if n.is_back_edge() {
                return Err(Error::new(
                    "unsupported: recursive opaque execution source actual",
                ));
            }
            for ty in n.def().type_args.iter() {
                child!(ty)?;
            }
            for variant in n.def().variants.iter() {
                for field in variant.fields.read(budget).map_err(Error::new)?.iter() {
                    child!(&field.ty)?;
                }
            }
        }
        ir::Ty::Ref { referent: ty, .. }
        | ir::Ty::RawPtr { pointee: ty, .. }
        | ir::Ty::Array { element: ty, .. }
        | ir::Ty::Slice { element: ty } => child!(ty)?,
        ir::Ty::Tuple(types) => {
            for ty in types.iter() {
                child!(ty)?;
            }
        }
        ir::Ty::Closure(closure) => {
            for ty in closure.types() {
                child!(ty)?;
            }
        }
        ir::Ty::FnItem(target) => {
            for ty in target.type_args.iter() {
                child!(ty)?;
            }
        }
        ir::Ty::FnPointer(pointer) => {
            for ty in pointer.inputs.iter() {
                child!(ty)?;
            }
            child!(&pointer.output)?;
        }
        ir::Ty::PhantomData(marker) => child!(marker.argument())?,
        ir::Ty::ManuallyDrop(wrapper) => child!(wrapper.argument())?,
        ir::Ty::Cell(cell) => child!(cell.argument())?,
        ir::Ty::Dyn(object) => {
            for ty in object.types() {
                child!(ty)?;
            }
        }
        ir::Ty::Opaque(opaque) => {
            for ty in opaque.owner_args() {
                child!(ty)?;
            }
            for bound in opaque.bounds() {
                for arg in bound.args() {
                    if let ir::OpaqueBoundArg::Type(ty) = arg {
                        child!(ty)?;
                    }
                }
                for (_, ty) in bound.equalities() {
                    child!(ty)?;
                }
            }
        }
        ir::Ty::DependentArray { .. } | ir::Ty::AssociatedType(_) | ir::Ty::Param(_) => {
            return Err(Error::new("unsupported: nonclosed opaque execution actual"));
        }
        ir::Ty::Int(_)
        | ir::Ty::Char
        | ir::Ty::Str
        | ir::Ty::Float(_)
        | ir::Ty::Bool
        | ir::Ty::Unit
        | ir::Ty::Never
        | ir::Ty::Const(_) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_execution_target_replays_typed_reads_and_rejects_foreign_engine() {
        struct Probe(CompilerProvider);
        impl Provider for Probe {
            fn outputs(&self, key: &Key) -> usize {
                self.0.outputs(key)
            }
            fn start(&self, cx: Context, key: Key) -> TaskFuture {
                if key.kind != 9011 {
                    return self.0.start(cx, key);
                }
                let provider = self.0.clone();
                Box::pin(async move {
                    let facts = Facts { cx: &cx };
                    let types = facts.types().await?;
                    let template: Arc<TemplateBodyFact> =
                        facts.get(keys::item(TEMPLATE_BODY, 1)).await?;
                    let publication = template
                        .publication
                        .as_ref()
                        .ok_or_else(|| Error::new("fixture omitted original publication"))?;
                    let (occurrence, call) = publication
                        .source()
                        .exprs
                        .iter()
                        .enumerate()
                        .find_map(|(index, expression)| {
                            if let ir::ExprKind::Call {
                                target: ir::Callee::TraitItem(call),
                                ..
                            } = &expression.kind
                            {
                                if matches!(&call.trait_ref.self_ty, ir::Ty::Opaque(_)) {
                                    return Some((ir::ExprId(index), call));
                                }
                            }
                            None
                        })
                        .ok_or_else(|| Error::new("fixture omitted original opaque call"))?;
                    let budget = &provider.0.budget;
                    let caller = ir::CallTarget {
                        declaration: types.callable_decl(1).map_err(Error::new)?,
                        type_args: Arc::from([]),
                    };
                    let closed = types.instance_facts(&caller).map_err(Error::new)?;
                    let request = cx.request(super::super::key(&caller, occurrence, call, budget)?);
                    let authorization: Arc<super::super::OpaqueCallAuthorization> =
                        cx.read_fact(request.get(0)).await?;
                    let transport = &authorization.transport;
                    assert_ne!(
                        transport.selected, transport.execution,
                        "fixture must transport an actual graph nominal"
                    );
                    transport.replay_arguments(&closed, cx.authority(), budget)?;
                    let mut graph_pair = false;
                    for (selected, execution) in transport
                        .selected
                        .type_args
                        .iter()
                        .zip(transport.execution.type_args.iter())
                    {
                        if let (ir::Ty::Struct(a), ir::Ty::Struct(b)) = (selected, execution) {
                            if a.id().index.is_graph() && !b.id().index.is_graph() {
                                assert!(
                                    source_replay::replay_types_equal(selected, execution, budget)
                                        .map_err(|e| Error::new(e.to_string()))?
                                );
                                graph_pair = true;
                            }
                        }
                    }
                    assert!(
                        graph_pair,
                        "fixture must exercise Graph versus Source, not only stored engine equality"
                    );
                    let foreign = Engine::new(CompilerProvider(provider.0.clone()));
                    let mut changed = transport.clone();
                    changed.engine = foreign.authority();
                    let rejected = changed
                        .replay_arguments(&closed, foreign.authority(), budget)
                        .unwrap_err();
                    assert!(
                        rejected.to_string().contains("foreign engine"),
                        "{rejected}"
                    );
                    let mut changed = transport.clone();
                    changed.execution.declaration = caller.declaration;
                    assert!(
                        changed
                            .replay_arguments(&closed, cx.authority(), budget)
                            .is_err()
                    );
                    let mut changed = transport.clone();
                    changed.execution.type_args = Arc::from([]);
                    assert!(
                        changed
                            .replay_arguments(&closed, cx.authority(), budget)
                            .is_err()
                    );
                    let mut changed = transport.clone();
                    let mut args = changed.execution.type_args.to_vec();
                    assert!(args.len() >= 2);
                    args.swap(0, 1);
                    changed.execution.type_args = args.into();
                    assert!(
                        changed
                            .replay_arguments(&closed, cx.authority(), budget)
                            .is_err()
                    );
                    let ready: Arc<super::super::PhysicalCalleeReady> =
                        cx.read_fact(request.get(1)).await?;
                    assert_eq!(ready.target(), &transport.execution);
                    cx.publish_fact(0, ())
                })
            }
        }
        for workers in [1, 8] {
            let text = "fn selected(values:&[i32])->impl Iterator<Item=i32>+'_ {[0usize,1usize,2usize,3usize].into_iter().filter_map(move |i| {if i<values.len(){Some(values[i])}else{None}})}fn main(){let values=[4i32,5i32,6i32];let result=selected(&values).collect::<Vec<i32>>();assert!(result.len()==3);}";
            let root = std::path::PathBuf::from("/opaque-execution-transport");
            let source_snapshot = source::SourceSnapshot::new(
                root.clone(),
                [(
                    std::path::PathBuf::from("main.rs"),
                    text.as_bytes().to_vec(),
                )],
            )
            .unwrap();
            let snapshot = Arc::new(Snapshot {
                input: Input::File(root.join("main.rs")),
                source_options: source::SourceOptions {
                    source_snapshot: Some(source_snapshot),
                    compiler_image: Some(procfs::CompilerImage::current().unwrap()),
                    ..Default::default()
                },
                budget: ir::PlanBudget::default(),
            });
            let engine = Engine::new(Probe(CompilerProvider(snapshot)));
            let request = engine.request(keys::test_key(9011, vec![]));
            engine.run(workers).unwrap();
            request.completion().unwrap();
        }
    }
}
