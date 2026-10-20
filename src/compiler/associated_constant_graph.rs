//! Per-declaration associated constant metadata and universal initializer checks.
//! Closed member-use selection and value execution are separate graph demands.
use super::*;
mod qualification;
use qualification::QualifiedMemberUse;

#[derive(Clone, Debug)]
pub(super) struct AssociatedConstantHeaderFact {
    pub(super) header: traits::AssociatedConstHeader,
    pub(super) initializer: Option<Arc<const_source::DeclarationFact>>,
}
#[derive(Debug)]
pub(super) struct AssociatedConstantCheckedFact {
    pub(super) header: Arc<AssociatedConstantHeaderFact>,
    pub(super) body: Option<Arc<const_check::BodyFact>>,
    pub(super) qualification: Option<const_check::QualificationFact>,
}

impl AssociatedConstantCheckedFact {
    pub(super) fn validate(
        &self,
        plan: &frontend::PlanCtx,
        declaration: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<(), Error> {
        let expected = plan
            .associated_constant_header(declaration, budget)
            .map_err(Error::new)?;
        let actual = &self.header.header;
        if actual.declaration != expected.declaration
            || actual.owner != expected.owner
            || actual.owner_ty != expected.owner_ty
            || actual.implemented_member != expected.implemented_member
            || actual.ty != expected.ty
            || actual.predicates != expected.predicates
            || actual.has_initializer != expected.has_initializer
            || actual.env.scopes().len() != expected.env.scopes().len()
            || actual
                .env
                .scopes()
                .iter()
                .zip(expected.env.scopes())
                .any(|(a, b)| a.binder() != b.binder() || a.arity() != b.arity())
        {
            return Err(Error::new(
                "associated constant checked header identity changed",
            ));
        }
        match (&self.header.initializer, &self.body, &self.qualification) {
            (None, None, None) if !expected.has_initializer => Ok(()),
            (Some(source), Some(body), Some(proof))
                if expected.has_initializer
                    && source.declaration == declaration
                    && Arc::ptr_eq(source, body.source())
                    && Arc::ptr_eq(body, proof.body()) =>
            {
                source.validate_authority().map_err(Error::new)
            }
            _ => Err(Error::new(
                "associated constant checked initializer authority changed",
            )),
        }
    }
}

#[derive(Debug)]
pub(super) struct AssociatedConstantValueFact {
    pub(super) authorization: Arc<frontend::AuthorizedAssociatedConstant>,
    pub(super) value: Arc<const_check::ValueFact>,
}

fn same_caller_computation(
    left: &ir::CallTarget,
    right: &ir::CallTarget,
    budget: &ir::PlanBudget,
) -> Result<bool, String> {
    Ok(instances::graph_key::encode(left, budget)? == instances::graph_key::encode(right, budget)?)
}

impl AssociatedConstantValueFact {
    pub(super) fn closed_scalar(
        &self,
        caller: &traits::ValidatedFacts,
        operand: &ir::AssociatedConstOperand,
        budget: &ir::PlanBudget,
    ) -> Result<const_eval::ScalarValue, String> {
        let target = caller
            .target()
            .ok_or("associated constant value requires closed caller")?;
        if !same_caller_computation(target, self.authorization.caller(), budget)?
            || operand.caller() != target.declaration
            || operand.path() != self.authorization.path()
            || operand.member() != self.authorization.selected().member()
        {
            return Err(
                "associated constant graph value changed exact closed source occurrence".into(),
            );
        }
        self.authorization
            .selected()
            .validate_for(caller)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let selected = self.authorization.selected();
        let ty = const_source::associated_scalar_type(selected.ty())?;
        if self.value.scalar().declaration() != selected.declaration()
            || self.value.body().source().declaration != selected.declaration()
            || self.value.scalar().value().ty() != ty
            || self.value.body().source().ty != ty
        {
            return Err("associated constant graph value changed selected declaration/type".into());
        }
        self.value.body().source().validate_authority()?;
        Ok(self.value.scalar().value())
    }
    pub(super) fn validate(
        &self,
        plan: &frontend::PlanCtx,
        caller: &ir::CallTarget,
        path: &[u32],
        budget: &ir::PlanBudget,
    ) -> Result<(), Error> {
        // Nominal application IDs belong to one realization. A graph-key
        // round trip can reconstruct the same declaration and ordered arguments
        // with different local IDs, so compare the exact canonical computation.
        if !same_caller_computation(self.authorization.caller(), caller, budget)
            .map_err(Error::new)?
            || self.authorization.path() != path
        {
            return Err(Error::new(
                "associated constant value belongs to another caller occurrence",
            ));
        }
        self.authorization
            .validate(plan, budget)
            .map_err(Error::new)?;
        let selected = self.authorization.selected();
        let ty = const_source::associated_scalar_type(selected.ty()).map_err(Error::new)?;
        if self.value.scalar().declaration() != selected.declaration()
            || self.value.body().source().declaration != selected.declaration()
            || self.value.scalar().value().ty() != ty
            || self.value.body().source().ty != ty
        {
            return Err(Error::new(
                "associated constant value changed selected declaration/type",
            ));
        }
        self.value
            .body()
            .source()
            .validate_authority()
            .map_err(Error::new)
    }
}

fn initializer_dependency_path(
    parent: &[u32],
    input: &[u32],
    budget: &ir::PlanBudget,
) -> Result<Vec<u32>, Error> {
    if input.contains(&u32::MAX) || !input.len().is_multiple_of(2) {
        return Err(Error::new(
            "invalid original initializer dependency coordinate",
        ));
    }
    let marker = usize::from(!parent.contains(&u32::MAX));
    let length = parent
        .len()
        .checked_add(input.len())
        .and_then(|n| n.checked_add(1 + marker))
        .ok_or_else(|| Error::new("associated constant dependency path overflow"))?;
    budget.check_depth(length).map_err(Error::new)?;
    budget.charge_work(length).map_err(Error::new)?;
    let mut path = Vec::new();
    path.try_reserve_exact(length)
        .map_err(|_| Error::new("associated constant dependency path allocation"))?;
    path.extend_from_slice(parent);
    if marker != 0 {
        path.push(u32::MAX);
    }
    path.push(
        u32::try_from(input.len())
            .map_err(|_| Error::new("associated constant dependency coordinate length overflow"))?,
    );
    path.extend_from_slice(input);
    Ok(path)
}

pub(super) fn member_use_key(
    kind: u32,
    target: &ir::CallTarget,
    path: &[u32],
    budget: &ir::PlanBudget,
) -> Result<Key, Error> {
    let caller = instances::graph_key::encode(target, budget).map_err(Error::new)?;
    budget.check_depth(path.len()).map_err(Error::new)?;
    budget
        .charge_work(caller.len().saturating_add(path.len()))
        .map_err(Error::new)?;
    let mut params = Vec::new();
    params
        .try_reserve_exact(
            caller
                .len()
                .checked_add(path.len())
                .and_then(|n| n.checked_add(2))
                .ok_or_else(|| Error::new("associated constant key overflow"))?,
        )
        .map_err(|_| Error::new("associated constant key allocation"))?;
    params.push(caller.len() as u64);
    params.extend(caller);
    params.push(path.len() as u64);
    params.extend(path.iter().copied().map(u64::from));
    Ok(Key::new(kind, params))
}

impl CompilerProvider {
    pub(super) async fn associated_constant_use_node(
        &self,
        cx: &Context,
        key: &Key,
    ) -> Result<(), Error> {
        let facts = Facts { cx };
        let plan = facts.types().await?;
        let caller_len = usize::try_from(
            *key.params
                .first()
                .ok_or_else(|| Error::new("truncated associated constant use key"))?,
        )
        .map_err(|_| Error::new("associated constant caller frame overflow"))?;
        let caller_end = caller_len
            .checked_add(1)
            .ok_or_else(|| Error::new("associated constant caller frame overflow"))?;
        let caller_words = key
            .params
            .get(1..caller_end)
            .ok_or_else(|| Error::new("truncated associated constant caller frame"))?;
        let target = instances::graph_key::decode(caller_words, &plan, &self.0.budget)
            .map_err(Error::new)?;
        let path_len = usize::try_from(
            *key.params
                .get(caller_end)
                .ok_or_else(|| Error::new("missing associated constant occurrence frame"))?,
        )
        .map_err(|_| Error::new("associated constant occurrence frame overflow"))?;
        self.0.budget.check_depth(path_len).map_err(Error::new)?;
        let start = caller_end
            .checked_add(1)
            .ok_or_else(|| Error::new("associated constant occurrence frame overflow"))?;
        if key.params.len().checked_sub(start) != Some(path_len) {
            return Err(Error::new(
                "associated constant occurrence frame length changed",
            ));
        }
        let (mut path, _path_storage) = self.scratch_vec(path_len)?;
        for word in &key.params[start..] {
            path.push(
                u32::try_from(*word).map_err(|_| {
                    Error::new("associated constant occurrence coordinate overflow")
                })?,
            );
        }
        if member_use_key(key.kind, &target, &path, &self.0.budget)? != *key {
            return Err(Error::new("associated constant key is not canonical"));
        }
        if key.kind == ASSOCIATED_CONST_SELECT {
            let authorization = plan
                .authorize_associated_constant(&target, &path, &self.0.budget)
                .map_err(Error::new)?;
            let selected = authorization.selected();
            let member = selected.member();
            let declaration = selected.declaration();
            let member_handle = cx
                .request(Key::new(
                    ASSOCIATED_CONST_HEADER,
                    vec![member.snapshot.graph_word(), u64::from(member.index)],
                ))
                .get(0);
            let selected_handle = cx
                .request(Key::new(
                    ASSOCIATED_CONST_HEADER,
                    vec![
                        declaration.snapshot.graph_word(),
                        u64::from(declaration.index),
                    ],
                ))
                .get(0);
            let required: Arc<AssociatedConstantHeaderFact> = cx.read_fact(member_handle).await?;
            let supplied: Arc<AssociatedConstantHeaderFact> = cx.read_fact(selected_handle).await?;
            if required.header.declaration != member
                || supplied.header.declaration != declaration
                || !supplied.header.has_initializer
            {
                return Err(Error::new(
                    "associated constant selected header identity changed",
                ));
            }
            let authorization = Arc::new(authorization);
            cx.publish_fact(0, authorization.as_ref().clone())?;
            let checked: Arc<AssociatedConstantCheckedFact> = cx
                .read_fact(
                    cx.request(Key::new(
                        ASSOCIATED_CONST_HEADER,
                        vec![
                            declaration.snapshot.graph_word(),
                            u64::from(declaration.index),
                        ],
                    ))
                    .get(1),
                )
                .await?;
            checked.validate(&plan, declaration, &self.0.budget)?;
            let body = checked
                .body
                .as_ref()
                .ok_or_else(|| Error::new("selected member qualification has no initializer"))?;
            let (mut children, _children_storage) =
                self.scratch_vec(body.source().expressions.len())?;
            for expression in body.source().expressions.iter() {
                if let const_source::SyntaxKind::MemberInput(input) = &expression.kind {
                    let dependency =
                        initializer_dependency_path(&path, input.path(), &self.0.budget)?;
                    let handle = cx
                        .request(member_use_key(
                            ASSOCIATED_CONST_SELECT,
                            &target,
                            &dependency,
                            &self.0.budget,
                        )?)
                        .get(1);
                    children.push((dependency, handle));
                }
            }
            let (mut dependencies, _dependencies_storage) = self.scratch_vec(children.len())?;
            for (dependency, handle) in children {
                let child: Arc<QualifiedMemberUse> = cx.read_fact(handle).await?;
                child.validate(&plan, &target, &dependency, &self.0.budget)?;
                dependencies.push(child);
            }
            return cx.publish_fact(
                1,
                QualifiedMemberUse::new(
                    authorization,
                    checked,
                    dependencies,
                    &plan,
                    &self.0.budget,
                )?,
            );
        }
        if key.kind != ASSOCIATED_CONST_VALUE {
            return Err(Error::new("invalid associated constant use producer"));
        }
        let qualified: Arc<QualifiedMemberUse> = cx
            .read_fact(
                cx.request(member_use_key(
                    ASSOCIATED_CONST_SELECT,
                    &target,
                    &path,
                    &self.0.budget,
                )?)
                .get(1),
            )
            .await?;
        qualified.validate(&plan, &target, &path, &self.0.budget)?;
        let authorization = qualified.authorization.clone();
        let declaration = authorization.selected().declaration();
        let checked = qualified.checked.clone();
        let caller = plan.instance_facts(&target).map_err(Error::new)?;
        let source = const_source::project_associated_selected(
            &plan,
            authorization.selected(),
            &caller,
            &self.0.budget,
        )
        .map_err(Error::new)?;
        let body = checked
            .body
            .as_ref()
            .ok_or_else(|| Error::new("selected associated constant has no checked initializer"))?;
        let proof = checked
            .qualification
            .as_ref()
            .ok_or_else(|| Error::new("selected associated constant has no qualification"))?;
        if body.source().ty != source.ty || body.source().declaration != source.declaration {
            return Err(Error::new(
                "unsupported: closed associated constant initializer requires generic scalar specialization",
            ));
        }
        let value = const_check::ValueFact::evaluate_with_inputs(
            body.clone(), proof, &self.0.budget,
            |id| {
                let facts = Facts { cx };
                async move { facts.get(Facts::item(CONST_VALUE, id.index as usize)).await }
            },
            |index| {
                let plan = &plan;
                let body = &body;
                let target = &target;
                let path = &path;
                async move {
                    let expression = body.source().expressions.get(index)
                        .ok_or_else(|| Error::new("constant member input index is outside its original initializer"))?;
                    let const_source::SyntaxKind::MemberInput(input) = &expression.kind else {
                        return Err(Error::new("constant member input does not identify an original member expression"));
                    };
                    input.validate(plan, &self.0.budget).map_err(Error::new)?;
                    let dependency = initializer_dependency_path(path, input.path(), &self.0.budget)?;
                    let value: Arc<AssociatedConstantValueFact> = Facts { cx }
                        .get(member_use_key(ASSOCIATED_CONST_VALUE, target, &dependency, &self.0.budget)?)
                        .await?;
                    value.validate(plan, target, &dependency, &self.0.budget)?;
                    let scalar = value.value.scalar().value();
                    if scalar.ty() != const_source::associated_scalar_type(input.ty()).map_err(Error::new)? {
                        return Err(Error::new("associated constant initializer input value changed declared scalar type"));
                    }
                    Ok(Some(scalar))
                }
            },
        ).await?;
        if value.scalar().declaration() != declaration || value.scalar().value().ty() != source.ty {
            return Err(Error::new(
                "associated constant executed value type changed",
            ));
        }
        cx.publish_fact(
            0,
            AssociatedConstantValueFact {
                authorization,
                value: Arc::new(value),
            },
        )
    }
}

impl CompilerProvider {
    pub(super) async fn associated_constant_header_node(
        &self,
        cx: &Context,
        key: &Key,
    ) -> Result<(), Error> {
        let [snapshot, index] = key.params.as_slice() else {
            return Err(Error::new(
                "associated constant header key requires snapshot and declaration",
            ));
        };
        let declaration = ir::DeclId {
            snapshot: ir::SnapshotId::from_graph_word(*snapshot),
            index: u32::try_from(*index)
                .map_err(|_| Error::new("associated constant declaration key overflow"))?,
        };
        let facts = Facts { cx };
        let plan = facts.types().await?;
        let header = plan
            .associated_constant_header(declaration, &self.0.budget)
            .map_err(Error::new)?;
        let initializer =
            const_source::project_associated_header(&plan, declaration, &self.0.budget)
                .map_err(Error::new)?
                .map(Arc::new);
        let header = Arc::new(AssociatedConstantHeaderFact {
            header,
            initializer: initializer.clone(),
        });
        cx.publish_fact(0, header.as_ref().clone())?;
        let Some(initializer) = initializer else {
            return cx.publish_fact(
                1,
                AssociatedConstantCheckedFact {
                    header,
                    body: None,
                    qualification: None,
                },
            );
        };
        let (mut direct, _direct_storage) = self.scratch_vec(initializer.expressions.len())?;
        for expression in initializer.expressions.iter() {
            self.0.budget.charge_work(1).map_err(Error::new)?;
            if let const_source::SyntaxKind::Reference(id) = expression.kind {
                direct.push(id);
            }
        }
        direct.sort_unstable();
        direct.dedup();
        let (mut handles, _handles_storage) = self.scratch_vec(direct.len())?;
        let (mut qualification_handles, _qualification_handles_storage) =
            self.scratch_vec(direct.len())?;
        for id in &direct {
            handles.push(
                cx.request(Facts::item(CONST_DECL, id.index as usize))
                    .get(0),
            );
            qualification_handles.push(
                cx.request(Facts::item(CONST_BODY, id.index as usize))
                    .get(1),
            );
        }
        let (mut references, _references_storage) = self.scratch_vec(direct.len())?;
        for (id, handle) in direct.iter().zip(handles) {
            let reference: Arc<const_source::DeclarationFact> = cx.read_fact(handle).await?;
            if reference.declaration != *id {
                return Err(Error::new(
                    "associated constant reference metadata identity changed",
                ));
            }
            references.push(reference);
        }
        let body = Arc::new(
            const_check::check(initializer, &references, &plan, &self.0.budget)
                .map_err(Error::new)?,
        );
        let (mut proofs, _proofs_storage) = self.scratch_vec(direct.len())?;
        for (id, handle) in direct.iter().zip(qualification_handles) {
            let proof: Arc<const_check::QualificationFact> = cx.read_fact(handle).await?;
            if proof.body().source().declaration != *id {
                return Err(Error::new(
                    "associated constant qualification identity changed",
                ));
            }
            proofs.push(proof);
        }
        let qualification =
            const_check::QualificationFact::new(body.clone(), &proofs, &self.0.budget)
                .map_err(Error::new)?;
        cx.publish_fact(
            1,
            AssociatedConstantCheckedFact {
                header,
                body: Some(body),
                qualification: Some(qualification),
            },
        )
    }
}

#[cfg(test)]
mod caller_tests {
    use super::*;

    #[test]
    fn canonical_caller_keeps_nominal_arguments_order_and_declaration() {
        let program = frontend::parse(
            "struct A;struct B;fn types(a:A,b:B){}fn first<T,U>(){}fn second<T,U>(){}fn main(){}",
        )
        .unwrap();
        let budget = ir::PlanBudget::default();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        let named = |name: &str| {
            (0..plan.callable_count())
                .map(|index| plan.callable_decl(index).unwrap())
                .find(|declaration| {
                    matches!(
                        plan.persistent_decl_path(*declaration).unwrap().item(),
                        frontend::PersistentDeclItem::Name(found) if found == name
                    )
                })
                .unwrap()
        };
        let arguments = plan
            .signature(named("types"))
            .unwrap()
            .signature
            .params
            .clone();
        let original = ir::CallTarget {
            declaration: named("first"),
            type_args: arguments.clone().into(),
        };
        let key = instances::graph_key::encode(&original, &budget).unwrap();
        let decoded = instances::graph_key::decode(&key, &plan, &budget).unwrap();
        assert!(same_caller_computation(&original, &decoded, &budget).unwrap());
        let reordered = ir::CallTarget {
            declaration: original.declaration,
            type_args: Arc::from([arguments[1].clone(), arguments[0].clone()]),
        };
        assert!(!same_caller_computation(&original, &reordered, &budget).unwrap());
        let different = ir::CallTarget {
            declaration: named("second"),
            type_args: arguments.into(),
        };
        assert!(!same_caller_computation(&original, &different, &budget).unwrap());
        let mut foreign = original.clone();
        foreign.declaration.snapshot = ir::SnapshotId::allocate().unwrap();
        assert!(!same_caller_computation(&original, &foreign, &budget).unwrap());
    }
}
