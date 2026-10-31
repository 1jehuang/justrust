//! Early source outputs for declaration-only macro exports. No body parser runs.
use super::{ExportSubtree, LocalExports};
use crate::compiler::{ParseFileFailure, expansion_budget, keys, owner_path, source};
use crate::engine::{Context, Error};
use std::sync::Arc;

pub(in crate::compiler) type LocalResult = Result<Arc<LocalExports>, ParseFileFailure>;
pub(in crate::compiler) type SubtreeResult = Result<Arc<ExportSubtree>, ParseFileFailure>;

fn fail(
    cx: &Context,
    error: ParseFileFailure,
    budget: &expansion_budget::ExpansionBudget,
) -> Result<(), Error> {
    cx.publish_fact(1, Err::<Arc<LocalExports>, _>(error.inherited(budget)?))?;
    cx.publish_fact(2, Err::<Arc<ExportSubtree>, _>(error))
}
/// SOURCE retains only its immutable declaration projection and child count
/// while publishing the original unit unchanged. No self-output dependency,
/// shared side slot, duplicate projection, or body parser is needed.
pub(in crate::compiler) async fn finish(
    cx: &Context,
    file: keys::SourceFile,
    view: Result<source::FileView, source::SourceError>,
    child_count: usize,
) -> Result<(), Error> {
    let admission = cx.request_item(&keys::Admit(file.clone()));
    let admitted = admission.get(0);
    let admitted_exit = admission.get(1);
    let parent = file.parent().map(|parent| cx.request_item(&parent).get(1));
    let budget = expansion_budget::ExpansionBudget::default();
    // Explicit per-file source completeness. Request every independent child
    // inventory before awaiting any, never any child body or parse operation.
    budget
        .charge(
            expansion_budget::Counter::Work,
            (child_count as u64).saturating_mul((file.depth() as u64).saturating_add(1)),
        )
        .map_err(|error| Error::new(error.to_string()))?;
    // SourceFile::child clones the ordinal path and then grows it for one
    // child. Cover that backing as well as the key structs in BudgetVec.
    let _key_paths = budget
        .reserve_array(
            expansion_budget::Storage::Temporary,
            child_count as u64,
            (file.depth() as u64)
                .saturating_mul(2)
                .max(4)
                .saturating_mul(std::mem::size_of::<u64>() as u64),
        )
        .map_err(|error| Error::new(error.to_string()))?;
    let mut keys = crate::compiler::token_session::BudgetVec::new(
        &budget,
        expansion_budget::Storage::Temporary,
    )
    .map_err(|error| Error::new(error.to_string()))?;
    for index in 0..child_count {
        keys.push(file.child(index))
            .map_err(|error| Error::new(error.to_string()))?;
    }
    let handles = cx.request_all(keys.as_slice().iter(), 2);
    let admitted: Arc<source::source_admit::AdmitState> = cx.read_fact(admitted).await?;
    let parent: Option<Arc<LocalResult>> = match parent {
        Some(parent) => Some(cx.read_fact(parent).await?),
        None => None,
    };
    if admitted.failed() {
        let error = admitted
            .failure()
            .ok_or_else(|| Error::new("failed source admission lacks diagnostic"))?;
        return fail(
            cx,
            ParseFileFailure::from_error(source::copy_error(error, error.primary), &budget)?,
            &budget,
        );
    }
    let parent = match parent.as_deref() {
        Some(Ok(parent)) => Some(parent.as_ref()),
        Some(Err(error)) => return fail(cx, error.inherited(&budget)?, &budget),
        None => None,
    };
    let mut view = match view {
        Ok(view) => view,
        Err(error) => return fail(cx, ParseFileFailure::from_error(error, &budget)?, &budget),
    };
    view.base = admitted.id();
    let mut issuer = owner_path::OwnerIssuer::new(cx.owner_root()?, &budget)
        .map_err(|error| Error::new(format!("export owner issuance failed: {error:?}")))?;
    let snapshot = issuer
        .issue(owner_path::OwnerDomain::SourceSnapshot)
        .map_err(|error| Error::new(format!("export owner issuance failed: {error:?}")))?;
    let owner = issuer
        .issue(owner_path::OwnerDomain::TokenSession)
        .map_err(|error| Error::new(format!("export owner issuance failed: {error:?}")))?;
    let projection = match match parent {
        Some(parent) => source::PreparedLexInput::child_file_view(
            &view,
            parent.names().session().sources(),
            &budget,
            snapshot,
        ),
        None => source::PreparedLexInput::file_view(&view, &budget, snapshot),
    } {
        Ok(projection) => projection,
        Err(error) => return fail(cx, ParseFileFailure::from_error(error, &budget)?, &budget),
    };
    let local = match LocalExports::collect(
        file.clone(),
        child_count,
        &view.prepared,
        &projection,
        budget.clone(),
        owner,
        parent,
    ) {
        Ok(local) => Arc::new(local),
        Err(error) => return fail(cx, ParseFileFailure::from_error(error, &budget)?, &budget),
    };
    cx.publish_fact(1, Ok::<_, ParseFileFailure>(local.clone()))?;
    // Admission alone owns canonical file discovery and its failure boundary.
    // Its exit reads SOURCE output zero, never either export output, so this
    // wait cannot introduce a body or export dependency cycle.
    let exit: Arc<source::source_admit::AdmitState> = cx.read_fact(admitted_exit).await?;
    if let Some(error) = exit.failure() {
        return cx.publish_fact(
            2,
            Err::<Arc<ExportSubtree>, _>(ParseFileFailure::from_error(
                source::copy_error(error, error.primary),
                &budget,
            )?),
        );
    }
    let mut children = crate::compiler::token_session::BudgetVec::new(
        &budget,
        expansion_budget::Storage::Retained,
    )
    .map_err(|error| Error::new(error.to_string()))?;
    for handle in handles {
        let child: Arc<SubtreeResult> = cx.read_fact(handle).await?;
        match child.as_ref() {
            Ok(child) => children
                .push(child.clone())
                .map_err(|error| Error::new(error.to_string()))?,
            Err(error) => {
                return cx.publish_fact(2, Err::<Arc<ExportSubtree>, _>(error.inherited(&budget)?));
            }
        }
    }
    let complete = match ExportSubtree::complete(local, children, &budget) {
        Ok(complete) => Ok(Arc::new(complete)),
        Err(error) => Err(ParseFileFailure::from_error(error, &budget)?),
    };
    cx.publish_fact(2, complete)
}

#[cfg(test)]
mod authority_tests {
    use crate::compiler::{
        CompilerProvider, Input, Snapshot, frontend,
        frontend::{AstPath, Expr, PlanCtx, Program},
        ir::PlanBudget,
        keys, source,
    };
    use crate::engine::Engine;
    use std::sync::Arc;

    const SOURCE: &str = r#"pub fn helper()->i32{7} mod definitions {#[macro_export] macro_rules! exported {()=>{$crate::helper()};}} fn first()->i32{crate::exported!()} fn second()->i32{crate::exported!()} fn main(){if first()+second()==14{println!("export7");}}"#;

    fn fixture(workers: usize) -> frontend::ParsedProgram {
        let snapshot = Arc::new(Snapshot {
            input: Input::Text(SOURCE.into()),
            source_options: source::SourceOptions::default(),
            budget: PlanBudget::default(),
        });
        let engine = Engine::new(CompilerProvider(snapshot));
        let parsed = engine.request_item(&keys::ParseFile(keys::SourceFile::input()));
        let report = engine.run(workers).unwrap();
        assert_eq!(report.failures, 0, "{:?}", report.node_errors);
        parsed.completion().unwrap();
        let parsed = parsed
            .fact::<Result<frontend::ParsedProgram, crate::compiler::ParseFileFailure>>(0)
            .unwrap();
        parsed.as_ref().as_ref().unwrap().clone()
    }

    fn original_exported_call(program: &Program, spelling: &str) -> AstPath {
        let function = program
            .functions
            .iter()
            .find(|function| {
                function.name.len() == 1
                    && program.names.spelling(function.name.segments()[0]).unwrap() == spelling
            })
            .unwrap();
        let Some(Expr::Call { name, .. }) = function.body.tail.as_deref() else {
            panic!("expected actual exported-rule call path");
        };
        assert!(matches!(
            name.anchor(),
            frontend::PathAnchor::DefiningCrate(_)
        ));
        name.clone()
    }

    #[test]
    fn original_dollar_crate_anchor_cannot_be_erased_or_transplanted_between_real_calls() {
        for workers in [1, 8] {
            let parsed = fixture(workers);
            let first = original_exported_call(&parsed.program, "first");
            let second = original_exported_call(&parsed.program, "second");
            first.check(parsed.program.names.as_ref()).unwrap();
            second.check(parsed.program.names.as_ref()).unwrap();
            assert_eq!(
                first.anchor(),
                second.anchor(),
                "same canonical definition crate"
            );
            assert_ne!(
                parsed.program.names.span(first.segments()[0]).unwrap(),
                parsed.program.names.span(second.segments()[0]).unwrap(),
                "distinct genuine source occurrences"
            );
            assert!(
                first
                    .with_anchor(second.anchor())
                    .check(parsed.program.names.as_ref())
                    .is_err()
            );
            assert!(
                first
                    .with_anchor(frontend::PathAnchor::Crate)
                    .check(parsed.program.names.as_ref())
                    .is_err()
            );
            let resolved = frontend::resolve(&parsed).unwrap();
            PlanCtx::from_shared_program(Arc::new(resolved), &PlanBudget::default()).unwrap();
        }
    }

    #[test]
    fn original_dollar_crate_anchor_cannot_import_a_real_foreign_snapshot() {
        for workers in [1, 8] {
            let original = fixture(workers);
            let foreign = fixture(workers);
            let first = original_exported_call(&original.program, "first");
            let donor = original_exported_call(&foreign.program, "first");
            assert_ne!(
                first.anchor(),
                donor.anchor(),
                "same bytes are not canonical source-crate authority"
            );
            assert!(
                first
                    .with_anchor(donor.anchor())
                    .check(original.program.names.as_ref())
                    .is_err()
            );
        }
    }
}
