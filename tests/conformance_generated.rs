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
        (source, Expect::Invalid(code))
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
// This newly added source is stock-valid but native CHECK currently rejects it
// with "contract: nested index place projection". The progress driver always
// uses --include-ignored, so this remains a FAILED compatibility obligation,
// never a passing unsupported-boundary test. Keep the original source unchanged.
#[test]
#[ignore = "known native gap: owned move closure with shared tuple-field origin"]
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
    oracle::validate_semantic_rejection(&semantic).unwrap();
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
#[ignore = "pending native direct let borrow temporary lifetime extension obligation"]
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
