//! Real parser/body-solver controls. These tests grant no unsafe or memory authority.
use super::*;

fn inspect(source: &str, malformed_arity: bool) -> Result<(usize, Vec<ir::Ty>), String> {
    let program = parse(source)?;
    let signatures = signature_table(&program)?;
    let index = program
        .functions
        .iter()
        .position(|function| {
            function.name.segments().last().is_some_and(|name| {
                program
                    .names
                    .spelling(*name)
                    .is_ok_and(|name| name == "probe")
            })
        })
        .ok_or("missing source probe")?;
    let plan = signatures.plan()?;
    let declaration = plan.function_decl(index)?;
    let function = plan.callable_body(&program, declaration)?;
    let mut checker = Checker::new(&signatures, ir::Ty::Unit, function.module);
    checker.install_lexical_scope(function.lexical_scope)?;
    checker.declaration = Some(declaration);
    let malformed = Expr::NativeSyscall(Vec::new());
    let mut solver = Solver {
        checker: &checker,
        variables: Vec::new(),
        scopes: vec![HashMap::new()],
        sites: Vec::new(),
        methods: Vec::new(),
        projections: Vec::new(),
        delayed_results: Vec::new(),
        coercions: Vec::new(),
        iterator_items: Vec::new(),
        patterns: Vec::new(),
        pattern_bindings: None,
        replaying_pattern: false,
        closure_terms: Vec::new(),
        local_context: None,
        return_terms: Vec::new(),
        control_targets: Vec::new(),
        unit_defaults: Vec::new(),
        associated_equalities: Vec::new(),
        operators: Vec::new(),
        projection_defaults: false,
        opaque_return: None,
        preparation: None,
        missing_nominals: std::cell::RefCell::new(Vec::new()),
        missing_source_schemas: std::cell::Cell::new(false),
        missing_declarations: std::cell::RefCell::new(Vec::new()),
        preparation_needs_actuals: std::cell::Cell::new(false),
    };
    let unit = solver.fixed(ir::Ty::Unit)?;
    solver.return_terms.push(unit);
    if malformed_arity {
        solver.expr(&malformed, 0)?;
    }
    solver.block(&function.body, 0)?;
    solver.solve()?;
    let pending = solver.projections.len() + solver.coercions.len();
    let mut types = Vec::new();
    for (_, term) in &solver.sites {
        if let Some(ty) = solver.resolve(*term, 0)? {
            types.push(ty);
        }
    }
    Ok((pending, types))
}

#[test]
fn syscall_operands_and_result_have_exact_usize_constraints() {
    let source = "fn probe(){let n=1;let result=unsafe{__justrust_linux_syscall6!(39usize,n,0usize,0usize,0usize,0usize,0usize)};let copied=result;}fn main(){}";
    let (pending, types) = inspect(source, false).unwrap();
    assert_eq!(pending, 0);
    assert!(types.len() >= 3);
    assert!(types.iter().all(|ty| *ty == ir::Ty::Int(ir::IntTy::Usize)));
    assert!(
        inspect(source, true)
            .unwrap_err()
            .contains("exactly seven operands")
    );
    let wrong = "fn probe(){let result=unsafe{__justrust_linux_syscall6!(39usize,true,0usize,0usize,0usize,0usize,0usize)};}fn main(){}";
    assert!(
        inspect(wrong, false).is_err(),
        "bool operand must not acquire usize authority"
    );
}

#[test]
fn raw_pointer_assignment_projects_only_its_declared_pointee() {
    for kind in ["mut", "const"] {
        let source = format!(
            "fn probe(){{let pointer=1usize as *{kind} u8;unsafe{{*pointer=7;}}}}fn main(){{}}"
        );
        let (pending, _) = inspect(&source, false).unwrap();
        assert_eq!(pending, 0, "the structural pointee is known for {kind}");
    }
    let wrong = "fn probe(){let pointer=1usize as *mut u8;unsafe{*pointer=true;}}fn main(){}";
    assert!(inspect(wrong, false).is_err());
    let nonpointer = "fn probe(){let value=1usize;unsafe{*value=7;}}fn main(){}";
    assert!(
        inspect(nonpointer, false).unwrap().0 > 0,
        "nonpointer dereference must remain unresolved, never blessed"
    );
}

#[test]
fn scalar_inference_does_not_grant_unsafe_or_mutable_memory_permission() {
    for source in [
        "fn probe(){let pointer=1usize as *mut u8;*pointer=7;}fn main(){}",
        "fn probe(){let pointer=1usize as *const u8;unsafe{*pointer=7;}}fn main(){}",
        "fn probe(){let value=1usize;unsafe{*value=7;}}fn main(){}",
    ] {
        for workers in [1, 8] {
            assert!(
                crate::compiler::compile(source.into(), workers, true).is_err(),
                "mandatory CHECK must reject workers={workers}: {source}"
            );
        }
    }
}
