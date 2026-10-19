//! Forged self occurrences and unused invalid bodies cannot acquire a receiver.
use justrust_engine::compiler::{
    frontend::{self, Expr},
    ir,
    semantic_path::{AstPath, PathAnchor},
};
#[path = "support/source_macro_binding_oracle.rs"]
mod oracle;

#[test]
fn invalid_unused_self_and_foreign_definition_context_match_stock_rejection() {
    for source in [
        "macro_rules! bad{()=>{self.v};}struct S{v:i32}impl S{fn unused(&self)->i32{bad!()}}fn main(){}",
        "macro_rules! bad{()=>{x};}fn unused()->i32{let x=7;bad!()}fn main(){}",
        "macro_rules! bad{()=>{self.v};}struct S{v:i32}impl S{fn unused(&self)->i32{macro_rules! nested{()=>{bad!()};}nested!()}}fn main(){}",
        "fn unused()->i32{macro_rules! bad{()=>{self.v};}bad!()}fn main(){}",
        "struct S{v:i32}impl S{fn unused(&self)->i32{macro_rules! bad{()=>{true};}bad!()}}fn main(){}",
        "struct S{v:i32}impl S{fn unused()->i32{macro_rules! bad{()=>{self.v};}bad!()}}fn main(){}",
    ] {
        let fixture = oracle::Fixture::new(source, &[], None);
        assert!(
            !fixture.stock(true).status.success(),
            "stock accepted {source}"
        );
        for workers in [1, 8] {
            for check in [true, false] {
                assert!(
                    fixture.native(workers, check).is_err(),
                    "invalid receiver accepted workers={workers} check={check}: {source}"
                );
            }
        }
    }
}

fn body_path(program: &frontend::Program, index: usize) -> AstPath {
    let Some(Expr::Field { base, .. }) =
        program.impls[0].items[index].function.body.tail.as_deref()
    else {
        panic!("field fixture")
    };
    let Expr::Var(path) = &**base else {
        panic!("receiver fixture")
    };
    path.clone()
}
fn replace_body_path(program: &mut frontend::Program, path: AstPath) {
    let Some(Expr::Field { base, .. }) =
        program.impls[0].items[0].function.body.tail.as_deref_mut()
    else {
        panic!("field fixture")
    };
    **base = Expr::Var(path);
}
const SOURCE: &str = "struct S{v:i32}impl S{fn first(&self)->i32{macro_rules! get{()=>{self.v};}get!()}fn second(&self)->i32{macro_rules! get{()=>{self.v};}get!()}}fn main(){}";
#[test]
fn foreign_receiver_scope_binder_source_and_omission_fail_immutable_replay() {
    let original = frontend::parse(SOURCE).unwrap();
    frontend::PlanCtx::from_program(&original, &ir::PlanBudget::default()).unwrap();
    for case in 0..6 {
        let mut forged = original.clone();
        match case {
            0 => replace_body_path(&mut forged, body_path(&original, 1)),
            1 => {
                forged.impls[0].items[0].function.lexical_scope =
                    original.impls[0].items[1].function.lexical_scope
            }
            2 => forged.impls[0].items[0].function.lexical_scope = None,
            3 => forged.impls[0].items[0].receiver = frontend::ReceiverKind::None,
            4 => {
                let path = AstPath::new(&*forged.names, PathAnchor::SelfModule, &[]).unwrap();
                replace_body_path(&mut forged, path);
            }
            5 => {
                let foreign = frontend::parse(SOURCE).unwrap();
                replace_body_path(&mut forged, body_path(&foreign, 0));
            }
            _ => unreachable!(),
        }
        assert!(
            frontend::PlanCtx::from_program(&forged, &ir::PlanBudget::default()).is_err(),
            "forged receiver case {case}"
        );
    }
    let path = body_path(&original, 0).with_anchor(PathAnchor::SelfType);
    assert!(
        path.check(&*original.names).is_err(),
        "receiver cannot become qualified Self type syntax"
    );
}
