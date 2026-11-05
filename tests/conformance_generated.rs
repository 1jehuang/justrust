//! Bounded cross-feature differential probes, not exhaustive Rust conformance.
//! Reproduce a case by its test name and the fixed seed printed with its source.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

const SEED: u64 = 0x6a09_e667_f3bc_c909;
const VALID_CASES: usize = 16;
const INVALID_CASES: usize = 8;

// Each ID has its own state: adding or reordering tests cannot perturb others.
// Arithmetic operands stay in 1..=13, and each source has at most two closures.
fn operands(id: usize) -> (i32, i32) {
    let mut state = SEED ^ (id as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    ((state % 13 + 1) as i32, ((state >> 16) % 13 + 1) as i32)
}

fn valid(id: usize) -> (String, String) {
    assert!(id < VALID_CASES);
    if id >= 12 {
        return match id {
            12 => (
                r#"fn main(){let mut n=3i32;let r=&mut n;let mut f=||{let s=&mut *r;let mut g=||{*s+=2i32;*s};g()};println!("{}",f());println!("{}",n);}"#.into(),
                "5\n5\n".into(),
            ),
            13 => (
                r#"struct Guard{n:i32}impl Drop for Guard{fn drop(&mut self){println!("drop {}",self.n);}}fn main(){let guard=Guard{n:7};let mut n=3i32;let r=&mut n;let mut f=||{*r+=2i32;*r};println!("{}",f());drop(guard);}"#.into(),
                "5\ndrop 7\n".into(),
            ),
            14 => (
                r#"fn main(){let mut pair=(3i32,4i32);let shared=&pair.0;let mutable=&mut pair.1;let mut f=||{*mutable+=*shared;*mutable};println!("{}",f());println!("{} {}",pair.0,pair.1);}"#.into(),
                "7\n3 7\n".into(),
            ),
            15 => (
                r#"struct Guard{n:i32}impl Drop for Guard{fn drop(&mut self){println!("drop {}",self.n);}}fn main(){let pair=(3i32,4i32);let r=&pair.0;let guard=Guard{n:7};let f=move||{let value=*r+guard.n;drop(guard);value};println!("{}",f());println!("{}",pair.1);}"#.into(),
                "drop 7\n10\n4\n".into(),
            ),
            _ => unreachable!(),
        };
    }
    let (a, b) = operands(id);
    match id % 4 {
        0 => (
            format!(
                "fn identity<T>(x:T)->T{{x}} fn apply<F:Fn(i32)->i32>(f:F,x:i32)->i32{{f(x)}} fn main(){{let pair=identity(({a}i32,{b}i32));let r=&pair.0;let f=|x:i32|{{let g=|y:i32|y+1;g(x)+*r}};let result=Some(apply(f,pair.1));match result{{Some(n)=>println!(\"{{}}\",n),None=>println!(\"none\")}}}}"
            ),
            format!("{}\n", a + b + 1),
        ),
        1 => (
            format!(
                "fn identity<T>(x:T)->T{{x}} fn apply<F:FnMut(i32)->i32>(mut f:F,x:i32)->i32{{f(x)}} fn main(){{let pair=identity(({a}i32,{b}i32));let mut n=pair.0;let r=&mut n;let f=|x:i32|{{*r+=x;*r}};let result=Some(apply(f,pair.1));match result{{Some(value)=>println!(\"{{}}\",value),None=>println!(\"none\")}}println!(\"{{}}\",n);}}"
            ),
            format!("{0}\n{0}\n", a + b),
        ),
        2 => (
            format!(
                "struct Guard{{n:i32}} impl Drop for Guard{{fn drop(&mut self){{println!(\"drop {{}}\",self.n);}}}} fn identity<T>(x:T)->T{{x}} fn apply<F:Fn(i32)->i32>(f:F,x:i32)->i32{{f(x)}} fn main(){{let guard=Guard{{n:{a}}};let pair=identity(({a}i32,{b}i32));let f=|x:i32|{{let g=|y:i32|y+1;g(x)+guard.n}};match Some(apply(f,pair.1)){{Some(n)=>println!(\"{{}}\",n),None=>println!(\"none\")}}}}"
            ),
            format!("{}\ndrop {a}\n", a + b + 1),
        ),
        3 => (
            format!(
                "fn identity<T>(x:T)->T{{x}} fn apply<F:FnOnce()->i32>(f:F)->i32{{f()}} fn main(){{let pair=identity(({a}i32,{b}i32));let r=&pair.1;let f=||{{let g=|x:i32|x+*r;g(pair.0)}};match Some(apply(f)){{Some(n)=>println!(\"{{}}\",n),None=>println!(\"none\")}}}}"
            ),
            format!("{}\n", a + b),
        ),
        _ => unreachable!(),
    }
}

fn invalid(id: usize) -> (String, &'static str) {
    assert!(id < INVALID_CASES);
    // Preserve original invalid fixture sources when adding valid probes.
    let (a, b) = operands(12 + id);
    // Every defect is in an uncalled function. Main alone is deliberately valid.
    let (body, code) = match id {
        0 => (
            format!(
                "fn unused(){{let pair=({a}i32,{b}i32);let f=||->bool{{pair.0}};let value=Some(f());}}"
            ),
            "E0308",
        ),
        1 => (
            format!(
                "fn unused(){{let pair=({a}i32,{b}i32);let mut n=pair.0;let f=||{{n+=pair.1;}};f();}}"
            ),
            "E0596",
        ),
        2 => (
            format!(
                "fn unused(){{let pair=({a}i32,{b}i32);let mut n=pair.0;let f=||n+pair.1;n=0;let result=Some(f());}}"
            ),
            "E0506",
        ),
        3 => (
            format!(
                "fn unused(){{let pair=({a}i32,{b}i32);let mut n=pair.0;let mut f=||{{n+=pair.1;n}};let value=n;let result=Some(f());}}"
            ),
            "E0503",
        ),
        4 => (
            format!(
                "fn apply<F:Fn()->i32>(f:F)->i32{{f()}} fn unused(){{let pair=({a}i32,{b}i32);let mut n=pair.0;let f=||{{n+=pair.1;n}};let result=Some(apply(f));}}"
            ),
            "E0525",
        ),
        5 => (
            format!(
                "struct Item{{n:i32}} fn identity<T>(x:T)->T{{x}} fn unused(){{let item=identity(Item{{n:{a}}});let f=move||item;let result=Some(f());let second=f();}}"
            ),
            "E0382",
        ),
        6 => (
            format!(
                "fn requires_copy<T:Copy>(x:T){{}} fn unused<T>(value:T){{let pair=({a}i32,{b}i32);let f=move||{{requires_copy(value);pair.0}};let result=Some(f());}}"
            ),
            "E0277",
        ),
        7 => (
            format!(
                "fn identity<T>(x:T)->T{{x}} fn unused(){{let r;{{let pair=identity(({a}i32,{b}i32));r=&pair.0;}}let f=||*r;let result=Some(f());}}"
            ),
            "E0597",
        ),
        _ => unreachable!(),
    };
    (format!("{body} fn main(){{}}"), code)
}

fn probe(id: usize, is_invalid: bool) {
    let kind = if is_invalid { "invalid" } else { "valid" };
    let tag = format!("generated_seed_{SEED:016x}_{kind}_{id:02}");
    let (source, expectation) = if is_invalid {
        let (source, code) = invalid(id);
        (source, Expect::InvalidAfterParse(code))
    } else {
        let (source, stdout) = valid(id);
        (source, Expect::Run(Box::leak(stdout.into_boxed_str())))
    };
    // The oracle currently takes static strings. These bounded process-lifetime
    // allocations retain exact reproduction text even if the oracle panics.
    let source: &'static str = Box::leak(source.into_boxed_str());
    let name: &'static str = Box::leak(format!("{tag}\nsource:\n{source}").into_boxed_str());
    let outcome = std::panic::catch_unwind(|| {
        oracle::check(
            &tag,
            &[Case {
                name,
                source,
                expect: expectation,
            }],
        );
    });
    if let Err(error) = outcome {
        let detail = error
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| error.downcast_ref::<&str>().copied())
            .unwrap_or("non-string oracle panic");
        panic!("{name}\noracle failure: {detail}");
    }
}

// Independent tests ensure one failing combination does not suppress the rest.
macro_rules! probes {
    ($invalid:literal; $($name:ident = $id:literal),+ $(,)?) => {
        $(#[test] fn $name() { probe($id, $invalid); })+
    };
}
probes!(false;
    valid_00 = 0, valid_01 = 1, valid_02 = 2, valid_03 = 3,
    valid_04 = 4, valid_05 = 5, valid_06 = 6, valid_07 = 7,
    valid_08 = 8, valid_09 = 9, valid_10 = 10, valid_11 = 11,
    valid_12 = 12, valid_13 = 13, valid_14 = 14,
);
// Retain the original source and strict worker-1/8 CHECK/BUILD/run oracle.
#[test]
fn valid_15() {
    probe(15, false);
}

probes!(true;
    unused_invalid_00 = 0, unused_invalid_01 = 1,
    unused_invalid_02 = 2, unused_invalid_03 = 3,
    unused_invalid_04 = 4, unused_invalid_05 = 5,
    unused_invalid_06 = 6, unused_invalid_07 = 7,
);

// Pending language obligations stay independent of the generated seed corpus.
// The compatibility tracker includes ignored tests, so Unsupported is a failure,
// not an accepted substitute for the stock Run/Invalid expectations below.
#[test]
#[ignore = "pending native async future creation and unpolled drop acceptance"]
fn pending_async_block_creation_and_unpolled_drop_match_stock() {
    oracle::check(
        "pending-async-unpolled-drop",
        &[Case {
            name: "async_move_creation_does_not_poll_and_drop_releases_capture",
            source: r#"struct Guard { value: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.value); } }
fn main() {
    let guard = Guard { value: 7 };
    let future = async move {
        let _owner = guard;
        println!("polled");
    };
    println!("created");
    drop(future);
    println!("done");
}
"#,
            expect: Expect::Run("created\ndrop 7\ndone\n"),
        }],
    );
}

#[test]
#[ignore = "pending native unused await-outside-async semantic rejection"]
fn pending_unused_await_outside_async_is_invalid() {
    oracle::check(
        "pending-unused-await-outside-async",
        &[Case {
            name: "unused_non_async_function_cannot_await",
            source: r#"fn unused() {
    let _value = async { 7i32 }.await;
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0728"),
        }],
    );
}

#[test]
#[ignore = "pending native higher-ranked callable function-item acceptance"]
fn pending_higher_ranked_callable_identity_matches_stock() {
    oracle::check(
        "pending-higher-ranked-callable-identity",
        &[Case {
            name: "one_function_item_identity_serves_distinct_borrow_scopes",
            source: r#"fn identity<'a>(value: &'a i32) -> &'a i32 { value }
fn use_identity<F>(f: F)
where F: for<'a> Fn(&'a i32) -> &'a i32 {
    let outer = 17i32;
    println!("{}", *f(&outer));
    {
        let inner = 25i32;
        println!("{}", *f(&inner));
    }
    println!("{}", *f(&outer));
}
fn main() { use_identity(identity); }
"#,
            expect: Expect::Run("17\n25\n17\n"),
        }],
    );
}

#[test]
fn pending_unused_associated_required_copy_bound_is_invalid() {
    oracle::check(
        "pending-unused-associated-required-copy-bound",
        &[Case {
            name: "unused_generic_body_requires_proof_of_projected_copy",
            source: r#"trait Source { type Item; }
fn require_copy<T: Copy>(_value: T) {}
fn unused<S: Source>(value: S::Item) { require_copy(value); }
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0277"),
        }],
    );
}

#[test]
#[ignore = "pending native compatibility obligation"]
fn runtime_repr_c_record_size_and_alignment_match_stock() {
    oracle::check(
        "runtime-repr-c-record-layout",
        &[Case {
            name: "repr_c_u8_u32_u16_size_alignment",
            source: r#"#[repr(C)]
struct Record { tag: u8, count: u32, tail: u16 }
fn main() {
    let value = Record { tag: 3, count: 257, tail: 513 };
    let size = std::mem::size_of::<Record>();
    let alignment = std::mem::align_of::<Record>();
    assert_eq!(size, 12usize);
    assert_eq!(alignment, 4usize);
    assert_eq!(value.tag, 3u8);
    assert_eq!(value.count, 257u32);
    assert_eq!(value.tail, 513u16);
    println!("{} {} {} {} {}", size, alignment, value.tag, value.count, value.tail);
}"#,
            expect: Expect::Run("12 4 3 257 513\n"),
        }],
    );
}

#[test]
fn memory_raw_pointer_cast_roundtrip_and_unsafe_write_match_stock() {
    oracle::check(
        "memory-raw-pointer-cast-roundtrip",
        &[Case {
            name: "mutable_const_mutable_pointer_roundtrip",
            source: r#"fn main() {
    let mut value = 7u32;
    let raw = &mut value as *mut u32;
    let read_only = raw as *const u32;
    let restored = read_only as *mut u32;
    assert!(restored == raw);
    unsafe {
        assert_eq!(*read_only, 7u32);
        *restored = 19u32;
        assert_eq!(*raw, 19u32);
    }
    assert_eq!(value, 19u32);
    println!("{}", value);
}"#,
            expect: Expect::Run("19\n"),
        }],
    );
}

#[test]
fn memory_unused_raw_pointer_write_without_unsafe_is_invalid() {
    oracle::check(
        "memory-unused-raw-pointer-write-unsafe-required",
        &[Case {
            name: "unused_raw_dereference_requires_unsafe",
            source: r#"fn unused(raw: *mut u32) {
    *raw = 19u32;
}
fn main() {}"#,
            expect: Expect::InvalidAfterParse("E0133"),
        }],
    );
}

#[test]
#[ignore = "pending native compatibility obligation"]
fn runtime_atomic_fetch_add_explicit_orderings_match_stock() {
    oracle::check(
        "runtime-atomic-fetch-add-orderings",
        &[Case {
            name: "single_thread_atomic_fetch_add_previous_values",
            source: r#"use std::sync::atomic::{AtomicUsize, Ordering};
fn main() {
    let value = AtomicUsize::new(7usize);
    let first = value.fetch_add(3usize, Ordering::AcqRel);
    let second = value.fetch_add(2usize, Ordering::SeqCst);
    let current = value.load(Ordering::Acquire);
    assert_eq!(first, 7usize);
    assert_eq!(second, 10usize);
    assert_eq!(current, 12usize);
    println!("{} {} {}", first, second, current);
}"#,
            expect: Expect::Run("7 10 12\n"),
        }],
    );
}

// Independent pending source-native obligations. Unsupported, resource and
// infrastructure failures are oracle failures, never successful expectations.
// The scope tracker runs these with --include-ignored.
#[test]
#[ignore = "pending native compatibility obligation"]
fn const_generic_array_identity_source() {
    oracle::check(
        "const-generic-array-identity-source",
        &[Case {
            name: "usize_const_parameter_preserves_array_values_and_length",
            source: r#"fn identity<const N: usize>(value: [i32; N]) -> [i32; N] { value }
fn main() {
    let array = identity::<3>([7i32, 11i32, 24i32]);
    println!("{} {} {} {}", array.len(), array[0], array[1], array[2]);
}"#,
            expect: Expect::Run("3 7 11 24\n"),
        }],
    );
}

#[test]
#[ignore = "pending native compatibility obligation"]
fn const_generic_wrong_argument_type_source() {
    oracle::check(
        "const-generic-wrong-argument-type-source",
        &[Case {
            name: "bool_cannot_substitute_usize_const_parameter_even_in_unused_body",
            source: r#"fn identity<const N: usize>(value: [i32; N]) -> [i32; N] { value }
fn unused() { let _ = identity::<true>([7i32, 11i32, 24i32]); }
fn main() {}"#,
            expect: Expect::InvalidAfterParse("E0308"),
        }],
    );
}

#[test]
#[ignore = "pending native compatibility obligation"]
fn const_fn_aggregate_initializer_source() {
    oracle::check(
        "const-fn-aggregate-initializer-source",
        &[Case {
            name: "const_fn_constructs_struct_with_array_and_scalar_fields",
            source: r#"struct Table { values: [i32; 3], sum: i32 }
const fn make_table() -> Table {
    let values = [7i32, 11i32, 24i32];
    Table { values, sum: values[0] + values[1] + values[2] }
}
const TABLE: Table = make_table();
fn main() {
    println!("{} {} {} {} {}", TABLE.values.len(), TABLE.values[0], TABLE.values[1], TABLE.values[2], TABLE.sum);
}"#,
            expect: Expect::Run("3 7 11 24 42\n"),
        }],
    );
}

#[test]
fn const_fn_unused_runtime_call_source() {
    oracle::check(
        "const-fn-unused-runtime-call-source",
        &[Case {
            name: "unused_const_fn_body_cannot_call_non_const_function",
            source: r#"fn runtime() -> i32 { 42 }
const fn unused() -> i32 { runtime() }
fn main() {}"#,
            expect: Expect::InvalidAfterParse("E0015"),
        }],
    );
}

#[test]
fn semantic_invalid_requires_source_and_declaration_publications() {
    let syntax = justrust_engine::compiler::compile_observed("fn main(){let = ;}".into(), 1, true)
        .err()
        .expect("malformed source must fail");

    let mut semantic = justrust_engine::compiler::compile_observed(
        "fn unused(){let value:i32=true;}fn main(){}".into(),
        1,
        true,
    )
    .err()
    .expect("unused type error must fail");

    assert!(oracle::validate_semantic_rejection(&syntax).is_err());
    oracle::validate_rejection_evidence(&syntax, false).unwrap();
    let gap = oracle::validate_rejection_evidence(&syntax, true).unwrap_err();
    assert_eq!(gap.annotation(), "semantic-stage-unproven");
    assert!(matches!(
        gap,
        oracle::RejectionEvidenceGap::SemanticStageUnproven(_)
    ));
    assert_eq!(
        gap.reason(),
        oracle::validate_semantic_rejection(&syntax).unwrap_err()
    );
    oracle::validate_semantic_rejection(&semantic).unwrap();
    oracle::validate_rejection_evidence(&semantic, true).unwrap();
    semantic.resource_usage.poisoned = true;
    for require_semantic in [false, true] {
        let gap = oracle::validate_rejection_evidence(&semantic, require_semantic).unwrap_err();
        assert_eq!(gap.annotation(), "rejection-proof-unproven");
        assert_eq!(
            gap.reason(),
            "poisoned resource ledger is not semantic rejection"
        );
        assert!(matches!(
            gap,
            oracle::RejectionEvidenceGap::UnhealthyRejection(_)
        ));
    }
    semantic.resource_usage.poisoned = false;
    let report = semantic.report.as_mut().unwrap();
    let declarations = report
        .node_ids
        .iter()
        .zip(&report.node_evaluations)
        .find_map(|(node, (key, _))| (key.kind == 101 && key.params.is_empty()).then_some(*node))
        .expect("declaration fact node");
    let events = report.events.clone();
    report
        .events
        .retain(|event| !(event.kind == "publish" && event.node == Some(declarations)));
    assert!(
        oracle::validate_semantic_rejection(&semantic).is_err(),
        "header parsing alone is insufficient"
    );
    assert_eq!(
        oracle::validate_rejection_evidence(&semantic, true)
            .unwrap_err()
            .annotation(),
        "semantic-stage-unproven"
    );
    semantic.report.as_mut().unwrap().events = events;
    let report = semantic.report.as_mut().unwrap();
    let root = report
        .node_ids
        .iter()
        .zip(&report.node_evaluations)
        .find_map(|(node, (key, _))| (key.kind == 121 && key.params == [0]).then_some(*node))
        .expect("original input parse node");
    report
        .events
        .retain(|event| !(event.kind == "publish" && event.node == Some(root)));
    assert!(
        oracle::validate_semantic_rejection(&semantic).is_err(),
        "other files' parse publications cannot stand in for the original source"
    );
    semantic.report.as_mut().unwrap().capture_complete = false;
    assert_eq!(
        oracle::validate_rejection_evidence(&semantic, true)
            .unwrap_err()
            .annotation(),
        "rejection-proof-unproven",
        "unhealthy capture takes precedence over a missing stage witness"
    );
}

// Literal memory obligations, independent of the generated seed corpus.
#[test]
fn memory_projected_move_remaining_field_drop_matches_stock() {
    oracle::check(
        "memory-projected-move-remaining-field-drop",
        &[Case {
            name: "moved_tuple_field_and_remaining_field_each_drop_once",
            source: r#"struct Guard { n: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.n); } }
fn main() {
    let pair = (Guard { n: 1 }, Guard { n: 2 });
    let first = pair.0;
    println!("remaining {}", pair.1.n);
    drop(first);
    println!("end");
}
"#,
            expect: Expect::Run("remaining 2\ndrop 1\nend\ndrop 2\n"),
        }],
    );
}

#[test]
fn memory_conditional_projected_move_drop_flags_match_stock() {
    oracle::check(
        "memory-conditional-projected-move-drop-flags",
        &[Case {
            name: "taken_and_untaken_move_paths_drop_only_initialized_fields",
            source: r#"struct Guard { n: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.n); } }
fn run(take: bool) {
    let pair = (Guard { n: 3 }, Guard { n: 4 });
    if take { let moved = pair.0; drop(moved); }
    println!("tail {}", pair.1.n);
}
fn main() { run(true); run(false); }
"#,
            expect: Expect::Run("drop 3\ntail 4\ndrop 4\ntail 4\ndrop 3\ndrop 4\n"),
        }],
    );
}

#[test]
fn memory_direct_borrow_temporary_lifetime_extension_matches_stock() {
    oracle::check(
        "memory-direct-borrow-temporary-lifetime-extension",
        &[Case {
            name: "borrowed_destructor_temporary_lives_until_enclosing_block_end",
            source: r#"struct Guard { n: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.n); } }
fn main() {
    let borrowed = &Guard { n: 7 };
    println!("value {}", borrowed.n);
    println!("end");
}
"#,
            expect: Expect::Run("value 7\nend\ndrop 7\n"),
        }],
    );
}

#[test]
fn memory_unused_call_argument_temporary_escape_is_invalid() {
    oracle::check(
        "memory-unused-call-argument-temporary-escape",
        &[Case {
            name: "function_call_does_not_extend_borrowed_temporary_to_next_statement",
            source: r#"struct Item { n: i32 }
impl Drop for Item { fn drop(&mut self) { println!("drop {}", self.n); } }
fn identity<'a>(value: &'a Item) -> &'a Item { value }
fn unused() {
    let borrowed = identity(&Item { n: 7 });
    println!("{}", borrowed.n);
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0716"),
        }],
    );
}

#[test]
fn memory_unused_drop_check_borrow_escape_is_invalid() {
    oracle::check(
        "memory-unused-drop-check-borrow-escape",
        &[Case {
            name: "destructor_reads_borrow_after_inner_referent_scope_ends",
            source: r#"struct Watch<'a> { value: &'a i32 }
impl<'a> Drop for Watch<'a> {
    fn drop(&mut self) { println!("drop {}", *self.value); }
}
fn unused() {
    let watcher;
    {
        let local = 7i32;
        watcher = Watch { value: &local };
    }
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0597"),
        }],
    );
}

#[test]
fn memory_unused_projected_use_after_move_is_invalid() {
    oracle::check(
        "memory-unused-projected-use-after-move",
        &[Case {
            name: "moving_noncopy_tuple_field_does_not_allow_reusing_that_field",
            source: r#"struct Item { n: i32 }
fn consume(value: Item) { println!("{}", value.n); }
fn unused() {
    let pair = (Item { n: 7 }, Item { n: 9 });
    consume(pair.0);
    consume(pair.0);
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0382"),
        }],
    );
}

// Independent edition and numeric obligations.
#[test]
fn edition2024_tail_expression_temporary_drops_before_local_source() {
    oracle::check(
        "edition2024-tail-expression-temporary-drops-before-local-source",
        &[Case {
            name: "edition2024_tail_temporary_drop_precedes_enclosing_local",
            source: r#"struct Probe { name: &'static str }
impl Drop for Probe { fn drop(&mut self) { println!("drop {}", self.name); } }
impl Probe { fn value(&self) -> u32 { 7u32 } }
fn tail() -> u32 {
    let _local = Probe { name: "local" };
    Probe { name: "temporary" }.value()
}
fn main() { println!("{}", tail()); }
"#,
            expect: Expect::Run("drop temporary\ndrop local\n7\n"),
        }],
    );
}

#[test]
fn integer_mixed_signedness_unused_rejected_source() {
    oracle::check(
        "integer-mixed-signedness-unused-rejected-source",
        &[Case {
            name: "unused_u32_cannot_initialize_i32_without_cast",
            source: r#"fn unused() { let unsigned = 7u32; let _: i32 = unsigned; }
fn main() {}"#,
            expect: Expect::InvalidAfterParse("E0308"),
        }],
    );
}

#[test]
fn integer_float_shift_unused_rejected_source() {
    oracle::check(
        "integer-float-shift-unused-rejected-source",
        &[Case {
            name: "unused_integer_cannot_shift_by_float",
            source: r#"fn unused() { let _ = 7u32 << 1.0f32; }
fn main() {}"#,
            expect: Expect::InvalidAfterParse("E0277"),
        }],
    );
}

#[test]
fn wide_integer_cast_to_bool_unused_rejected_source() {
    oracle::check(
        "wide-integer-cast-to-bool-unused-rejected-source",
        &[Case {
            name: "unused_u128_cannot_cast_to_bool",
            source: r#"fn unused() { let _ = 1u128 as bool; }
fn main() {}"#,
            expect: Expect::InvalidAfterParse("E0054"),
        }],
    );
}

#[test]
fn float_bitwise_unused_rejected_source() {
    oracle::check(
        "float-bitwise-unused-rejected-source",
        &[Case {
            name: "unused_f64_has_no_bitwise_and",
            source: r#"fn unused() { let _ = 1.0f64 & 2.0f64; }
fn main() {}"#,
            expect: Expect::InvalidAfterParse("E0369"),
        }],
    );
}

#[test]
fn float_implicit_narrowing_unused_rejected_source() {
    oracle::check(
        "float-implicit-narrowing-unused-rejected-source",
        &[Case {
            name: "unused_f64_cannot_initialize_f32_without_cast",
            source: r#"fn unused() { let wide = 1.5f64; let _: f32 = wide; }
fn main() {}"#,
            expect: Expect::InvalidAfterParse("E0308"),
        }],
    );
}

// Bounded derive, repetition, promotion and static-initializer obligations.
#[test]
fn derive_copy_clone_generic_tuple_runtime() {
    oracle::check(
        "derive-copy-clone-generic-tuple-runtime",
        &[Case {
            name: "generic_tuple_derives_preserve_copy_and_clone",
            source: r#"#[derive(Copy, Clone)]
struct Pair<T: Copy>(T, T);
fn duplicate<T: Copy>(value: T) -> (T, T) { (value, value) }
fn main() {
    let original = Pair(7i32, 9i32);
    let cloned = original.clone();
    let (left, right) = duplicate(original);
    println!("{} {} {} {}", original.0, cloned.1, left.0, right.1);
}
"#,
            expect: Expect::Run("7 9 7 9\n"),
        }],
    );
}

#[test]
fn derive_copy_noncopy_field_semantic_rejection() {
    oracle::check(
        "derive-copy-noncopy-field-semantic-rejection",
        &[Case {
            name: "unused_derived_copy_cannot_cover_noncopy_field",
            source: r#"#[derive(Clone)]
struct NonCopy(i32);
#[derive(Copy, Clone)]
struct Invalid(NonCopy);
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0204"),
        }],
    );
}

#[test]
fn macro_repetition_tokens_runtime_order() {
    oracle::check(
        "macro-repetition-tokens-runtime-order",
        &[Case {
            name: "comma_separated_expression_repetition_evaluates_once_in_order",
            source: r#"macro_rules! accumulate {
    ($($value:expr),* $(,)?) => {{
        let mut total = 0i32;
        $(total += $value;)*
        total
    }};
}
fn step(value: i32) -> i32 { println!("step {}", value); value }
fn main() {
    println!("empty {}", accumulate!());
    let total = accumulate!(step(2i32), step(3i32), step(5i32),);
    println!("sum {}", total);
}
"#,
            expect: Expect::Run("empty 0\nstep 2\nstep 3\nstep 5\nsum 10\n"),
        }],
    );
}

#[test]
#[ignore = "bounded compatibility obligation"]
fn const_promoted_static_borrow_runtime() {
    oracle::check(
        "const-promoted-static-borrow-runtime",
        &[Case {
            name: "promoted_array_outlives_function_and_combines_with_static",
            source: r#"static OFFSET: i32 = 5i32;
fn promoted() -> &'static [i32; 2] { &[7i32, 9i32] }
fn main() {
    let values = promoted();
    println!("{} {} {}", values[0], values[1], values[0] + OFFSET);
}
"#,
            expect: Expect::Run("7 9 12\n"),
        }],
    );
}

#[test]
fn const_runtime_call_borrow_semantic_rejection() {
    oracle::check(
        "const-runtime-call-borrow-semantic-rejection",
        &[Case {
            name: "unused_const_reference_cannot_promote_nonconst_call",
            source: r#"fn runtime() -> i32 { 7i32 }
const INVALID: &'static i32 = &runtime();
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0015"),
        }],
    );
}

#[test]
fn static_runtime_initializer_semantic_rejection() {
    oracle::check(
        "static-runtime-initializer-semantic-rejection",
        &[Case {
            name: "unused_static_cannot_call_runtime_initializer",
            source: r#"fn runtime() -> i32 { 9i32 }
static INVALID: i32 = runtime();
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0015"),
        }],
    );
}

// Bounded library obligations, authored against bd4d7849d; original fixtures unchanged.
#[test]
fn library_result_question_mark_vec_owned_drop_source() {
    oracle::check(
        "library-result-question-mark-vec-owned-drop-source",
        &[Case {
            name: "question_mark_cleans_owned_vec_on_error_and_success",
            source: r#"struct Guard { n: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.n); } }
fn step(fail: bool) -> Result<i32, i32> {
    if fail { Err(9) } else { Ok(4) }
}
fn run(fail: bool) -> Result<i32, i32> {
    let mut owned: Vec<Guard> = Vec::new();
    owned.push(Guard { n: 7 });
    let value = step(fail)?;
    println!("kept {}", owned.len());
    Ok(value + 1)
}
fn main() {
    match run(true) { Ok(n) => println!("ok {}", n), Err(n) => println!("err {}", n) }
    match run(false) { Ok(n) => println!("ok {}", n), Err(n) => println!("err {}", n) }
}
"#,
            expect: Expect::Run("drop 7\nerr 9\nkept 1\ndrop 7\nok 5\n"),
        }],
    );
}

#[test]
fn library_result_unused_return_error_mismatch_source() {
    oracle::check(
        "library-result-unused-return-error-mismatch-source",
        &[Case {
            name: "unused_result_error_payload_must_match_declared_error",
            source: r#"fn unused() -> Result<i32, bool> { Err(7i32) }
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0308"),
        }],
    );
}

#[test]
fn library_iterator_collect_typed_values_source() {
    oracle::check(
        "library-iterator-collect-typed-values-source",
        &[Case {
            name: "mapped_i64_values_collect_into_owned_vector_in_order",
            source: r#"fn main() {
    let values: Vec<i64> = (1i32..4i32).map(|n: i32| (n as i64) * 3i64).collect();
    println!("{} {} {} {}", values.len(), values[0], values[1], values[2]);
}
"#,
            expect: Expect::Run("3 3 6 9\n"),
        }],
    );
}

#[test]
fn library_iterator_unused_collect_wrong_type_source() {
    oracle::check(
        "library-iterator-unused-collect-wrong-type-source",
        &[Case {
            name: "unused_i32_iterator_cannot_collect_into_bool_vector",
            source: r#"fn unused() { let _: Vec<bool> = (0i32..3i32).collect(); }
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0277"),
        }],
    );
}

#[test]
fn library_collection_btree_ordered_shared_borrow_source() {
    oracle::check(
        "library-collection-btree-ordered-shared-borrow-source",
        &[Case {
            name: "btree_noncopy_values_are_read_through_shared_ordered_iterator",
            source: r#"use std::collections::BTreeMap;
struct Item { n: i64 }
fn main() {
    let mut map: BTreeMap<i32, Item> = BTreeMap::new();
    map.insert(3, Item { n: 30 });
    map.insert(1, Item { n: 10 });
    map.insert(2, Item { n: 20 });
    let shared = &map;
    for (key, value) in shared.iter() { println!("{} {}", key, value.n); }
    println!("len {}", map.len());
}
"#,
            expect: Expect::Run("1 10\n2 20\n3 30\nlen 3\n"),
        }],
    );
}

#[test]
fn library_collection_unused_moved_map_value_source() {
    oracle::check(
        "library-collection-unused-moved-map-value-source",
        &[Case {
            name: "unused_noncopy_payload_cannot_be_reused_after_map_insert",
            source: r#"use std::collections::BTreeMap;
struct Item { n: i32 }
fn unused() {
    let value = Item { n: 7 };
    let mut map: BTreeMap<i32, Item> = BTreeMap::new();
    map.insert(1, value);
    let _again = value;
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0382"),
        }],
    );
}

// Exact original-source token-rule obligations. Each ordinary regression keeps
// its pinned stock oracle and strict native phase evidence.
#[test]
fn token_suffix_literal_and_tt_macro_consumption_source() {
    oracle::check(
        "token-suffix-literal-and-tt-macro-consumption-source",
        &[Case {
            name: "arbitrary_literal_suffixes_are_tokens_until_interpreted",
            source: r####"macro_rules! discard_literal { ($value:literal) => { 17i32 }; }
macro_rules! discard_tree { ($value:tt) => { 19i32 }; }
fn main() {
    let first = discard_literal!("ignored"custom);
    let second = discard_literal!(27custom);
    let third = discard_tree!(b'Q'custom);
    println!("{} {} {}", first, second, third);
}
"####,
            expect: Expect::Run("17 17 19\n"),
        }],
    );
}

#[test]
fn token_raw_cooked_hash_escape_observable_source() {
    oracle::check(
        "token-raw-cooked-hash-escape-observable-source",
        &[Case {
            name: "raw_hash_boundaries_preserve_backslashes_and_cooked_escapes_decode",
            source: r####"fn main() {
    let raw = r##"left"#\n"right"##;
    let cooked = "left\"#\\n\"right";
    println!("{}", raw);
    println!("{}", cooked);
    let byte: u8 = b'\x7f';
    let letter: char = '\u{41}';
    println!("{} {}", byte, letter);
}
"####,
            expect: Expect::Run("left\"#\\n\"right\nleft\"#\\n\"right\n127 A\n"),
        }],
    );
}

#[test]
fn token_numeric_spelling_radix_tuple_observable_source() {
    oracle::check(
        "token-numeric-spelling-radix-tuple-observable-source",
        &[Case {
            name: "hex_digits_do_not_become_float_suffixes_or_exponents",
            source: r####"fn main() {
    let first: i32 = 0x01_f32;
    let second: i32 = 0x01_e3;
    let binary: i32 = 0b____1_0;
    let octal: i32 = 0o7_0;
    let floating: f64 = 1_2.5e+1;
    let pair = (first, second);
    println!("{} {} {} {} {} {}", pair.0, pair.1, binary, octal, floating, 00042i32);
}
"####,
            expect: Expect::Run("7986 483 2 56 125 42\n"),
        }],
    );
}

#[test]
fn token_unused_forwarded_float_suffix_type_error_source() {
    oracle::check(
        "token-unused-forwarded-float-suffix-type-error-source",
        &[Case {
            name: "valid_float_suffix_forwarded_from_literal_macro_is_not_u8",
            source: r####"macro_rules! literal_value { ($value:literal) => { $value }; }
fn unused() { let _: u8 = literal_value!(2f32); }
fn main() {}
"####,
            expect: Expect::InvalidAfterParse("E0308"),
        }],
    );
}

#[test]
fn token_unused_valid_raw_string_type_error_source() {
    oracle::check(
        "token-unused-valid-raw-string-type-error-source",
        &[Case {
            name: "valid_raw_delimiter_contents_do_not_become_a_boolean",
            source: r####"fn unused() { let _: bool = r##"left"#\n"right"##; }
fn main() {}
"####,
            expect: Expect::InvalidAfterParse("E0308"),
        }],
    );
}

#[test]
fn token_unused_tuple_leading_zero_field_error_source() {
    oracle::check(
        "token-unused-tuple-leading-zero-field-error-source",
        &[Case {
            name: "tuple_field_uses_literal_spelling_not_integer_value",
            source: r####"fn unused() {
    let pair = (11i32, 29i32);
    let _ = pair.01;
}
fn main() {}
"####,
            expect: Expect::InvalidAfterParse("E0609"),
        }],
    );
}

// Native staging regression, deliberately separate from the unchanged pinned
// stock obligation above. Passing this test alone is not stock acceptance.
#[test]
fn tuple_noncanonical_field_retains_semantic_source_facts() {
    use justrust_engine::compiler;
    for field in [
        "00",
        "01",
        "0001",
        "000000000000000000000000000000000000001",
        "0.01",
    ] {
        let pair = if field == "0.01" {
            "((11i32,29i32),37i32)"
        } else {
            "(11i32,29i32)"
        };
        let source = format!("fn unused(){{let pair={pair};let _=pair.{field};}}fn main(){{}}");
        let invalid = field.rsplit('.').next().unwrap();
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.clone(), workers, check)
                    .err()
                    .expect("noncanonical spelling cannot select a tuple field");
                oracle::validate_semantic_rejection(&failure).unwrap();
                assert!(
                    failure
                        .error
                        .0
                        .contains(&format!("unknown tuple field `{invalid}`")),
                    "{}",
                    failure.error.0
                );
                let report = failure.report.as_ref().unwrap();
                if check {
                    assert!(
                        report
                            .node_evaluations
                            .iter()
                            .all(|(key, _)| ![103, 105, 130, 131, 133].contains(&key.kind))
                    );
                }
                assert!(
                    report
                        .node_ids
                        .iter()
                        .zip(&report.node_evaluations)
                        .filter(|(_, (key, _))| key.kind == 105)
                        .all(|(node, _)| !report
                            .events
                            .iter()
                            .any(|event| event.kind == "publish" && event.node == Some(*node)))
                );
            }
        }
    }
    // The canonical selectors remain successful in CHECK and BUILD. The
    // existing differential tuple corpus exercises their executable behavior.
    for workers in [1, 8] {
        for check in [true, false] {
            compiler::compile_observed(
                "fn main(){let pair=(11i32,29i32);println!(\"{} {}\",pair.0,pair.1);}".into(),
                workers,
                check,
            )
            .unwrap();
        }
    }
}

// Independent exact original-source library regressions. These bounded
// ownership and cleanup witnesses do not establish library completeness.
#[test]
fn library_option_replace_take_nested_owned_drop_source() {
    oracle::check(
        "library-option-replace-take-nested-owned-drop-source",
        &[Case {
            name: "replace_returns_old_nested_owned_value_and_take_empties_slot",
            source: r#"struct Guard { n: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.n); } }
fn main() {
    let mut slot: Option<Result<Guard, i32>> = Some(Ok(Guard { n: 7 }));
    let previous = slot.replace(Err(9));
    println!("replaced");
    drop(previous);
    match slot.take() {
        Some(Err(code)) => println!("error {}", code),
        _ => println!("unexpected"),
    }
    println!("empty {}", slot.is_none());
    drop(slot.replace(Ok(Guard { n: 11 })));
    println!("restored");
    drop(slot);
}
"#,
            expect: Expect::Run("replaced\ndrop 7\nerror 9\nempty true\nrestored\ndrop 11\n"),
        }],
    );
}

#[test]
fn library_option_unused_replace_nested_payload_mismatch_source() {
    oracle::check(
        "library-option-unused-replace-nested-payload-mismatch-source",
        &[Case {
            name: "unused_replace_cannot_change_nested_owned_success_payload_type",
            source: r#"struct Guard { n: i32 }
fn unused() {
    let mut slot: Option<Result<Guard, i32>> = None;
    let _ = slot.replace(Ok(false));
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0308"),
        }],
    );
}

#[test]
fn library_vec_swap_remove_truncate_owned_drop_source() {
    oracle::check(
        "library-vec-swap-remove-truncate-owned-drop-source",
        &[Case {
            name: "swap_remove_moves_last_payload_and_truncate_drops_only_discarded_tail",
            source: r#"struct Guard { n: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.n); } }
fn main() {
    let mut values: Vec<Guard> = Vec::new();
    values.push(Guard { n: 1 });
    values.push(Guard { n: 2 });
    values.push(Guard { n: 3 });
    let removed = values.swap_remove(0usize);
    println!("removed {} remaining {} {}", removed.n, values[0].n, values[1].n);
    drop(removed);
    values.truncate(1usize);
    println!("kept {} len {}", values[0].n, values.len());
    drop(values);
}
"#,
            expect: Expect::Run("removed 1 remaining 3 2\ndrop 1\ndrop 2\nkept 3 len 1\ndrop 3\n"),
        }],
    );
}

#[test]
fn library_vec_unused_truncate_live_element_borrow_source() {
    oracle::check(
        "library-vec-unused-truncate-live-element-borrow-source",
        &[Case {
            name: "unused_truncate_cannot_invalidate_element_borrow_read_later",
            source: r#"fn unused() {
    let mut values: Vec<i32> = Vec::new();
    values.push(7i32);
    let borrowed = &values[0];
    values.truncate(0usize);
    println!("{}", *borrowed);
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0502"),
        }],
    );
}

#[test]
fn library_iterator_nth_owned_skipped_drop_source() {
    oracle::check(
        "library-iterator-nth-owned-skipped-drop-source",
        &[Case {
            name: "nth_consumes_skipped_owned_item_and_returns_selected_item_once",
            source: r#"struct Guard { n: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.n); } }
fn main() {
    let mut values: Vec<Guard> = Vec::new();
    values.push(Guard { n: 1 });
    values.push(Guard { n: 2 });
    values.push(Guard { n: 3 });
    let mut iterator = values.into_iter();
    match iterator.nth(1usize) {
        Some(value) => { println!("selected {}", value.n); drop(value); }
        None => println!("unexpected"),
    }
    match iterator.next() {
        Some(value) => { println!("tail {}", value.n); drop(value); }
        None => println!("unexpected"),
    }
    println!("empty {}", iterator.nth(0usize).is_none());
}
"#,
            expect: Expect::Run("drop 1\nselected 2\ndrop 2\ntail 3\ndrop 3\nempty true\n"),
        }],
    );
}

#[test]
fn library_iterator_unused_nth_non_usize_index_source() {
    oracle::check(
        "library-iterator-unused-nth-non-usize-index-source",
        &[Case {
            name: "unused_nth_requires_usize_index_not_bool",
            source: r#"fn unused() {
    let mut values: Vec<i32> = Vec::new();
    values.push(7i32);
    let mut iterator = values.into_iter();
    let _ = iterator.nth(true);
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0308"),
        }],
    );
}

// Bounded projected-place obligations, pending exact-source stock/native review.
#[test]
fn memory_projected_nested_disjoint_mutable_fields_source() {
    oracle::check(
        "memory-projected-nested-disjoint-mutable-fields-source",
        &[Case {
            name: "nested_named_fields_keep_disjoint_mutable_loans_live",
            source: r#"struct Inner { left: i32, right: i32 }
struct Outer { inner: Inner, spare: i32 }
fn main() {
    let mut value = Outer { inner: Inner { left: 3, right: 8 }, spare: 17 };
    let left = &mut value.inner.left;
    let right = &mut value.inner.right;
    *left += 4;
    *right += *left;
    println!("loans {} {}", *left, *right);
    value.inner.left += 1;
    value.inner.right += 2;
    println!("fields {} {} {}", value.inner.left, value.inner.right, value.spare);
}
"#,
            expect: Expect::Run("loans 7 15\nfields 8 17 17\n"),
        }],
    );
}

#[test]
#[ignore = "pending native projected-loan compatibility obligation"]
fn memory_projected_generic_short_reborrow_source() {
    oracle::check(
        "memory-projected-generic-short-reborrow-source",
        &[Case {
            name: "generic_field_short_reborrow_ends_before_original_handle_reuse",
            source: r#"struct Slot<T> { value: T, tag: i32 }
fn field<T>(slot: &mut Slot<T>) -> &mut T { &mut slot.value }
fn replace_twice<T>(slot: &mut Slot<T>, first: T, second: T) {
    let field = field(slot);
    {
        let short = &mut *field;
        *short = first;
    }
    *field = second;
}
fn main() {
    let mut slot = Slot { value: 3i32, tag: 19 };
    replace_twice(&mut slot, 7i32, 11i32);
    println!("{} {}", slot.value, slot.tag);
}
"#,
            expect: Expect::Run("11 19\n"),
        }],
    );
}

#[test]
fn memory_projected_tuple_explicit_borrows_closure_source() {
    oracle::check(
        "memory-projected-tuple-explicit-borrows-closure-source",
        &[Case {
            name: "nonmove_closure_uses_explicit_shared_and_mutable_disjoint_tuple_loans",
            source: r#"fn main() {
    let mut values = (5i32, 9i32, 23i32);
    let shared = &values.0;
    let changed = &mut values.1;
    {
        let mut apply = || {
            println!("before {} {}", *shared, *changed);
            *changed += *shared;
            println!("after {} {}", *shared, *changed);
        };
        apply();
    }
    println!("values {} {} {}", values.0, values.1, values.2);
}
"#,
            expect: Expect::Run("before 5 9\nafter 5 14\nvalues 5 14 23\n"),
        }],
    );
}

#[test]
fn memory_projected_unused_nested_double_mutable_source() {
    oracle::check(
        "memory-projected-unused-nested-double-mutable-source",
        &[Case {
            name: "unused_nested_same_field_mutable_loans_both_remain_used",
            source: r#"struct Inner { left: i32, right: i32 }
struct Outer { inner: Inner }
fn unused() {
    let mut value = Outer { inner: Inner { left: 3, right: 8 } };
    let first = &mut value.inner.left;
    let second = &mut value.inner.left;
    *first += 1;
    *second += 2;
    println!("{} {} {}", *first, *second, value.inner.right);
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0499"),
        }],
    );
}

#[test]
fn memory_projected_unused_shared_then_assign_source() {
    oracle::check(
        "memory-projected-unused-shared-then-assign-source",
        &[Case {
            name: "unused_direct_projected_assignment_conflicts_with_later_shared_read",
            source: r#"struct Pair { left: i32, right: i32 }
fn unused() {
    let mut pair = Pair { left: 5, right: 9 };
    let shared = &pair.left;
    pair.right += 1;
    pair.left = 12;
    println!("{} {}", *shared, pair.right);
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0506"),
        }],
    );
}

#[test]
fn memory_projected_unused_string_field_reuse_source() {
    oracle::check(
        "memory-projected-unused-string-field-reuse-source",
        &[Case {
            name: "unused_owned_string_field_move_keeps_sibling_usable_not_moved_field",
            source: r#"struct Parcel { text: String, number: i32 }
fn unused() {
    let parcel = Parcel { text: String::from("cargo"), number: 31 };
    let moved = parcel.text;
    println!("{} {}", moved, parcel.number);
    let reused = &parcel.text;
    println!("{} {}", reused, parcel.number);
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0382"),
        }],
    );
}

// Additional strict semantic obligations remain pending until their original
// sources pass the independent stock/native oracle, including unused bodies.
#[test]
fn region_unused_local_reference_cannot_be_static_source() {
    oracle::check(
        "region-unused-local-reference-cannot-be-static-source",
        &[Case {
            name: "unused_local_reference_cannot_acquire_static_authority",
            source: r#"fn unused() -> &'static i32 {
    let local = 17i32;
    &local
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0515"),
        }],
    );
}

#[test]
fn drop_unused_explicit_destructor_call_is_rejected_source() {
    oracle::check(
        "drop-unused-explicit-destructor-call-is-rejected-source",
        &[Case {
            name: "unused_body_cannot_call_drop_trait_method_explicitly",
            source: r#"struct Guard { value: i32 }
impl Drop for Guard {
    fn drop(&mut self) { println!("drop {}", self.value); }
}
fn unused() {
    let mut guard = Guard { value: 7 };
    guard.drop();
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0040"),
        }],
    );
}

#[test]
fn layout_unused_non_power_of_two_alignment_is_rejected_source() {
    oracle::check(
        "layout-unused-non-power-of-two-alignment-is-rejected-source",
        &[Case {
            name: "unused_record_alignment_must_be_power_of_two",
            source: r#"#[repr(align(3))]
struct Record { value: u8 }
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0589"),
        }],
    );
}

#[test]
fn module_unused_duplicate_inline_module_is_rejected_source() {
    oracle::check(
        "module-unused-duplicate-inline-module-is-rejected-source",
        &[Case {
            name: "unused_owner_cannot_redeclare_inline_module_in_same_namespace",
            source: r#"mod owner {
    mod duplicate { pub fn value() -> i32 { 7 } }
    mod duplicate { pub fn value() -> i32 { 11 } }
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0428"),
        }],
    );
}

#[test]
fn module_duplicate_inline_headers_reject_after_declarations_pinned_stock() {
    oracle::check(
        "module-duplicate-inline-headers-after-declarations",
        &[
            Case {
                name: "unused_empty_duplicate_modules",
                source: "mod owner { mod duplicate {} mod duplicate {} } fn main() {}",
                expect: Expect::InvalidAfterParse("E0428"),
            },
            Case {
                name: "root_duplicate_with_same_named_bodies",
                source: "mod duplicate { pub fn value()->i32 {7} } mod duplicate { pub fn value()->i32 {11} } fn main() {}",
                expect: Expect::InvalidAfterParse("E0428"),
            },
            Case {
                name: "duplicate_with_disjoint_bodies",
                source: "mod owner { mod duplicate { pub fn first() {} } mod duplicate { pub fn second() {} } } fn main() {}",
                expect: Expect::InvalidAfterParse("E0428"),
            },
            Case {
                name: "deep_unused_duplicate_modules",
                source: "mod a { mod b { mod duplicate {} mod duplicate {} } } fn main() {}",
                expect: Expect::InvalidAfterParse("E0428"),
            },
            Case {
                name: "three_unused_duplicate_modules",
                source: "mod owner { mod duplicate {} mod duplicate {} mod duplicate {} } fn main() {}",
                expect: Expect::InvalidAfterParse("E0428"),
            },
        ],
    );
}

#[test]
fn module_distinct_inline_headers_preserve_scopes_pinned_stock() {
    oracle::check(
        "module-distinct-inline-headers-preserve-scopes",
        &[
            Case {
                name: "same_child_spelling_in_different_parents",
                source: "mod a { pub mod same { pub fn value()->i32 {7} } } mod b { pub mod same { pub fn value()->i32 {11} } } fn main() { println!(\"{} {}\", a::same::value(), b::same::value()); }",
                expect: Expect::Run("7 11\n"),
            },
            Case {
                name: "distinct_children_in_one_unused_parent",
                source: "mod owner { mod first {} mod second {} } fn main() {}",
                expect: Expect::Run(""),
            },
            Case {
                name: "module_and_function_are_distinct_namespaces",
                source: "mod same { pub fn value()->i32 {7} } fn same()->i32 {11} fn main() { println!(\"{} {}\", same::value(), same()); }",
                expect: Expect::Run("7 11\n"),
            },
            Case {
                name: "same_child_spelling_at_different_depths",
                source: "mod same { pub mod same { pub fn value()->i32 {7} } } fn main() { println!(\"{}\", same::same::value()); }",
                expect: Expect::Run("7\n"),
            },
        ],
    );
}

// Append only to tests/conformance_generated.rs at frozen 21c1b58c5.
// Authored expectations, not execution evidence. Each selector is independent.
#[test]
fn interaction_generic_owned_option_projected_shared_source() {
    oracle::check(
        "interaction-generic-owned-option-projected-shared",
        &[Case {
            name: "generic_owned_tuple_shared_projection_then_option_branch_drop",
            source: r#"struct Guard { n: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.n); } }
fn identity<T>(value: T) -> T { value }
fn run(take: bool) {
    let pair = identity::<(Guard, i32)>((Guard { n: 7 }, 11));
    let borrowed = &pair.0.n;
    println!("borrow {} {}", *borrowed, pair.1);
    let chosen = if take { Some(pair.0) } else { None };
    match chosen {
        Some(value) => { println!("some {}", value.n); drop(value); }
        None => println!("none"),
    }
    println!("tail {}", pair.1);
}
fn main() { run(true); run(false); }
"#,
            expect: Expect::Run(
                "borrow 7 11\nsome 7\ndrop 7\ntail 11\nborrow 7 11\nnone\ntail 11\ndrop 7\n",
            ),
        }],
    );
}

#[test]
fn interaction_generic_projected_owned_replacement_source() {
    oracle::check(
        "interaction-generic-projected-owned-replacement",
        &[Case {
            name: "generic_mutable_tuple_projection_replacement_drops_old_value_in_each_arm",
            source: r#"struct Guard { n: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.n); } }
fn identity<T>(value: T) -> T { value }
fn replace<T>(place: &mut T, value: T) { *place = value; }
fn run(left: bool) {
    let mut pair = identity::<(Guard, i32)>((Guard { n: 3 }, 19));
    let slot = &mut pair.0;
    match left {
        true => replace::<Guard>(slot, Guard { n: 5 }),
        false => replace::<Guard>(slot, Guard { n: 8 }),
    }
    println!("after {} {}", pair.0.n, pair.1);
}
fn main() { run(true); run(false); }
"#,
            expect: Expect::Run("drop 3\nafter 5 19\ndrop 5\ndrop 3\nafter 8 19\ndrop 8\n"),
        }],
    );
}

#[test]
#[ignore = "pending strict generic/projected/drop interaction acceptance"]
fn interaction_generic_fnonce_projected_borrow_early_return_source() {
    oracle::check(
        "interaction-generic-fnonce-projected-borrow-early-return",
        &[Case {
            name: "generic_fnonce_moves_guard_with_shared_projection_and_two_return_paths",
            source: r#"struct Guard { n: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.n); } }
fn identity<T>(value: T) -> T { value }
fn apply<F: FnOnce() -> i32>(f: F) -> i32 { f() }
fn run(early: bool) {
    let pair = identity::<(i32, i32)>((4, 23));
    let projected = &pair.0;
    let guard = Guard { n: 9 };
    let call = move || {
        if early { drop(guard); return *projected; }
        let answer = *projected + guard.n;
        drop(guard);
        answer
    };
    println!("answer {}", apply(call));
    println!("sibling {}", pair.1);
}
fn main() { run(true); run(false); }
"#,
            expect: Expect::Run("drop 9\nanswer 4\nsibling 23\ndrop 9\nanswer 13\nsibling 23\n"),
        }],
    );
}

#[test]
fn interaction_unused_generic_projected_replacement_type_source() {
    oracle::check(
        "interaction-unused-generic-projected-replacement-type",
        &[Case {
            name: "unused_explicit_generic_replacement_rejects_wrong_owned_argument",
            source: r#"struct Guard { n: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.n); } }
fn replace<T>(place: &mut T, value: T) { *place = value; }
fn unused(left: bool) {
    let mut pair = (Guard { n: 3 }, 19i32);
    let slot = &mut pair.0;
    if left { replace::<Guard>(slot, 5i32); }
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0308"),
        }],
    );
}

#[test]
fn interaction_unused_generic_owned_move_live_projection_source() {
    oracle::check(
        "interaction-unused-generic-owned-move-live-projection",
        &[Case {
            name: "unused_generic_owned_tuple_move_conflicts_with_later_projected_read",
            source: r#"struct Guard { n: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.n); } }
fn identity<T>(value: T) -> T { value }
fn unused(take: bool) {
    let pair = identity::<(Guard, i32)>((Guard { n: 7 }, 11));
    let borrowed = &pair.0.n;
    if take { drop(pair.0); }
    println!("{}", *borrowed);
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0505"),
        }],
    );
}

#[test]
fn interaction_unused_generic_fnonce_conditional_double_move_source() {
    oracle::check(
        "interaction-unused-generic-fnonce-conditional-double-move",
        &[Case {
            name: "unused_fnonce_conditional_guard_move_cannot_reach_second_drop",
            source: r#"struct Guard { n: i32 }
impl Drop for Guard { fn drop(&mut self) { println!("drop {}", self.n); } }
fn apply<F: FnOnce() -> i32>(f: F) -> i32 { f() }
fn unused(early: bool) {
    let pair = (4i32, 23i32);
    let projected = &pair.0;
    let guard = Guard { n: 9 };
    let call = move || {
        if early { drop(guard); }
        drop(guard);
        *projected
    };
    let _ = apply(call);
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0382"),
        }],
    );
}

// Scratch-authored append at b3b08baa6. Bounded campaigns, not exhaustive Rust,
// sustained fuzzing, or failure minimization. Existing fixtures stay unchanged.
mod bounded_seed_campaign {
    use super::{Case, Expect, oracle};

    // Literal source identity, independent of environment, scheduling and order.
    const SEEDS: [u64; 4] = [0x10, 0x21, 0x32, 0x43];
    const FAMILIES: [&str; 6] = [
        "owned_option_partial_drop",
        "scalar_projected_replace",
        "closure_result_option",
        "unused_live_projection_assignment",
        "unused_generic_wrong_argument",
        "unused_conditional_noncopy_move",
    ];

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Outcome {
        Run(String),
        InvalidAfterParse(&'static str),
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Generated {
        source: String,
        expected: Outcome,
    }

    // Finite grammar: each seed selects if vs match and true/false call order,
    // not just constants. Every valid program calls both paths exactly once.
    // Three scalar operands in 1..=13, only addition (at most 26), two calls,
    // no recursion/loops/unsafe/heap input. Invalid bodies remain uncalled and
    // retain a live loan, explicit type mismatch, or conditional owned move.
    fn values(seed: u64) -> (i32, i32, i32) {
        (
            (seed % 13 + 1) as i32,
            (seed.rotate_left(7) % 13 + 1) as i32,
            (seed.rotate_left(11) % 13 + 1) as i32,
        )
    }
    fn branch(seed: u64, yes: &str, no: &str) -> String {
        if seed & 1 == 0 {
            format!("if flag {{ {yes} }} else {{ {no} }}")
        } else {
            format!("match flag {{ true => {{ {yes} }}, false => {{ {no} }} }}")
        }
    }
    fn order(seed: u64) -> [bool; 2] {
        if seed & 2 == 0 {
            [true, false]
        } else {
            [false, true]
        }
    }
    fn generate(family: usize, seed: u64) -> Generated {
        assert!(family < FAMILIES.len(), "strict family guard");
        let (a, b, c) = values(seed);
        let flags = order(seed);
        let calls = format!("fn main() {{ run({}); run({}); }}\n", flags[0], flags[1]);
        let identity = "fn identity<T>(value: T) -> T { value }\n";
        let guard = "struct Guard { n: i32 }\nimpl Drop for Guard { fn drop(&mut self) { println!(\"drop {}\", self.n); } }\n";
        match family {
            0 => {
                let choose = branch(seed, "Some(pair.0)", "None");
                let source = format!(
                    "{guard}{identity}fn run(flag: bool) {{\nlet pair = identity::<(Guard, i32)>((Guard {{ n: {a} }}, {b}));\nlet borrowed = &pair.0.n; println!(\"borrow {{}} {{}}\", *borrowed, pair.1);\nlet chosen = {choose};\nmatch chosen {{ Some(value) => {{ println!(\"some {{}}\", value.n); drop(value); }}, None => println!(\"none\"), }}\nprintln!(\"tail {{}}\", pair.1);\n}}\n{calls}"
                );
                let mut stdout = String::new();
                for flag in flags {
                    stdout.push_str(&format!("borrow {a} {b}\n"));
                    if flag {
                        stdout.push_str(&format!("some {a}\ndrop {a}\ntail {b}\n"));
                    } else {
                        stdout.push_str(&format!("none\ntail {b}\ndrop {a}\n"));
                    }
                }
                Generated {
                    source,
                    expected: Outcome::Run(stdout),
                }
            }
            1 => {
                let choose = branch(
                    seed,
                    &format!("replace::<i32>(slot, {b});"),
                    &format!("replace::<i32>(slot, {c});"),
                );
                let source = format!(
                    "{identity}fn replace<T>(place: &mut T, value: T) {{ *place = value; }}\nfn run(flag: bool) {{ let mut pair = identity::<(i32, i32)>(({a}, {a})); let slot = &mut pair.0; {choose}; println!(\"after {{}} {{}}\", pair.0, pair.1); }}\n{calls}"
                );
                let stdout = flags
                    .iter()
                    .map(|flag| format!("after {} {a}\n", if *flag { b } else { c }))
                    .collect::<String>();
                Generated {
                    source,
                    expected: Outcome::Run(stdout),
                }
            }
            2 => {
                let choose = branch(
                    seed,
                    &format!("Ok::<i32, i32>({a} + {b})"),
                    &format!("Err::<i32, i32>({c})"),
                );
                let source = format!(
                    "fn run(flag: bool) {{ let call = || {{ {choose} }}; let value = match call() {{ Ok(n) => Some(n), Err(_) => None }}; match value {{ Some(n) => println!(\"some {{}}\", n), None => println!(\"none\"), }} }}\n{calls}"
                );
                let stdout = flags
                    .iter()
                    .map(|flag| {
                        if *flag {
                            format!("some {}\n", a + b)
                        } else {
                            "none\n".into()
                        }
                    })
                    .collect::<String>();
                Generated {
                    source,
                    expected: Outcome::Run(stdout),
                }
            }
            3 => {
                let choose = branch(seed, &format!("pair.0 = {b};"), &format!("pair.0 = {c};"));
                Generated {
                    source: format!(
                        "fn unused(flag: bool) {{ let mut pair = ({a}i32, {b}i32); let borrowed = &pair.0; {choose}; println!(\"{{}}\", *borrowed); }}\nfn main() {{}}\n"
                    ),
                    expected: Outcome::InvalidAfterParse("E0506"),
                }
            }
            4 => {
                let choose = branch(
                    seed,
                    &format!("replace::<i32>(slot, {b}i32);"),
                    "replace::<i32>(slot, false);",
                );
                Generated {
                    source: format!(
                        "fn replace<T>(place: &mut T, value: T) {{ *place = value; }}\nfn unused(flag: bool) {{ let mut pair = ({a}i32, {c}i32); let slot = &mut pair.0; {choose}; }}\nfn main() {{}}\n"
                    ),
                    expected: Outcome::InvalidAfterParse("E0308"),
                }
            }
            5 => {
                let choose = branch(
                    seed,
                    "drop(value);",
                    &format!("println!(\"keep {{}}\", {b}i32 + {c}i32);"),
                );
                Generated {
                    source: format!(
                        "{guard}{identity}fn unused(flag: bool) {{ let value = identity::<Guard>(Guard {{ n: {a} }}); {choose}; drop(value); }}\nfn main() {{}}\n"
                    ),
                    expected: Outcome::InvalidAfterParse("E0382"),
                }
            }
            _ => unreachable!(),
        }
    }

    // One attempt per seed, including after an earlier panic. No retries and no
    // success masking. Generic harness permits pure panic-aggregation testing.
    fn attempt_all<F>(family: usize, mut attempt: F) -> Vec<String>
    where
        F: FnMut(u64, &Generated),
    {
        let mut failures = Vec::new();
        for seed in SEEDS {
            let mut reproduction = format!("grammar={} seed={seed:#018x}", FAMILIES[family]);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let generated = generate(family, seed);
                reproduction.push_str(&format!(
                    "\nexpected={:?}\nsource:\n{}",
                    generated.expected, generated.source
                ));
                eprintln!("{reproduction}");
                attempt(seed, &generated);
            }));
            if let Err(payload) = result {
                let detail = payload
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| payload.downcast_ref::<&str>().copied())
                    .unwrap_or("non-string panic");
                failures.push(format!("{reproduction}\noracle panic: {detail}"));
            }
        }
        failures
    }
    pub(super) fn campaign(family: usize) {
        let failures = attempt_all(family, |seed, generated| {
            let tag = format!("bounded-seed-{}-{seed:016x}", FAMILIES[family]);
            // closure_oracle requires process-lifetime strings. At most four
            // cases per literal selector. No weakening of pinned stock/native
            // CHECK/BUILD/run or worker 1/8 and invalid graph-evidence gates.
            let source = Box::leak(generated.source.clone().into_boxed_str());
            let name = Box::leak(tag.clone().into_boxed_str());
            let expect = match &generated.expected {
                Outcome::Run(stdout) => Expect::Run(Box::leak(stdout.clone().into_boxed_str())),
                Outcome::InvalidAfterParse(code) => Expect::InvalidAfterParse(code),
            };
            oracle::check(
                &tag,
                &[Case {
                    name,
                    source,
                    expect,
                }],
            );
        });
        assert!(
            failures.is_empty(),
            "{} of four seeds failed, all four attempted, no retry:\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }

    pub(super) fn deterministic_bytes_independent_order_and_finite_operands() {
        for family in 0..FAMILIES.len() {
            let forward: Vec<_> = SEEDS.iter().map(|&seed| generate(family, seed)).collect();
            for index in (0..4).rev() {
                // Interleave another family to expose accidental shared state.
                let _ = generate((family + 1) % FAMILIES.len(), SEEDS[(index + 1) % 4]);
                assert_eq!(forward[index], generate(family, SEEDS[index]));
                for value in [
                    values(SEEDS[index]).0,
                    values(SEEDS[index]).1,
                    values(SEEDS[index]).2,
                ] {
                    assert!((1..=13).contains(&value));
                }
            }
            assert!(forward[0].source.contains("if flag"));
            assert!(forward[1].source.contains("match flag"));
            assert_ne!(forward[0].source, forward[1].source);
            assert_eq!(order(SEEDS[0]), [true, false]);
            assert_eq!(order(SEEDS[2]), [false, true]);
        }
        for seed in [0, 1, 13, u64::MAX, 1 << 63] {
            let (a, b, c) = values(seed);
            assert!([a, b, c].iter().all(|n| (1..=13).contains(n)));
        }
        assert!(std::panic::catch_unwind(|| generate(6, SEEDS[0])).is_err());
    }

    pub(super) fn expected_output_and_strict_invalid_contract() {
        // Independently authored goldens for seed 0x10: (a,b,c) = (4,8,9).
        assert_eq!(
            SEEDS.map(values),
            [(4, 8, 9), (8, 13, 11), (12, 5, 13), (3, 10, 2)]
        );
        assert_eq!(
            generate(0, 0x10).expected,
            Outcome::Run(
                "borrow 4 8\nsome 4\ndrop 4\ntail 8\nborrow 4 8\nnone\ntail 8\ndrop 4\n".into()
            )
        );
        assert_eq!(
            generate(1, 0x10).expected,
            Outcome::Run("after 8 4\nafter 9 4\n".into())
        );
        assert_eq!(
            generate(2, 0x10).expected,
            Outcome::Run("some 12\nnone\n".into())
        );
        for (index, expected) in [
            "after 8 4\nafter 9 4\n",
            "after 13 8\nafter 11 8\n",
            "after 13 12\nafter 5 12\n",
            "after 2 3\nafter 10 3\n",
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(
                generate(1, SEEDS[index]).expected,
                Outcome::Run((*expected).into())
            );
            let (_, b, c) = values(SEEDS[index]);
            assert_ne!(b, c, "replacement paths must remain observable");
        }
        for (index, expected) in [
            "some 12\nnone\n",
            "some 21\nnone\n",
            "none\nsome 17\n",
            "none\nsome 13\n",
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(
                generate(2, SEEDS[index]).expected,
                Outcome::Run((*expected).into())
            );
        }
        for seed in SEEDS {
            for (family, code, marker) in [
                (3, "E0506", "*borrowed"),
                (4, "E0308", "replace::<i32>(slot, false)"),
                (5, "E0382", "drop(value)"),
            ] {
                let generated = generate(family, seed);
                assert_eq!(generated.expected, Outcome::InvalidAfterParse(code));
                assert!(generated.source.contains("fn unused(flag: bool)"));
                assert!(generated.source.ends_with("fn main() {}\n"));
                assert!(generated.source.contains(marker));
                assert!(!generated.source.contains("unused(true)"));
            }
        }
    }

    pub(super) fn panic_aggregation_attempts_all_four_without_retry() {
        let mut visited = Vec::new();
        let failures = attempt_all(0, |seed, _| {
            visited.push(seed);
            panic!("synthetic harness failure {seed}");
        });
        assert_eq!(visited, SEEDS);
        assert_eq!(failures.len(), 4);
        for (failure, seed) in failures.iter().zip(SEEDS) {
            assert!(failure.contains(&format!("seed={seed:#018x}")));
            assert!(failure.contains("expected=Run"));
            assert!(failure.contains("source:\n"));
            assert!(failure.contains("synthetic harness failure"));
        }
        let mut mixed = Vec::new();
        let failures = attempt_all(1, |seed, _| {
            mixed.push(seed);
            if seed == SEEDS[0] || seed == SEEDS[2] {
                panic!("mixed failure");
            }
        });
        assert_eq!(mixed, SEEDS);
        assert_eq!(failures.len(), 2);
        assert!(attempt_all(2, |_, _| {}).is_empty());
    }
}

#[test]
fn campaign_seed_generator_deterministic_independent_bounded() {
    bounded_seed_campaign::deterministic_bytes_independent_order_and_finite_operands();
}
#[test]
fn campaign_seed_generator_expected_output_strict_invalid() {
    bounded_seed_campaign::expected_output_and_strict_invalid_contract();
}
#[test]
fn campaign_seed_generator_panic_all_four_no_retry() {
    bounded_seed_campaign::panic_aggregation_attempts_all_four_without_retry();
}

#[test]
fn campaign_seed_owned_option_partial_drop() {
    bounded_seed_campaign::campaign(0);
}
#[test]
fn campaign_seed_scalar_projected_replace() {
    bounded_seed_campaign::campaign(1);
}
#[test]
fn campaign_seed_closure_result_option() {
    bounded_seed_campaign::campaign(2);
}
#[test]
fn campaign_seed_unused_live_projection_assignment() {
    bounded_seed_campaign::campaign(3);
}
#[test]
fn campaign_seed_unused_generic_wrong_argument() {
    bounded_seed_campaign::campaign(4);
}
#[test]
fn campaign_seed_unused_conditional_noncopy_move() {
    bounded_seed_campaign::campaign(5);
}

// Strict bounded x86_64 obligations, not full intrinsics support.
#[test]
#[ignore = "pending native compatibility obligation"]
fn runtime_intrinsics_sse2_ordered_lanes() {
    oracle::check(
        "runtime_intrinsics_sse2_ordered_lanes",
        &[Case {
            name: "runtime_intrinsics_sse2_ordered_lanes",
            source: r#"use std::arch::x86_64::{__m128i, _mm_add_epi32, _mm_setr_epi32, _mm_storeu_si128};
fn main() {
    let mut lanes = [0i32; 4];
    unsafe {
        let left = _mm_setr_epi32(1, 20, -3, 400);
        let right = _mm_setr_epi32(6, -2, 12, 5);
        let sum = _mm_add_epi32(left, right);
        _mm_storeu_si128(lanes.as_mut_ptr().cast::<__m128i>(), sum);
    }
    println!("{} {} {} {}", lanes[0], lanes[1], lanes[2], lanes[3]);
}
"#,
            expect: Expect::Run("7 18 9 405\n"),
        }],
    );
}

#[test]
#[ignore = "pending native compatibility obligation"]
fn runtime_intrinsics_target_feature_sse2_scalar() {
    oracle::check(
        "runtime_intrinsics_target_feature_sse2_scalar",
        &[Case {
            name: "runtime_intrinsics_target_feature_sse2_scalar",
            source: r#"#[target_feature(enable = "sse2")]
unsafe fn calculate(value: u32) -> u32 {
    value * 3 + 2
}
fn main() {
    let result = unsafe { calculate(7) };
    println!("{}", result);
}
"#,
            expect: Expect::Run("23\n"),
        }],
    );
}

#[test]
#[ignore = "pending native compatibility obligation"]
fn runtime_intrinsics_asm_register_add() {
    oracle::check(
        "runtime_intrinsics_asm_register_add",
        &[Case {
            name: "runtime_intrinsics_asm_register_add",
            source: r#"fn main() {
    let mut value = 7u64;
    unsafe {
        std::arch::asm!("add {value}, 5", value = inout(reg) value, options(nomem, nostack));
    }
    println!("{}", value);
}
"#,
            expect: Expect::Run("12\n"),
        }],
    );
}

#[test]
#[ignore = "pending native compatibility obligation"]
fn runtime_intrinsics_unused_simd_wrong_second_argument() {
    oracle::check(
        "runtime_intrinsics_unused_simd_wrong_second_argument",
        &[Case {
            name: "runtime_intrinsics_unused_simd_wrong_second_argument",
            source: r#"use std::arch::x86_64::{_mm_add_epi32, _mm_setr_epi32};
fn unused() {
    unsafe {
        let lanes = _mm_setr_epi32(1, 2, 3, 4);
        let _result = _mm_add_epi32(lanes, 7u32);
    }
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0308"),
        }],
    );
}

#[test]
#[ignore = "pending native compatibility obligation"]
fn runtime_intrinsics_unused_asm_without_unsafe() {
    oracle::check(
        "runtime_intrinsics_unused_asm_without_unsafe",
        &[Case {
            name: "runtime_intrinsics_unused_asm_without_unsafe",
            source: r#"fn unused() {
    let mut value = 7u64;
    std::arch::asm!("add {value}, 5", value = inout(reg) value, options(nomem, nostack));
}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("E0133"),
        }],
    );
}

#[test]
#[ignore = "pending native compatibility obligation"]
fn runtime_intrinsics_unused_nonexistent_target_feature() {
    oracle::check(
        "runtime_intrinsics_unused_nonexistent_target_feature",
        &[Case {
            name: "runtime_intrinsics_unused_nonexistent_target_feature",
            source: r#"#[target_feature(enable = "justrust_nonexistent_feature")]
unsafe fn unused_marker() {}
fn main() {}
"#,
            expect: Expect::InvalidAfterParse("not valid for this target"),
        }],
    );
}
