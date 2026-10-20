//! Actual generic BODY issuance and typed-read omission controls.
use super::*;

pub(in crate::compiler) fn issued_body_fixture(
    source: &str,
) -> (
    Arc<crate::compiler::InstanceBodyFact>,
    crate::engine::EngineAuthority,
) {
    use crate::compiler::{
        CANONICAL_BODY, CompilerProvider, Input, InstanceBodyFact, RESOLVE, SIGNATURES, Snapshot,
    };
    use crate::engine::{Engine, Key, Provider};
    struct Fixture(Arc<Snapshot>);
    impl Provider for Fixture {
        fn outputs(&self, key: &Key) -> usize {
            if key.kind == 997 {
                1
            } else {
                CompilerProvider(self.0.clone()).outputs(key)
            }
        }
        fn start(&self, cx: crate::engine::Context, key: Key) -> crate::engine::TaskFuture {
            if key.kind != 997 {
                return CompilerProvider(self.0.clone()).start(cx, key);
            }
            Box::pin(async move {
                let signatures = cx.request(Key::new(SIGNATURES, vec![])).get(0);
                let resolved = cx.request(Key::new(RESOLVE, vec![])).get(0);
                let plan: Arc<crate::compiler::frontend::PlanCtx> =
                    cx.read_fact(signatures).await?;
                let program: Arc<Arc<crate::compiler::frontend::Program>> =
                    cx.read_fact(resolved).await?;
                let indices = program
                    .functions
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| {
                        f.name
                            .segments()
                            .last()
                            .is_some_and(|name| program.names.spelling(*name).unwrap() == "forward")
                    })
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                assert_eq!(indices.len(), 1);
                let target = CallTarget {
                    declaration: plan.function_decl(indices[0]).unwrap(),
                    type_args: Arc::from([Ty::Int(IntTy::U32)]),
                };
                let params = crate::compiler::instances::graph_key::encode(
                    &target,
                    &self::PlanBudget::default(),
                )
                .map_err(crate::engine::Error::new)?;
                let handle = cx.request(Key::new(CANONICAL_BODY, params)).get(0);
                let body: Arc<InstanceBodyFact> = cx.read_fact(handle).await?;
                body.validate_current_engine(cx.authority())?;
                cx.publish_fact(0, (body, cx.authority()))
            })
        }
    }
    let budget = PlanBudget::default();
    let engine = Engine::new(Fixture(Arc::new(Snapshot {
        input: Input::Text(source.into()),
        source_options: crate::compiler::source::SourceOptions::default(),
        budget,
    })));
    let demand = engine.request(Key::new(997, vec![]));
    engine.run(1).unwrap();
    demand.completion().unwrap();
    let issued = demand
        .fact::<(Arc<InstanceBodyFact>, crate::engine::EngineAuthority)>(0)
        .unwrap();
    issued.as_ref().clone()
}

pub(in crate::compiler) fn reject_changed_rows(
    body: &Arc<crate::compiler::InstanceBodyFact>,
    authority: crate::engine::EngineAuthority,
    rows: Option<Arc<[crate::compiler::frontend::nominal_call_receipts::closed::ClosedReceipt]>>,
) -> bool {
    let mut proof = body
        .checked
        .body_substitution
        .as_ref()
        .unwrap()
        .as_ref()
        .clone();
    proof.closed_calls = rows;
    let mut checked = body.checked.as_ref().clone();
    checked.body_substitution = Some(Arc::new(proof));
    let changed = crate::compiler::InstanceBodyFact {
        facts: body.facts.clone(),
        checked: Arc::new(checked),
    };
    changed
        .validate_current_engine(authority)
        .and_then(|_| {
            changed
                .checked
                .body_substitution
                .as_ref()
                .unwrap()
                .validate(&changed.checked, &changed.facts, changed.facts.budget())
                .map_err(crate::engine::Error::new)
        })
        .is_err()
}

pub(in crate::compiler) fn issued_rows(
    body: &Arc<crate::compiler::InstanceBodyFact>,
) -> Arc<[crate::compiler::frontend::nominal_call_receipts::closed::ClosedReceipt]> {
    body.checked
        .body_substitution
        .as_ref()
        .unwrap()
        .closed_calls
        .as_ref()
        .unwrap()
        .clone()
}

#[test]
fn issued_raw_call_body_rejects_stripped_engine_receipts_and_whole_proof() {
    use crate::compiler::InstanceBodyFact;
    let (body, authority) = issued_body_fixture(
        "fn raw()->u32{1}fn forward<T>(v:T)->T{let n=raw();v}fn main(){let n=forward(7u32);}",
    );
    let proof = body
        .checked
        .body_substitution
        .as_ref()
        .expect("actual generic BODY proof");
    assert!(
        !proof
            .closed_calls
            .as_ref()
            .expect("engine-issued call inventory")
            .is_empty()
    );
    assert!(
        body.checked
            .exprs
            .iter()
            .any(|expression| matches!(expression.kind, ExprKind::Call { .. }))
    );
    for mode in 0..3 {
        let mut checked = body.checked.as_ref().clone();
        if mode == 2 {
            checked.body_substitution = None;
        } else {
            let mut changed = proof.as_ref().clone();
            changed.closed_calls = None;
            if mode == 1 {
                changed.engine = None;
            }
            checked.body_substitution = Some(Arc::new(changed));
        }
        let changed = InstanceBodyFact {
            facts: body.facts.clone(),
            checked: Arc::new(checked),
        };
        let consumed = changed.validate_current_engine(authority);
        if mode == 2 {
            assert!(
                consumed.is_err(),
                "typed generic BODY accepted whole substitution proof removal"
            );
            continue;
        }
        assert!(
            consumed
                .and_then(|_| {
                    changed
                        .checked
                        .body_substitution
                        .as_ref()
                        .unwrap()
                        .validate(&changed.checked, &changed.facts, changed.facts.budget())
                        .map_err(crate::engine::Error::new)
                })
                .is_err(),
            "typed BODY accepted omitted proof component mode {mode}"
        );
    }
}
