use super::*;

fn fixture_key(program: &Program, path: &AstPath, namespace: Namespace) -> PathKey {
    name_scope::path_key(program.names.as_ref(), path, namespace).unwrap()
}

// Authored test labels only. Resolution itself never uses display strings.
fn canonical_path_matches(program: &Program, path: &AstPath, expected: &str) -> bool {
    path.check(program.names.as_ref()).unwrap();
    path.anchor() == PathAnchor::Crate
        && path.segments().len() == expected.split("::").count()
        && path
            .segments()
            .iter()
            .zip(expected.split("::"))
            .all(|(segment, label)| program.names.spelling(*segment).unwrap() == label)
}

#[test]
fn tuple_patterns_preserve_old_scope_and_simultaneous_fresh_bindings() {
    let program = parse("fn f()->i32{7} fn main(){let x=1;let ((mut x,),y):((i32,),bool)=((x,),true);x=2;let _=x;let _y=y;let (f,z)=(f(),f());}").unwrap();
    let env = type_env(&program).unwrap();
    let checked = type_function_with_env(&program, 1, &env).unwrap();
    checked.validate().unwrap();
    let ir::Stmt::LetPattern {
        value, bindings, ..
    } = &checked.body.statements[1]
    else {
        panic!("expected tuple pattern")
    };
    assert_eq!(
        bindings
            .iter()
            .map(|binding| binding
                .projection
                .iter()
                .map(|p| match p {
                    ir::Projection::Field(index) => *index,
                    _ => panic!("unexpected downcast"),
                })
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        vec![vec![0, 0], vec![1]]
    );
    assert_eq!(
        bindings
            .iter()
            .map(|binding| binding.local)
            .collect::<Vec<_>>(),
        vec![ir::LocalId(1), ir::LocalId(2)]
    );
    assert!(checked.locals[1].mutable);
    assert_eq!(checked.locals[2].ty, ir::Ty::Bool);
    assert!(
        checked.exprs[..value.0]
            .iter()
            .any(|expr| matches!(expr.kind, ir::ExprKind::Local(ir::LocalId(0))))
    );
    let ir::Stmt::LetPattern { bindings, .. } = &checked.body.statements[3] else {
        panic!("expected wildcard")
    };
    assert!(bindings.is_empty());
    assert_eq!(
        checked
            .locals
            .iter()
            .filter(|local| local.name == "x")
            .count(),
        2
    );
    assert!(
        parse("fn f()->i32{1} fn main(){let (f,z)=(f(),2);f();}")
            .unwrap_err()
            .contains("not callable")
    );
    check(
        &parse("fn main(){let(x,y)=(1,2);let (z,)=(x,);let ()=();let (mut grouped)=z;grouped=y;}")
            .unwrap(),
    )
    .unwrap();
}

#[test]
fn struct_patterns_publish_source_order_paths_and_nominal_types() {
    let program = parse("struct Inner{x:i32} struct S{a:Inner,b:bool} use S as Alias; fn main(){let s=S{a:Inner{x:1},b:true};let Alias{b:mut flag,a:Inner{x:n}}:S=s;flag=false;let Alias{..}=S{a:Inner{x:n},b:flag};}").unwrap();
    let env = type_env(&program).unwrap();
    let checked = type_function_with_env(&program, 0, &env).unwrap();
    checked.validate().unwrap();
    let ir::Stmt::LetPattern { ty, bindings, .. } = &checked.body.statements[1] else {
        panic!("expected struct pattern")
    };
    assert_eq!(
        *ty,
        ir::Ty::Struct(
            env.structs[&fixture_key(&program, &program.structs[1].name, Namespace::Type)].clone()
        )
    );
    assert_eq!(
        bindings
            .iter()
            .map(|binding| binding
                .projection
                .iter()
                .map(|p| match p {
                    ir::Projection::Field(index) => *index,
                    _ => panic!("unexpected downcast"),
                })
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        vec![vec![1], vec![0, 0]]
    );
    assert_eq!(checked.locals[bindings[0].local.0].name, "flag");
    assert!(checked.locals[bindings[0].local.0].mutable);
    assert_eq!(
        checked.locals[bindings[1].local.0].ty,
        ir::Ty::Int(ir::IntTy::I32)
    );
}

#[test]
fn pattern_shape_duplicate_annotation_and_privacy_errors_reject_even_when_dead() {
    for source in [
        "fn main(){let(x,x)=(1,2);}",
        "fn main(){let(x,y)=(1,);}",
        "fn main(){let ()=(1,);}",
        "fn main(){let(x,y):(i32,bool)=(1,2);}",
        "fn main(){let(x,y)=(1,true);x=false;}",
        "struct S{x:i32,y:bool} fn main(){let S{x}=S{x:1,y:true};}",
        "struct S{x:i32} fn main(){let S{x:_,x:_}=S{x:1};}",
        "struct S{x:i32} fn main(){let S{missing:_,..}=S{x:1};}",
        "struct S{x:i32} struct T{x:i32} fn main(){let S{x}=T{x:1};}",
        "mod m{pub struct S{private:i32} pub fn make()->S{S{private:1}}} fn main(){let m::S{private:_,..}=m::make();}",
        "mod m{struct H{x:i32} pub struct S{pub hidden:H} pub fn make()->S{S{hidden:H{x:1}}}} fn main(){let m::S{hidden:_,..}=m::make();}",
        "fn main(){return;let(x,x)=(1,2);}",
    ] {
        assert!(
            parse(source).and_then(|program| check(&program)).is_err(),
            "accepted {source}"
        );
    }
    check(&parse("mod m{struct H{x:i32} pub struct S{pub hidden:H} pub fn make()->S{S{hidden:H{x:1}}}} fn main(){let m::S{..}=m::make();}").unwrap()).unwrap();
}

#[test]
fn never_patterns_use_explicit_shape_and_preserve_dead_value_types() {
    for source in [
        "fn main(){let(x,y):(i32,bool)=loop{};}",
        "struct S{x:i32} fn main(){let S{x}=loop{};}",
        "fn main(){let _=loop{};}",
        "fn main(){let ()=loop{};}",
        "struct S{x:i32} fn main(){let(S{x},())=loop{};}",
        "struct S{x:i32} fn main(){let (S{x},)=({return;},);}",
    ] {
        let program = parse(source).unwrap();
        let env = type_env(&program).unwrap();
        type_function_with_env(&program, 0, &env)
            .unwrap()
            .validate()
            .unwrap();
    }
    let error = check(&parse("fn main(){let(x,y)=loop{};}").unwrap()).unwrap_err();
    assert!(error.contains("inference is unsupported"));
    assert!(check(&parse("fn main(){let(x,y):(i32,bool)={return;(false,0)};}").unwrap()).is_err());
}

#[test]
fn unsupported_patterns_and_pattern_nesting_are_explicit() {
    for source in [
        "fn main(){let ref x=1;}",
        "fn main(){let x @ _=1;}",
        "fn main(){let 1=1;}",
        "fn main(){let(mut _)=1;}",
        "fn main(){let(x,y)=(1,2) else {};}",
        "struct S{x:i32} fn main(){let S{..,x}=S{x:1};}",
        "struct S{x:i32} fn main(){let S{..,}=S{x:1};}",
        "struct S{x:i32} fn main(){let S{x,..,}=S{x:1};}",
        "struct S{x:i32} fn main(){let S{. .}=S{x:1};}",
        "fn f((x,y):(i32,i32)){} fn main(){}",
    ] {
        assert!(parse(source).is_err(), "accepted {source}");
    }
    check(&parse("fn main(){let(x,..)=(1,2);}").unwrap()).unwrap();
    let source = format!(
        "fn main(){{let {}x{}=1;}}",
        "(".repeat(140),
        ")".repeat(140)
    );
    assert!(parse(&source).unwrap_err().contains("nesting"));
    let mut program = parse("fn main(){let _=1;}").unwrap();
    let mut pattern = Pattern::Wildcard;
    for _ in 0..140 {
        pattern = Pattern::Tuple(vec![pattern]);
    }
    let Stmt::LetPattern {
        pattern: target, ..
    } = &mut program.functions[0].body.statements[0]
    else {
        unreachable!()
    };
    *target = pattern;
    assert!(check(&program).unwrap_err().contains("nesting"));
}

#[test]
fn structs_share_nominal_handles_within_one_environment_only() {
    let program = parse("struct S{x:i32} use S as Alias; fn id(x:Alias)->S{x} fn main(){let x=Alias{x:1};let y=id(x);y.x;}").unwrap();
    let env = type_env(&program).unwrap();
    let struct_key = fixture_key(&program, &program.structs[0].name, Namespace::Type);
    let function_key = fixture_key(&program, &program.functions[0].name, Namespace::Value);
    let ty = ir::Ty::Struct(env.structs[&struct_key].clone());
    assert_eq!(env.functions[&function_key].params, vec![ty.clone()]);
    assert_eq!(env.functions[&function_key].result, ty);
    assert!(!ty.is_copy());
    assert!(!ir::Ty::Tuple(vec![ty.clone()].into()).is_copy());
    let separate = type_env(&program).unwrap();
    assert_ne!(env.structs[&struct_key], separate.structs[&struct_key]);
    for index in 0..program.functions.len() {
        type_function_with_env(&program, index, &env).unwrap();
    }
}

#[test]
fn struct_initializers_retain_source_order_and_checked_field_indices() {
    let program =
        parse("struct S{a:i32,b:bool} fn main(){let mut s=S{b:true,a:3};s.a=4;s.b;}").unwrap();
    let env = type_env(&program).unwrap();
    let body = type_function_with_env(&program, 0, &env).unwrap();
    let fields = body
        .exprs
        .iter()
        .find_map(|expr| match &expr.kind {
            ir::ExprKind::Struct { fields, .. } => Some(fields),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        fields.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
        vec![1, 0]
    );
    assert!(
            body.body.statements.iter().any(
                |stmt| matches!(stmt,ir::Stmt::AssignPlace{place,..} if place.projection==vec![ir::Projection::Field(0)])
            )
        );
}

#[test]
fn structs_reject_invalid_unused_declarations_and_nominal_mismatches() {
    for source in [
        "struct A{x:A} fn main(){}",
        "struct A{x:(B,)} struct B{x:A} fn main(){}",
        "struct A{x:Missing} fn main(){}",
        "struct A{x:i32,x:bool} fn main(){}",
        "struct A{} fn main(){}",
        "struct A{x:i32} struct B{x:i32} fn main(){let x:A=B{x:1};}",
        "struct A{x:i32} struct B{x:i32} fn main(){if true{A{x:1}}else{B{x:1}};}",
        "struct A{x:i32} fn main(){A{x:true};}",
        "struct A{x:i32} fn main(){A{};}",
        "struct A{x:i32} fn main(){A{x:1,x:2};}",
        "struct A{x:i32} fn main(){A{x:1,y:2};}",
        "struct A{x:i32} fn main(){A{x:1}==A{x:1};}",
    ] {
        assert!(
            parse(source).and_then(|program| check(&program)).is_err(),
            "accepted {source}"
        );
    }
}

#[test]
fn struct_privacy_and_type_value_namespaces_are_distinct() {
    let legal = "mod m{pub struct S{x:i32} pub fn S()->i32{7} pub fn make()->S{S{x:1}} mod child{fn read(s:super::S)->i32{s.x}}} use m::S; fn main(){let s=m::make();S();}";
    check(&parse(legal).unwrap()).unwrap();
    for source in [
        "mod m{pub struct S{x:i32}} fn main(){m::S{x:1};}",
        "mod m{pub struct S{x:i32} pub fn make()->S{S{x:1}}} fn main(){m::make().x;}",
        "mod m{struct S{pub x:i32}} fn main(){let x:m::S=loop{};}",
        "mod m{} fn main(){let x:m=loop{};}",
        "mod S{} struct S{x:i32} fn main(){}",
    ] {
        assert!(
            parse(source).and_then(|program| check(&program)).is_err(),
            "accepted {source}"
        );
    }
}

#[test]
fn inferred_private_types_are_not_exposed_through_calls_or_public_fields() {
    for source in [
        "mod m{struct S{pub x:i32} pub fn make()->S{S{x:1}}} fn main(){m::make();}",
        "mod m{struct H{pub x:i32} pub struct O{pub inner:H} pub fn make()->O{O{inner:H{x:1}}}} fn main(){m::make().inner;}",
        "mod m{struct H{x:i32} pub fn take(x:H){}} fn main(){m::take(loop{});}",
        "mod m{struct H{pub x:i32} pub struct O{pub inner:H} pub fn make()->O{O{inner:H{x:1}}}} fn main(){let mut s=m::make();s.inner.x=2;}",
        "mod m{struct H{pub x:i32} pub struct O{pub inner:H} pub fn make()->O{O{inner:H{x:1}}}} fn main(){let mut s=m::make();s.inner=loop{};}",
        "mod m{struct H{pub x:i32} pub struct O{pub inner:H}} fn main(){m::O{inner:loop{}};}",
    ] {
        assert!(
            parse(source).and_then(|program| check(&program)).is_err(),
            "accepted {source}"
        );
    }
}

#[test]
fn struct_conditions_allow_parenthesized_literals_and_nested_blocks() {
    check(&parse("struct S{x:bool} fn test(s:S)->bool{s.x} fn main(){if (S{x:true}).x {} while test(S{x:false}) {} if {let s=S{x:true};s.x} {}}").unwrap()).unwrap();
}

#[test]
fn struct_expanded_depth_is_bounded_independent_of_declaration_order() {
    let mut source = "struct S0{x:i32}".to_owned();
    for index in 1..140 {
        source.push_str(&format!("struct S{index}{{x:S{}}}", index - 1));
    }
    source.push_str("fn main(){}");
    let program = parse(&source).unwrap();
    assert!(type_env(&program).unwrap_err().contains("nesting"));
    let mut source = "struct S0{x:i32}".to_owned();
    for index in 1..40 {
        source.push_str(&format!(
            "struct S{index}{{a:S{},b:S{}}}",
            index - 1,
            index - 1
        ));
    }
    source.push_str("fn f(x:S39){x;} fn main(){}");
    let program = parse(&source).unwrap();
    let env = type_env(&program).unwrap();
    // This fixture intentionally has an exponentially large value layout.
    // Its shared semantic DAG is shallow, independent of backend/analysis
    // resource limits when expanding concrete move paths or storage.
    type_function_with_env(&program, 0, &env).unwrap();
}

#[test]
fn struct_dependency_width_is_not_dependency_depth() {
    let fields = (0..140)
        .map(|i| format!("f{i}:Leaf"))
        .collect::<Vec<_>>()
        .join(",");
    let program = parse(&format!(
        "struct Leaf{{x:i32}} struct Wide{{{fields}}} fn main(){{}}"
    ))
    .unwrap();
    let env = type_env(&program).unwrap();
    assert_eq!(
        env.structs[&fixture_key(&program, &program.structs[1].name, Namespace::Type)]
            .def()
            .fields
            .len(),
        140
    );
    let values = (0..140)
        .rev()
        .map(|i| format!("f{i}:Leaf{{x:1}}"))
        .collect::<Vec<_>>()
        .join(",");
    let program = parse(&format!(
        "struct Leaf{{x:i32}} struct Wide{{{fields}}} fn main(){{Wide{{{values}}};}}"
    ))
    .unwrap();
    check(&program).unwrap();
}

#[test]
fn module_paths_are_canonical_and_main_stays_root() {
    let p = parse(
            "mod a { fn hidden()->i32{1} pub mod b { pub fn run()->i32 {super::hidden()+super::super::root()} } pub fn run()->i32{self::b::run()} } fn root()->i32{2} fn main(){let x=crate::a::run(); let y=a::b::run();}",
        ).unwrap();
    check(&p).unwrap();
    let names = ["a::hidden", "a::b::run", "a::run", "root", "main"];
    assert_eq!(p.functions.len(), names.len());
    for (function, expected) in p.functions.iter().zip(names) {
        assert!(canonical_path_matches(&p, &function.name, expected));
    }
    let Expr::Binary { left, right, .. } = p.functions[1].body.tail.as_deref().unwrap() else {
        panic!()
    };
    assert!(
        matches!(left.as_ref(), Expr::Call { name, .. } if canonical_path_matches(&p, name, "a::hidden"))
    );
    assert!(
        matches!(right.as_ref(), Expr::Call { name, .. } if canonical_path_matches(&p, name, "root"))
    );
}
#[test]
fn syntax_and_resolution_are_separate() {
    let parsed = parse_unresolved("use crate::missing as alias; fn main(){nope();}").unwrap();
    assert!(canonical_path_matches(
        &parsed.program,
        &parsed.program.functions[0].name,
        "main"
    ));
    assert!(resolve(&parsed).unwrap_err().contains("unresolved"));
    let parsed = parse_unresolved("mod a{fn f(){}} fn main(){a::f();}").unwrap();
    assert!(resolve(&parsed).unwrap_err().contains("private"));
    // Body type errors remain independent of declaration/name preparation.
    let parsed = parse_unresolved("fn bad()->i32{true} fn main(){}").unwrap();
    let p = resolve(&parsed).unwrap();
    let signatures = signature_table(&p).unwrap();
    assert!(check_function_with_signatures(&p, 0, &signatures).is_err());
    check_function_with_signatures(&p, 1, &signatures).unwrap();
}
#[test]
fn forward_function_and_module_aliases() {
    good(
        "use next as renamed; use crate::a as next; use renamed::f as call; mod a{pub fn f()->i32{4}} fn main(){let x=call(); let y=renamed::f();}",
    );
    good(
        "mod a { fn f(){} mod child {use super::f as call; pub fn run(){call();}} pub fn run(){child::run();}} fn main(){a::run();}",
    );
    good("mod a{use self as me; pub fn f(){}} fn main(){}");
    good("use crate as root; fn f(){} fn main(){root::f();}");
    bad("use other as one; use one as other; fn main(){}");
    bad("use crate::absent; fn main(){}");
    bad("mod a{fn f(){}} use crate::a::f as unused; fn main(){}");
}
#[test]
fn private_items_accessible_only_from_owner_and_descendants() {
    good(
        "mod a{fn f(){} mod inner{pub fn g(){super::f();}} pub fn run(){inner::g();}} fn main(){a::run();}",
    );
    good(
        "mod a{fn f(){} use self::f as alias; mod b{pub fn f(){super::alias();}} pub fn run(){b::f();}} fn main(){a::run();}",
    );
    bad("mod a{fn f(){}} mod b{pub fn run(){crate::a::f();}} fn main(){b::run();}");
    bad("mod a{mod hidden{pub fn f(){}}} fn main(){a::hidden::f();}");
    bad("mod a{pub fn f(){} use self::f as alias;} fn main(){a::alias();}");
    // Component boundaries, not textual prefixes, establish descendants.
    bad("mod a{fn f(){}} mod ab{pub fn g(){crate::a::f();}} fn main(){}");
}
#[test]
fn module_scopes_do_not_inherit_unqualified_parent_items() {
    bad("fn f(){} mod a{pub fn g(){f();}} fn main(){}");
    bad("mod a{pub fn f(){}} mod b{pub fn g(){a::f();}} fn main(){}");
    good("mod a{pub fn f(){}} mod b{use crate::a; pub fn g(){a::f();}} fn main(){b::g();}");
    bad("fn main(){super::f();}");
    bad("mod a{pub fn f(){super::super::g();}} fn main(){}");
    bad("mod a{fn main(){}}");
}
#[test]
fn source_shadowing_is_checked_before_canonicalization() {
    good("fn f(){} fn main(){let f=1;crate::f();self::f();}");
    good("mod a{pub fn f(){}} use a::f as alias; fn main(){let f=1;alias();}");
    good("mod a{pub fn f(){}} fn main(){let a=1;a::f();}");
    good("fn f()->i32{1} fn main(){let f=f();}");
    bad("fn f(){} fn main(){let f=1;f();}");
    bad("mod a{pub fn f(){}} use a::f as alias; fn main(){let alias=1;alias();}");
    bad("mod a{fn f(){} pub fn g(f:i32){f();}} fn main(){}");
    good("fn f(){} fn main(){{let f=1;}f();}");
}
#[test]
fn separate_namespaces_and_import_collisions() {
    good(
        "mod same{pub fn value()->i32{3}} fn same()->i32{7} fn main(){let x=same()+same::value();}",
    );
    good(
        "mod same{pub fn value()->i32{3}} fn same()->i32{7} use crate::same as alias; fn main(){let x=alias()+alias::value();}",
    );
    good("mod a{pub fn f(){}} use a::f as a; fn main(){a();a::f();}");
    good(
        "mod outer{mod a{pub fn f(){}} use a::f as a; pub fn run(){a();a::f();}} fn main(){outer::run();}",
    );
    good(
        "mod a{pub mod same{pub fn f(){}} fn same(){}} use a::same as alias; fn main(){alias::f();}",
    );
    good("mod a{mod same{} pub fn same(){}} use a::same as alias; fn main(){alias();}");
    bad("mod a{mod same{} fn same(){}} use a::same as alias; fn main(){}");
    good("mod a{} fn f(){} use a as x; use f as x; fn main(){x();}");
    bad("fn f(){} use f as x; use f as x; fn main(){}");
    bad("mod a{} use a as x; mod x{} fn main(){}");
    bad("fn f(){} use f as x; fn x(){} fn main(){}");
}
#[test]
fn grouped_use_trees_preserve_paths_and_identity() {
    good(
        "mod api{pub struct S{pub x:i32} pub mod inner{pub fn f()->i32{3}}} use crate::{api::{self as a,S as T,inner::{f as call,},},}; fn id(s:T)->a::S{s} fn main(){let s=id(T{x:call()});let n=s.x;}",
    );
    good(
        "mod a{fn f(){} mod b{use super::{f as call};use crate::{a as root};pub fn run(){call();root::f();}}pub fn run(){b::run();}}fn main(){a::run();}",
    );
    good(
        "mod a{pub fn f(){}} use {a::{self as first,self as second,f},};fn main(){f();first::f();second::f();}",
    );
    good("enum E{A,B}use E::{A as First,B,};fn main(){let n=match First{First=>1,B=>2};}");
    good("mod a{} use {};use a::{};fn main(){}");
    good("mod a{pub mod b{}}use a::{b::self as x};fn main(){}");
    good("trait T{fn f(self);}use T::{};use T::{self as Alias};fn main(){}");
    good("fn f(){}use {self as root};use crate::{self as other};fn main(){root::f();other::f();}");
    good(
        "use {next as renamed};use crate::{a as next};use renamed::{f as call};mod a{pub fn f(){}}fn main(){call();}",
    );
    good(
        "mod a{fn f(){}mod b{mod c{use super::super::{f as call};pub fn run(){call();}}pub fn run(){self::c::run();}}pub fn run(){b::run();}}fn main(){a::run();}",
    );
}
#[test]
fn grouped_self_is_type_namespace_only() {
    good(
        "mod a{pub fn f(){}}fn a(){}use a::{self as alias};fn alias(){}fn main(){alias();alias::f();}",
    );
    bad("fn f(){}use f::{self as alias};fn main(){}");
    bad("mod a{}fn a(){}use a::{self as alias};fn main(){alias();}");
    bad("struct S{x:i32}use S::{self as T};fn id(s:T)->S{s}fn main(){let s=id(T{x:1});}");
}
#[test]
fn grouped_imports_reject_collisions_privacy_and_cycles() {
    for source in [
        "mod a{pub fn f(){}}use a::{f,f};fn main(){}",
        "mod a{pub fn f(){}pub fn g(){}}use a::{f as x,g as x};fn main(){}",
        "mod a{fn f(){}}use a::{f};fn main(){}",
        "mod a{pub mod b{pub fn f(){}}}mod c{use crate::a::{b as alias};}fn main(){c::alias::f();}",
        "use {a as b,b as a};fn main(){}",
        "use missing::{};fn main(){}",
        "mod a{mod hidden{}}use a::hidden::{};fn main(){}",
    ] {
        bad(source);
    }
    good("mod a{}fn f(){}use {a as x,f as x};fn main(){x();}");
}
#[test]
fn grouped_use_syntax_and_resource_fences() {
    for source in ["use a::{*};fn main(){}", "use a::{f,*};fn main(){}"] {
        assert!(parse(source).unwrap_err().contains("glob imports"));
    }
    for source in [
        "use a::{,};fn main(){}",
        "use a::{f,,};fn main(){}",
        "use a::{f g};fn main(){}",
        "use a::{f} as x;fn main(){}",
        "use a{f};fn main(){}",
    ] {
        assert!(parse_unresolved(source).is_err(), "accepted {source}");
    }
    let nested = format!("use {}f{};fn main(){{}}", "{".repeat(129), "}".repeat(129));
    assert!(parse_unresolved(&nested).unwrap_err().contains("nesting"));
    let wide = format!("use {{{}}};fn main(){{}}", "f,".repeat(4097));
    assert!(parse_unresolved(&wide).unwrap_err().contains("work"));
    let long = "a".repeat(1024 * 1024 + 1);
    assert!(
        parse_unresolved(&format!("use {long};fn main(){{}}"))
            .unwrap_err()
            .contains("allocation")
    );
}
#[test]
fn grouped_use_whole_source_expansion_is_bounded() {
    let many = format!("{}fn main(){{}}", "use a::{};".repeat(65537));
    let error = parse_unresolved(&many).unwrap_err();
    assert!(error.contains("count exceeds"), "{error}");
    let prefix = "a".repeat(40000);
    let expanded = format!(
        "mod {prefix}{{{}}}fn main(){{}}",
        "use crate::{};".repeat(420)
    );
    assert!(
        parse_unresolved(&expanded)
            .unwrap_err()
            .contains("logical expansion")
    );
}
#[test]
fn public_named_reexports_preserve_canonical_identity() {
    good(
        "mod hidden{pub fn f()->i32{7}pub struct S{pub x:i32}pub trait T{fn f(self);}} pub use hidden::{f as call,S as Alias,T};fn id(s:Alias)->hidden::S{s}fn main(){let s=id(Alias{x:call()});}",
    );
    good(
        "pub use next as first;pub use hidden::f as next;mod hidden{pub fn f(){}}fn main(){first();next();}",
    );
    good(
        "mod hidden{pub mod api{pub fn f(){}}}pub use hidden::{api::{self as exported,f as call}};fn main(){exported::f();call();}",
    );
    good(
        "pub mod a{pub fn f(){}}fn a(){}pub use a::{self as alias};fn alias(){}fn main(){alias();alias::f();}",
    );
    good("mod a{}pub fn f(){}pub use {f as x};use a as x;fn main(){x();}");
    good("pub mod a{pub use self as alias;}fn main(){}");
    good("pub use crate::{self as root};fn main(){}");
    good(
        "mod a{pub struct Mixed{pub x:i32}fn Mixed(){}pub use self::Mixed as Alias;pub fn run(){Alias();}}fn main(){a::run();let x=a::Alias{x:1};}",
    );
    good("mod a{pub enum E{A}}pub use a::E::A;fn main(){let x=A;}");
}
#[test]
fn restricted_visibility_normalizes_ast_and_registry_scopes() {
    let program = parse(
            "mod a{pub(crate) struct S{pub(super) x:i32}pub(in crate::a) enum E{A}pub(self) trait T{fn f(self);}mod b{pub(in super::super) fn f(){}}}fn main(){}",
        ).unwrap();
    let root = program.modules.source_root().unwrap();
    let a = program.structs[0].module;
    assert_eq!(
        program.structs[0].visibility,
        ir::Visibility::Restricted(root)
    );
    assert_eq!(
        program.structs[0].fields[0].visibility,
        ir::Visibility::Restricted(root)
    );
    assert_eq!(program.enums[0].visibility, ir::Visibility::Restricted(a));
    assert_eq!(program.traits[0].visibility, ir::Visibility::Restricted(a));
    assert_eq!(
        program.functions[0].visibility,
        ir::Visibility::Restricted(root)
    );
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&program, &budget).unwrap();
    for (module, path, namespace, scope) in [
        (a, &program.structs[0].name, Namespace::Type, root),
        (
            program.functions[0].module,
            &program.functions[0].name,
            Namespace::Value,
            root,
        ),
        (a, &program.enums[0].name, Namespace::Type, a),
        (a, &program.traits[0].name, Namespace::Type, a),
    ] {
        let expected = *path.segments().last().unwrap();
        let info = plan
            .registry
            .declarations
            .iter()
            .find(|info| {
                info.path.module == module
                    && matches!(info.path.item,
                    DeclItemName::Source(actual) if name_scope::ident_eq(
                        program.names.as_ref(), actual, expected,
                        LookupClass::Item(namespace),
                    ).unwrap())
            })
            .unwrap();
        assert_eq!(info.visibility, ir::Visibility::Restricted(scope));
    }
}
#[test]
fn restricted_visibility_rejects_unused_nonlexical_paths_and_explicit_trait_items() {
    for source in [
        "mod a{}mod b{pub(in crate::a) fn f(){}}fn main(){}",
        "mod a{use crate::a as alias;mod b{pub(in crate::a::alias) fn f(){}}}fn main(){}",
        "mod a{pub(in self::super) fn f(){}}fn main(){}",
        "pub(super) fn f(){}fn main(){}",
        "trait T{pub(self) fn f(self);}fn main(){}",
        "trait T{fn f(self);}struct S{x:i32}impl T for S{pub(self) fn f(self){}}fn main(){}",
        "enum E{A{pub(self) x:i32}}fn main(){}",
    ] {
        assert!(parse_unresolved(source).is_err(), "accepted {source}");
    }
    let oversized = format!(
        "pub(in crate::{}) fn f(){{}}fn main(){{}}",
        "a::".repeat(1024) + "a"
    );
    assert!(
        parse_unresolved(&oversized)
            .unwrap_err()
            .contains("visibility path nesting")
    );
}
#[test]
fn module_arena_capacity_is_retained_until_last_clone_owner() {
    let public = parse("pub fn main(){}").unwrap();
    let budget = public.names.budget().clone();
    let cloned = public.clone();
    assert!(std::sync::Arc::ptr_eq(&public.modules, &cloned.modules));
    let retained = budget.snapshot().retained_bytes;
    assert!(retained > 0);
    drop(public);
    assert_eq!(budget.snapshot().retained_bytes, retained);

    // Restricted visibility is now an inline ModuleId, not a cloned 4096-byte
    // String. Its authority is retained by this shared, charged module arena.
    let modules = cloned.modules.clone();
    drop(cloned);
    let arena_bytes = budget.snapshot().retained_bytes;
    assert!(arena_bytes > 0);
    let last_owner = modules.clone();
    drop(modules);
    assert_eq!(budget.snapshot().retained_bytes, arena_bytes);
    drop(last_owner);
    assert_eq!(budget.snapshot().retained_bytes, 0);
}
#[test]
fn module_parser_depth_limit_uses_an_ordinary_stack() {
    for depth in [128, 129, 300] {
        let source = format!(
            "{}{}fn main(){{}}",
            "mod a{".repeat(depth),
            "}".repeat(depth)
        );
        let result = parse_unresolved(&source);
        if depth == 128 {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert!(
                result
                    .unwrap_err()
                    .contains("module nesting exceeds limit 128")
            );
        }
    }
}
#[test]
fn public_reexports_validate_unused_originals_and_aliases() {
    for source in [
        "enum E{A}pub use E::A;fn main(){}",
        "mod a{pub struct Mixed{pub x:i32}fn Mixed(){}pub use self::Mixed as Alias;}fn main(){a::Alias();}",
        "fn f(){}pub use f as alias;fn main(){}",
        "struct S{x:i32}pub use S as Alias;fn main(){}",
        "enum E{A}pub use E as Alias;fn main(){}",
        "trait T{fn f(self);}pub use T as Alias;fn main(){}",
        "mod a{}pub use a as alias;fn main(){}",
        "pub fn f(){}use f as private;pub use private as alias;fn main(){}",
        "mod a{pub fn f(){}use self::f as private;pub use self::private as alias;}fn main(){}",
        "mod a{pub fn f(){}use self::f as private;}pub use a::private as alias;fn main(){}",
        "pub use later as first;use f as later;pub fn f(){}fn main(){}",
        "pub use second as first;pub use first as second;fn main(){}",
        "pub mod a{}pub use a as alias;pub use a as alias;fn main(){}",
        "pub fn f(){}pub use f::{self as alias};fn main(){}",
        "mod a{pub use self as alias;}fn main(){}",
        "mod a{pub mod b{pub use super as alias;}}fn main(){}",
    ] {
        bad(source);
    }
}
#[test]
fn legal_imported_main_is_an_explicit_unsupported_boundary() {
    for source in [
        "mod a{pub fn f(){}}pub use a::f as main;",
        "mod a{pub fn f(){}}use a::f as main;",
    ] {
        assert!(
            parse(source)
                .unwrap_err()
                .contains("imported main entry points are unsupported")
        );
    }
}
#[test]
fn unsupported_module_forms_are_rejected_explicitly() {
    for (source, message) in [
        ("mod a; fn main(){}", "external module"),
        (
            "type Alias<T> where T:Copy=T;fn main(){}",
            "type alias bounds and where clauses are unsupported",
        ),
        ("use crate::a::*; fn main(){}", "glob imports"),
        ("pub use crate::a::*; fn main(){}", "glob imports"),
        ("mod a{} mod a{} fn main(){}", "duplicate item"),
    ] {
        assert!(parse(source).unwrap_err().contains(message), "{source}");
    }
    bad("mod a{} fn main(){a();}");
    bad("fn f(){} fn main(){f::g();}");
    bad("fn main(){use crate::a;}");
    let nested = format!("{}{}fn main(){{}}", "mod a{".repeat(300), "}".repeat(300));
    assert!(parse(&nested).unwrap_err().contains("nesting"));
}
#[test]
fn source_aliases_erase_binders_without_nominal_or_value_declarations() {
    for source in [
        "type A<T>=B<(T,T)>;type B<T>=T;fn f<U>(x:A<U>)->(U,U){x}fn main(){let p=f::<i32>((2,3));}",
        "type T=A<i32>;type A<T>=T;fn main(){let n:T=7;}",
        "mod a{mod hidden{pub struct S{pub x:i32}}pub type A=hidden::S;}fn main(){let a=a::A{x:7};let a::A{x}=a;}",
        "struct S<T>{x:T}type A=S<i32>;fn main(){let a=A{x:7};let A{x}=a;}",
        "enum E<T>{V(T),N}type A<T>=E<T>;fn main(){let a=A::<i32>::V(7);match a{A::<i32>::V(x)=>{let n=x;},A::<i32>::N=>{}}}",
        "struct S{x:i32}type A=S;fn A()->i32{7}fn main(){let n=A();let A=8;let n=A;let s=crate::A{x:n};}",
    ] {
        let program = parse(source).unwrap_or_else(|e| panic!("{source}: {e}"));
        check(&program).unwrap_or_else(|e| panic!("{source}: {e}"));
    }
}
#[test]
fn unused_aliases_validate_targets_arity_parameters_and_cycles() {
    for (source, expected) in [
        ("type A=Missing;fn main(){}", "unresolved type"),
        ("struct S<T>{x:T}type A=S;fn main(){}", "E0107"),
        ("type A<T>=i32;fn main(){}", "E0091"),
        ("type A<T>=A<(T,T)>;fn main(){}", "E0391"),
        ("type A=B;type B=A;fn main(){}", "E0391"),
        ("type A=(_,(i32,_));fn main(){}", "E0121"),
        (
            "struct S{x:i32}type A=S;use A::{self as B};fn main(){}",
            "not a module",
        ),
    ] {
        let error = parse(source).unwrap_err();
        assert!(error.contains(expected), "{source}: {error}");
    }
}
#[test]
fn alias_resolution_is_bounded_on_a_normal_128k_stack() {
    let source = (0..40)
        .map(|i| format!("type A{i}=A{};", i + 1))
        .collect::<String>()
        + "type A40=i32;fn main(){}";
    let parsed = parse_unresolved(&source).unwrap();
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || {
            resolve(&parsed).unwrap();
        })
        .unwrap()
        .join()
        .unwrap();
    let source = "type A<T>=B<(T,T)>;type B<T>=A<(T,T)>;fn main(){}";
    let parsed = parse_unresolved(source).unwrap();
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || {
            assert!(resolve(&parsed).unwrap_err().contains("E0391"));
        })
        .unwrap()
        .join()
        .unwrap();
}
fn good(s: &str) {
    let p = parse(s).unwrap();
    check(&p).unwrap();
}
fn bad(s: &str) {
    assert!(parse(s).and_then(|p| check(&p)).is_err(), "accepted {s}");
}
#[test]
fn escapes_comparisons_and_reserved() {
    let p = parse(r#"fn main(){println!("\u{1f980}\x21");}"#).unwrap();
    assert!(
        matches!(&p.functions[0].body.statements[0], Stmt::Expr(Expr::Println(s)) if s == "🦀!")
    );
    good("fn main(){let x = (1 < 2) == true; let y = 1<2 && 3<4;}");
    bad("fn main(){let x = true == 1 < 2;}");
    bad("fn try(){} fn main(){}");
    bad("fn main(){let x = 1; let x = true; x=2;}");
    for s in [
        r#"fn main(){println!("\u{d800}");}"#,
        r#"fn main(){println!("\x80");}"#,
        r#"fn main(){println!("\u{}");}"#,
    ] {
        bad(s);
    }
}
#[test]
fn nesting_limits() {
    let nested = format!(
        "fn main(){{let x={}1{};}}",
        "(".repeat(300),
        ")".repeat(300)
    );
    assert!(parse(&nested).unwrap_err().contains("nesting"));
    let chain = format!("fn main(){{let x=1{};}}", "+1".repeat(10000));
    assert!(parse(&chain).unwrap_err().contains("nesting"));
    let blocks = format!("fn main(){{{}{} }}", "{".repeat(300), "}".repeat(300));
    assert!(parse(&blocks).unwrap_err().contains("nesting"));
}
#[test]
fn independent_checks() {
    let p = parse("fn wrong()->i32 {true} fn main(){}").unwrap();
    check_signatures(&p).unwrap();
    assert!(check_function(&p, 0).is_err());
    check_function(&p, 1).unwrap();
    assert!(check_function(&p, 2).is_err());
}
#[test]
fn recursion_and_flow() {
    good(
        "fn fib(n:i32)->i32 { if n < 2 { return n; } fib(n-1)+fib(n-2) } fn main(){ let mut x = fib(4); while x > 0 { x = x - 1; } println!(\"ok\\n{{done}}\"); }",
    );
}
#[test]
fn scopes() {
    good("fn main(){let x=1; let x=x+1; {let x=true; let y=x && !false;} let y=x+1;}");
    bad("fn main(){let x=1; x=2;}");
    bad("fn main(){{let x=1;} let y=x;}");
    bad("fn f(x:i32,x:i32){} fn main(){}");
}
#[test]
fn return_flow() {
    good("fn f(b:bool)->i32 {if b {return 1;} else {return 2;}} fn main(){}");
    bad("fn f(b:bool)->i32 {if b {return 1;}} fn main(){}");
    bad("fn f()->i32 {while true {return 1;}} fn main(){}");
    bad("fn f()->i32 {false && {return 1;};} fn main(){}");
    good("fn f()->i32 {return 1; false;} fn main(){}");
}
#[test]
fn rejects_invalid() {
    for s in [
        "fn main(x:i32){}",
        "fn main()->i32{0}",
        "fn main(){foo();}",
        "fn f(x:i32){} fn main(){f();}",
        "fn main(){let x:bool=1;}",
        "fn main(){let café=1;}",
        "fn main(){println!(\"{}\");}",
        "fn main(){let x=2147483648;}",
        "fn main(){let x=1 < 2 < 3;}",
        "fn main(){} fn main(){}",
        "fn main(){let x=1; x();}",
    ] {
        bad(s);
    }
}
#[test]
fn literals_and_comments() {
    good("/* a /* b */ */ fn main(){let x=-2147483648; let y:()=(); println!(\"a\\t\\\"\\\\\");}");
    bad("fn main(){println!(\"\\q\");}");
    assert!(parse("\nfn main(){@}").unwrap_err().starts_with("2:11:"));
}
