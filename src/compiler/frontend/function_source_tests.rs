use super::*;
use crate::compiler::source;

fn genuine_source() -> source::PreparedSource {
    let options = source::SourceOptions::default();
    let budget = crate::compiler::expansion_budget::ExpansionBudget::default();
    let mut assembler = source::SourceAssembler::new(&options);
    let mut step = assembler
        .provide(Arc::new(source::prepare_text_unit(
            "function-authority.rs",
            "fn main(){}",
            &options,
        )))
        .unwrap();
    let prepared = loop {
        match step {
            source::AssemblyStep::Done(prepared) => break prepared,
            source::AssemblyStep::Need(need) => {
                let unit = match need.parent.as_deref() {
                    None => source::prepare_prelude_unit(&options, &budget),
                    Some(parent) => {
                        source::prepare_child_unit(parent, need.index, &options, &budget)
                    }
                };
                step = assembler.provide(Arc::new(unit)).unwrap();
            }
        }
    };
    *prepared
}
fn genuine() -> ParsedProgram {
    parse_prepared(&genuine_source()).unwrap()
}

fn reject_original_token_range_mutations() {
    struct TokenRangeProvider;
    impl crate::Provider for TokenRangeProvider {
        fn outputs(&self, _: &crate::Key) -> usize {
            1
        }
        fn start(&self, cx: crate::Context, _: crate::Key) -> crate::TaskFuture {
            Box::pin(async move {
                let budget = crate::compiler::expansion_budget::ExpansionBudget::default();
                let mut issuer =
                    crate::compiler::owner_path::OwnerIssuer::new(cx.owner_root()?, &budget)
                        .map_err(|error| {
                            crate::Error::new(format!("test owner issuance: {error:?}"))
                        })?;
                let snapshot = issuer
                    .issue(crate::compiler::owner_path::OwnerDomain::SourceSnapshot)
                    .map_err(|error| {
                        crate::Error::new(format!("test owner issuance: {error:?}"))
                    })?;
                let owner = issuer
                    .issue(crate::compiler::owner_path::OwnerDomain::TokenSession)
                    .map_err(|error| {
                        crate::Error::new(format!("test owner issuance: {error:?}"))
                    })?;
                reject_token_ranges_with_owner(snapshot, owner, budget);
                cx.publish(0, 1)
            })
        }
    }
    let engine = crate::Engine::new(TokenRangeProvider);
    let output = engine.request(crate::compiler::keys::params(1, 0, vec![]));
    engine.run(1).unwrap();
    assert_eq!(output.result(0).unwrap(), 1);
}

fn reject_token_ranges_with_owner(
    snapshot: crate::compiler::owner_path::Issued,
    owner: crate::compiler::owner_path::Issued,
    budget: crate::compiler::expansion_budget::ExpansionBudget,
) {
    let prepared = genuine_source();
    let projection = source::PreparedLexInput::prepared(&prepared, &budget, snapshot).unwrap();
    let issuers = prepared
        .embedded_issuers(projection.snapshot(), &budget)
        .unwrap();
    let (tokens, session, cap) = native_input(&projection, budget, owner)
        .unwrap()
        .into_parts();
    let names =
        NameStoreBuilder::new(session, cap, crate::compiler::token::Edition::E2024).unwrap();
    let start = tokens
        .windows(3)
        .position(|tokens| {
            tokens[0].text == "pub" && tokens[1].text == "fn" && tokens[2].text == "utf8_width"
        })
        .unwrap();
    let name = start + 2;
    assert_eq!(tokens[start].text, "pub");
    assert_eq!(tokens[start + 1].text, "fn");
    let mut depth = 0usize;
    let mut started = false;
    let end = (name..tokens.len())
        .find(|&index| {
            match tokens[index].text.as_str() {
                "{" => {
                    depth += 1;
                    started = true;
                }
                "}" => {
                    depth = depth.checked_sub(1).unwrap();
                }
                _ => {}
            }
            started && depth == 0
        })
        .unwrap();
    let exact = &tokens[start..=end];
    let snapshot = names.session().sources();
    let span = original_token_range(&names, exact).unwrap().unwrap();
    assert!(
        issuers
            .iter()
            .any(|issuer| issuer.authorizes(snapshot, span))
    );
    let rendered = snapshot.render_span(span).unwrap();
    let close = names.session().span(tokens[end].whole.span).unwrap();
    let crate::compiler::expansion_span::SpanOrigin::Original(close) = close.origin else {
        panic!("actual consumed close lost original provenance");
    };
    assert_eq!(rendered.hi, snapshot.render_span(close).unwrap().hi);
    // This actual function ends with a nested if-chain: omitting the outer
    // brace still leaves a '}', which must NOT become the function's close.
    assert_eq!(exact[exact.len() - 2].text, "}");
    assert!(
        original_token_range(&names, &exact[..exact.len() - 1])
            .unwrap()
            .is_none()
    );
    assert!(
        original_token_range(&names, &tokens[start..=end + 1])
            .unwrap()
            .is_none()
    );
    assert!(
        original_token_range(&names, exact.iter().chain(std::iter::once(&tokens[end])))
            .unwrap()
            .is_none()
    );

    let other = tokens
        .iter()
        .find(|token| {
            let Ok(data) = names.session().span(token.whole.span) else {
                return false;
            };
            let crate::compiler::expansion_span::SpanOrigin::Original(span) = data.origin else {
                return false;
            };
            snapshot.render_span(span).unwrap().file != rendered.file
        })
        .expect("genuine loader fixture must contain distinct original files");
    let middle = exact.len() / 2;
    assert!(
        original_token_range(
            &names,
            exact[..middle]
                .iter()
                .chain(std::iter::once(other))
                .chain(exact[middle..].iter())
        )
        .unwrap()
        .is_none()
    );
    let closing = &tokens[end];
    let expanded = Token {
        text: closing.text.clone(),
        line: closing.line,
        column: closing.column,
        string: closing.string,
        kind: closing.kind.clone(),
        whole: TokenPart::expanded(closing.whole.span, 1, 1).unwrap(),
        constituents: closing.constituents.clone(),
        suffixes: closing.suffixes.clone(),
        name: closing.name,
    };
    assert!(
        original_token_range(
            &names,
            exact[..exact.len() - 1]
                .iter()
                .chain(std::iter::once(&expanded))
        )
        .unwrap()
        .is_none()
    );
}
fn width(program: &Program) -> usize {
    program
        .functions
        .iter()
        .position(|function| {
            function
                .name
                .segments()
                .last()
                .is_some_and(|name| program.names.spelling(*name).unwrap() == "utf8_width")
        })
        .expect("actual shipped utf8_width source missing")
}
fn declaration(plan: &PlanCtx, index: usize) -> ir::DeclId {
    plan.registry
        .declarations
        .iter()
        .find(|info| info.kind == ir::DeclKind::Function && info.source_index == index)
        .unwrap()
        .id
}

fn named(program: &Program, spelling: &str) -> usize {
    program
        .functions
        .iter()
        .position(|function| {
            function
                .name
                .segments()
                .last()
                .is_some_and(|name| program.names.spelling(*name).unwrap() == spelling)
        })
        .unwrap()
}

fn visit_block(block: &mut Block, visit: &mut impl FnMut(&mut Expr)) {
    for statement in &mut block.statements {
        match statement {
            Stmt::Let { value, .. } | Stmt::Expr(value) | Stmt::ImplicitExpr(value) => {
                visit_expr(value, visit)
            }
            Stmt::Return(Some(value)) => visit_expr(value, visit),
            Stmt::Return(None) => {}
            _ => panic!("genuine helper left frozen finite syntax"),
        }
    }
    if let Some(value) = &mut block.tail {
        visit_expr(value, visit);
    }
}
fn visit_expr(expr: &mut Expr, visit: &mut impl FnMut(&mut Expr)) {
    visit(expr);
    match expr {
        Expr::Call { args, .. } | Expr::NativeSyscall(args) => {
            for argument in args {
                visit_expr(argument, visit);
            }
        }
        Expr::If {
            condition,
            then_block,
            else_block,
        } => {
            visit_expr(condition, visit);
            visit_block(then_block, visit);
            visit_block(else_block, visit);
        }
        Expr::Binary { left, right, .. } => {
            visit_expr(left, visit);
            visit_expr(right, visit);
        }
        Expr::Unsafe(block) => visit_block(block, visit),
        Expr::Integer(_) | Expr::Var(_) | Expr::TypeAlignment(_) => {}
        _ => panic!("genuine helper left frozen finite syntax"),
    }
}

#[test]
fn embedded_function_authority_rejects_lossy_qualifiers_before_issuance() {
    for source in ["fn", "pub fn", "pub ( super ) fn", "pub const unsafe fn"] {
        let tokens = source.split_whitespace().collect::<Vec<_>>();
        assert!(supported_header(|index| tokens.get(index).copied()));
    }
    for source in [
        "extern fn",
        "extern \"C\" fn",
        "unsafe extern \"C\" fn",
        "pub extern \"Rust\" fn",
        "async fn",
        "pub async fn",
        "# [ no_mangle ] pub fn",
        "default fn",
        "pub ( in crate ) fn",
    ] {
        let tokens = source.split_whitespace().collect::<Vec<_>>();
        assert!(
            !supported_header(|index| tokens.get(index).copied()),
            "{source}"
        );
    }
}

#[test]
fn embedded_function_authority_genuine_called_helper_and_resolved_mutations() {
    let parsed = genuine();
    let index = named(&parsed.program, "raw_realloc");
    let receipt = parsed.program.functions[index]
        .embedded_source
        .as_ref()
        .expect("genuine original raw_realloc must issue without generated-span admission");
    receipt
        .original()
        .unwrap()
        .replay_source(
            &parsed.program.functions[index],
            &parsed.program.names,
            &parsed.program.embedded_issuers,
        )
        .unwrap();
    let function = &parsed.program.functions[index];
    let budget = crate::compiler::expansion_budget::ExpansionBudget::default();
    let relocated = receipt
        .relocate_parsed(function, index, 17, &parsed.program, &budget)
        .unwrap();
    assert_eq!(relocated.original().unwrap().function_index, index + 17);
    relocated
        .original()
        .unwrap()
        .replay_source(
            function,
            &parsed.program.names,
            &parsed.program.embedded_issuers,
        )
        .unwrap();
    assert!(
        receipt
            .relocate_parsed(function, index + 1, 17, &parsed.program, &budget)
            .is_err()
    );
    assert!(
        receipt
            .relocate_parsed(function, index, usize::MAX, &parsed.program, &budget)
            .is_err()
    );
    let mut forged = function.clone();
    forged.unsafety = !forged.unsafety;
    assert!(
        receipt
            .relocate_parsed(&forged, index, 17, &parsed.program, &budget)
            .is_err()
    );
    let program = resolve(&parsed).unwrap();
    let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    let id = declaration(&plan, index);
    plan.replay_embedded_function(id).unwrap();
    let bound = plan
        .declaration(id)
        .unwrap()
        .embedded_function
        .as_ref()
        .unwrap();
    let mut original = program.functions[index].clone();
    let mut calls = Vec::new();
    visit_block(&mut original.body, &mut |expr| {
        if let Expr::Call { name, .. } = expr {
            calls.push(name.clone());
        }
    });
    assert_eq!(
        calls.len(),
        2,
        "raw_free and raw_alloc must both be replayed"
    );
    let names = program.names.as_ref();
    let reject = |function: Function| {
        let mut changed = program.clone();
        changed.functions[index] = function;
        assert!(
            bound
                .replay_admission(&changed, &plan.registry, id, &ir::PlanBudget::default())
                .is_err()
        );
    };

    // Redirect the very same authored use to another real canonical Function.
    let mut changed = original.clone();
    let mut done = false;
    visit_block(&mut changed.body, &mut |expr| {
        if let Expr::Call { name, .. } = expr
            && !done
        {
            *name = calls[1].with_resolved_use(names, &calls[0]).unwrap();
            done = true;
        }
    });
    assert!(done);
    reject(changed);

    // Keep the canonical target, but replace its original-use channel.
    let target = AstPath::new(names, calls[0].anchor(), calls[0].segments()).unwrap();
    let mut changed = original.clone();
    let mut done = false;
    visit_block(&mut changed.body, &mut |expr| {
        if let Expr::Call { name, .. } = expr
            && !done
        {
            *name = target.with_resolved_use(names, &calls[1]).unwrap();
            done = true;
        }
    });
    assert!(done);
    reject(changed);

    // Reorder real call occurrences, preserving all other source syntax.
    let mut changed = original.clone();
    let mut call = 0;
    visit_block(&mut changed.body, &mut |expr| {
        if let Expr::Call { name, .. } = expr {
            *name = calls[1 - call].clone();
            call += 1;
        }
    });
    assert_eq!(call, 2);
    reject(changed);

    // The syntax-only syscall marker is not a permit to alter its ABI/order.
    let mut changed = original.clone();
    let mut done = false;
    visit_block(&mut changed.body, &mut |expr| {
        if let Expr::NativeSyscall(arguments) = expr
            && !done
        {
            arguments.swap(1, 2);
            done = true;
        }
    });
    assert!(done);
    reject(changed);
    let mut changed = original;
    let mut done = false;
    visit_block(&mut changed.body, &mut |expr| {
        if let Expr::NativeSyscall(arguments) = expr
            && !done
        {
            arguments.pop();
            done = true;
        }
    });
    assert!(done);
    reject(changed);
}

#[test]
fn embedded_function_authority_actual_loader_resolver_and_registration() {
    let parsed = genuine();
    // Real loader-issued source, not trusted test text. Its unsupported generic
    // header cannot receive this finite nongeneric free-function receipt.
    let generic = named(&parsed.program, "align_of");
    assert!(!parsed.program.functions[generic].type_params.is_empty());
    assert!(parsed.program.functions[generic].embedded_source.is_none());
    let index = width(&parsed.program);
    let receipt = parsed.program.functions[index]
        .embedded_source
        .as_ref()
        .unwrap();
    receipt
        .original()
        .unwrap()
        .replay_source(
            &parsed.program.functions[index],
            &parsed.program.names,
            &parsed.program.embedded_issuers,
        )
        .unwrap();
    let program = resolve(&parsed).unwrap();
    let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    plan.replay_embedded_function(declaration(&plan, index))
        .unwrap();
}

#[test]
fn embedded_function_authority_raw_copied_spoofed_and_foreign_issuer() {
    reject_original_token_range_mutations();
    for source in [
        "fn utf8_width(first:u8)->usize{if first<128u8{1usize}else{2usize}}fn main(){}",
        "mod __justrust_std{pub mod string{pub fn utf8_width(first:u8)->usize{if first<128u8{1usize}else{2usize}}}}fn main(){}",
    ] {
        let program = parse(source).unwrap();
        assert!(
            program
                .functions
                .iter()
                .all(|function| function.embedded_source.is_none())
        );
        let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
        assert!(
            plan.replay_embedded_function(declaration(&plan, width(&program)))
                .is_err()
        );
    }
    let parsed = genuine();
    let foreign = genuine();
    let index = width(&parsed.program);
    let receipt = parsed.program.functions[index]
        .embedded_source
        .as_ref()
        .unwrap()
        .original()
        .unwrap();
    let snapshot = parsed.program.names.session().sources();
    let span = snapshot.render_span(receipt.source).unwrap();
    let file = snapshot.file(span.file.0 as usize).unwrap();
    let copied = &snapshot.text(file).unwrap()[span.lo as usize..span.hi as usize];
    let copied = parse(&format!("{copied}\nfn main(){{}}")).unwrap();
    assert!(
        copied
            .functions
            .iter()
            .all(|function| function.embedded_source.is_none())
    );
    assert!(
        receipt
            .replay_source(
                &parsed.program.functions[index],
                &parsed.program.names,
                &foreign.program.embedded_issuers
            )
            .is_err()
    );
    assert!(
        receipt
            .replay_source(
                &foreign.program.functions[width(&foreign.program)],
                &foreign.program.names,
                &foreign.program.embedded_issuers
            )
            .is_err()
    );
}

#[test]
fn embedded_function_authority_header_body_site_and_canonical_identity_tampering() {
    let parsed = genuine();
    let index = width(&parsed.program);
    let receipt = parsed.program.functions[index]
        .embedded_source
        .as_ref()
        .unwrap()
        .original()
        .unwrap();
    let scoped =
        parse_unresolved("fn main(){macro_rules! marker{()=>{42}}let x=marker!();}").unwrap();
    let scope_source = &scoped.program.functions[0];
    assert!(scope_source.embedded_source.is_none());
    let scope = scope_source
        .lexical_scope
        .expect("actual local macro scope missing");
    let mut changed = parsed.program.functions[index].clone();
    assert!(changed.lexical_scope.is_none());
    changed.lexical_scope = Some(scope);
    let scope_error = receipt
        .replay_source(
            &changed,
            &parsed.program.names,
            &parsed.program.embedded_issuers,
        )
        .unwrap_err();
    assert_eq!(
        scope_error,
        "embedded function changed original header or issuer"
    );
    let mut changed = parsed.program.functions[index].clone();
    changed.body = Block {
        statements: Vec::new(),
        tail: Some(Box::new(Expr::Integer(IntegerLiteral {
            magnitude: 4,
            suffix: Some(ir::IntTy::Usize),
        }))),
    };
    assert!(
        receipt
            .replay_source(
                &changed,
                &parsed.program.names,
                &parsed.program.embedded_issuers
            )
            .is_err()
    );
    let mut changed = parsed.program.functions[index].clone();
    changed.unsafety = !changed.unsafety;
    assert!(
        receipt
            .replay_source(
                &changed,
                &parsed.program.names,
                &parsed.program.embedded_issuers
            )
            .is_err()
    );
    let mut changed = parsed.program.functions[index].clone();
    changed.params.pop();
    assert!(
        receipt
            .replay_source(
                &changed,
                &parsed.program.names,
                &parsed.program.embedded_issuers
            )
            .is_err()
    );
    let mut changed = parsed.program.functions[index].clone();
    changed.result = Type::Unit;
    assert!(
        receipt
            .replay_source(
                &changed,
                &parsed.program.names,
                &parsed.program.embedded_issuers
            )
            .is_err()
    );
    let mut changed = parsed.program.functions[index].clone();
    let Expr::If { condition, .. } = changed.body.tail.as_deref_mut().unwrap() else {
        panic!("actual width condition missing")
    };
    let Expr::Binary { op, site, .. } = condition.as_mut() else {
        panic!("actual width comparison missing")
    };
    *op = BinaryOp::Eq;
    *site = None;
    assert!(
        receipt
            .replay_source(
                &changed,
                &parsed.program.names,
                &parsed.program.embedded_issuers
            )
            .is_err()
    );
    let program = resolve(&parsed).unwrap();
    let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    let id = declaration(&plan, index);
    let bound = plan
        .declaration(id)
        .unwrap()
        .embedded_function
        .as_ref()
        .unwrap();
    let other = plan
        .registry
        .declarations
        .iter()
        .find(|info| info.kind == ir::DeclKind::Function && info.id != id)
        .unwrap()
        .id;
    assert!(
        bound
            .replay_admission(&program, &plan.registry, other, &ir::PlanBudget::default())
            .is_err()
    );
    let foreign_program = resolve(&genuine()).unwrap();
    let foreign_plan = PlanCtx::from_program(&foreign_program, &ir::PlanBudget::default()).unwrap();
    let mut forged = plan.declaration(id).unwrap().clone();
    forged.source_index = index.checked_add(1).unwrap();
    assert!(
        bind_registration(
            &bound.source,
            &program,
            &forged,
            index,
            id,
            &ir::PlanBudget::default()
        )
        .is_err()
    );
    let mut forged = plan.declaration(id).unwrap().clone();
    forged.env = foreign_plan
        .declaration(declaration(&foreign_plan, width(&foreign_program)))
        .unwrap()
        .env
        .clone();
    assert!(
        bind_registration(
            &bound.source,
            &program,
            &forged,
            index,
            id,
            &ir::PlanBudget::default()
        )
        .is_err()
    );
    let mut missing_issuer = program.clone();
    missing_issuer.embedded_issuers = Arc::from([]);
    assert!(
        bound
            .replay_admission(
                &missing_issuer,
                &plan.registry,
                id,
                &ir::PlanBudget::default()
            )
            .is_err()
    );
    assert!(
        bound
            .replay_admission(
                &foreign_program,
                &foreign_plan.registry,
                declaration(&foreign_plan, width(&foreign_program)),
                &ir::PlanBudget::default()
            )
            .is_err()
    );
}

/// Syntax-only control, not a substitute for loader or canonical authority.
#[test]
fn embedded_function_authority_recipe_preserves_ordered_calls_and_arguments() {
    fn events(names: &NameStore, function: &Function) -> Vec<FunctionSyntaxEvent> {
        let mut events = Vec::new();
        assert!(
            recipe(names, function, &mut |event| {
                events.push(event);
                Ok(())
            })
            .unwrap()
        );
        events
    }
    let parsed =
        parse_unresolved("fn f(a:usize,b:usize)->usize{g(a,b)+h(b,a)}fn main(){}").unwrap();
    let function = &parsed.program.functions[0];
    let original = events(&parsed.program.names, function);
    let mut reordered = function.clone();
    let Expr::Binary { left, right, .. } = reordered.body.tail.as_deref_mut().unwrap() else {
        panic!("binary fixture missing")
    };
    std::mem::swap(left, right);
    let reordered_events = events(&parsed.program.names, &reordered);
    assert!(
        original.len() != reordered_events.len()
            || original
                .iter()
                .zip(&reordered_events)
                .any(|(a, b)| !event_equal(&parsed.program.names, Some(a), b).unwrap())
    );
    let mut changed = function.clone();
    let Expr::Binary { left, .. } = changed.body.tail.as_deref_mut().unwrap() else {
        panic!("binary fixture missing")
    };
    let Expr::Call { args, .. } = left.as_mut() else {
        panic!("call fixture missing")
    };
    args.swap(0, 1);
    let changed_events = events(&parsed.program.names, &changed);
    assert!(
        original.len() != changed_events.len()
            || original
                .iter()
                .zip(&changed_events)
                .any(|(a, b)| !event_equal(&parsed.program.names, Some(a), b).unwrap())
    );
}
