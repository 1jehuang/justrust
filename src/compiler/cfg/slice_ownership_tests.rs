//! Shared byte slice (`&[u8]`) CFG lowering, ownership and borrow analysis.
//! Native emission is gated separately by the backend; these tests exercise
//! check-only compilation, which still requires successful ownership analysis.
use super::*;

const ADLER: &str = "fn adler2_adapted_update(bytes: &[u8], checksum: u32) -> u32 {
    let mut a: u32 = checksum % 65536;
    let mut b: u32 = checksum / 65536;
    let mut i: usize = 0;
    while i < bytes.len() {
        a = (a + bytes[i] as u32) % 65521;
        b = (b + a) % 65521;
        i = i + 1;
    }
    b * 65536 + a
}
";

fn check(source: &str) -> Result<(), String> {
    super::super::compile(source.into(), 1, true)
        .map(|_| ())
        .map_err(|error| error.to_string())
}
fn accepted(source: &str) {
    if let Err(error) = check(source) {
        panic!("rejected valid slice program: {error}\n{source}");
    }
}
fn rejected(source: &str) -> String {
    match check(source) {
        Ok(()) => panic!("accepted invalid slice program:\n{source}"),
        Err(error) => error,
    }
}
/// Rejected by ownership/borrow analysis, not by an admission fence.
fn borrow_rejected(source: &str) {
    let error = rejected(source);
    assert!(
        error.contains("initialization/move analysis")
            && (error.contains("BorrowConflict") || error.contains("StorageNotDefinitelyLive")),
        "{error}\n{source}"
    );
}
fn with_main(body: &str) -> String {
    format!("{ADLER}{body}")
}

fn lowered(source: &str, name: &str) -> Function {
    let source = if source.contains("fn main") {
        source.to_string()
    } else {
        format!("{source} fn main() {{}}")
    };
    let program = super::super::frontend::parse(&source).unwrap();
    let signatures = super::super::frontend::signature_table(&program).unwrap();
    let index = program
        .functions
        .iter()
        .position(|function| function.name == name)
        .expect("function");
    let checked =
        super::super::frontend::type_function_with_signatures(&program, index, &signatures)
            .unwrap();
    lower(&checked).unwrap()
}

#[test]
fn adler_recurrence_lowers_to_slice_operations_once() {
    let source = with_main(
        "fn main() { let p: [u8; 4] = [0, 1, 128, 255]; let s = adler2_adapted_update(&p, 1); }",
    );
    let f = lowered(&source, "adler2_adapted_update");
    f.validate().unwrap();
    let mut lens = 0;
    let mut reads = 0;
    for block in &f.blocks {
        for statement in &block.statements {
            match &statement.value {
                Statement::Assign {
                    value: Rvalue::SliceLen { slice },
                    ..
                } => {
                    lens += 1;
                    assert!(matches!(slice, Operand::Copy(p) if p.projection.is_empty()));
                }
                Statement::Assign {
                    value: Rvalue::SliceIndex { slice, index },
                    ..
                } => {
                    reads += 1;
                    let (Operand::Copy(s), Operand::Copy(i)) = (slice, index) else {
                        panic!("slice index operands must be copied snapshots");
                    };
                    assert_ne!(s.local, i.local, "receiver and index share a temp");
                    assert!(s.projection.is_empty() && i.projection.is_empty());
                }
                _ => {}
            }
        }
    }
    assert_eq!((lens, reads), (1, 1));
    let analysis = super::super::ownership::analyze(&f);
    assert!(analysis.passed(), "{analysis:?}");
}

#[test]
fn array_to_slice_evaluates_reference_once_and_copies_it() {
    let source = "fn f() -> usize { let s: [u8; 2] = [1, 2]; let b: &[u8] = &s; b.len() }";
    let f = lowered(source, "f");
    f.validate().unwrap();
    let unsizes: Vec<_> = f
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match &statement.value {
            Statement::Assign {
                value: Rvalue::ArrayToSlice { reference },
                ..
            } => Some(reference.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(unsizes.len(), 1);
    assert!(matches!(&unsizes[0], Operand::Copy(p) if p.projection.is_empty()));
    assert_eq!(f.loan_sites.len(), 1);
    assert!(super::super::ownership::analyze(&f).passed());
}

#[test]
fn validator_rejects_malformed_slice_operands() {
    let source = "fn f(b: &[u8], i: usize) -> u8 { b[i] }";
    let mut f = lowered(source, "f");
    f.validate().unwrap();
    let mut mutated = 0;
    for block in &mut f.blocks {
        for statement in &mut block.statements {
            if let Statement::Assign {
                value: Rvalue::SliceIndex { slice, .. },
                ..
            } = &mut statement.value
                && let Operand::Copy(place) = slice.clone()
            {
                *slice = Operand::Move(place);
                mutated += 1;
            }
        }
    }
    assert_eq!(mutated, 1);
    assert!(f.validate().unwrap_err().contains("whole local"));
}

#[test]
fn adler_fixture_and_callers_pass_check() {
    accepted(&with_main(
        "fn main() { let p0: [u8; 4] = [0, 1, 128, 255]; let mut state: u32 = 1;
         state = adler2_adapted_update(&p0, state); let e: [u8; 0] = [0u8; 0];
         state = adler2_adapted_update(&e, state); }",
    ));
    accepted(&with_main("fn main() {}"));
    accepted(
        "fn g(b: &[u8]) -> usize { b.len() }
         fn f(b: &[u8]) -> usize { let c = b; let d: &[u8] = c; g(b) + g(d) + c.len() }
         fn main() { let s: [u8; 3] = [1, 2, 3]; let r: &[u8; 3] = &s; let n = f(r) + g(&s); }",
    );
    accepted(
        "fn main() { let s: [u8; 2] = [1, 2]; let t: [u8; 1] = [9]; let mut b: &[u8] = &s;
         let x = b[1]; b = &t; let y = b[0]; let n = b.len(); }",
    );
}

#[test]
fn dangling_slice_from_dead_array_is_rejected() {
    borrow_rejected(
        "fn main() { let b: &[u8]; { let s: [u8; 2] = [1, 2]; b = &s; } let x = b[0]; }",
    );
    borrow_rejected(
        "fn main() { let e: [u8; 0] = [0u8; 0]; let mut b: &[u8] = &e; loop { let s: [u8; 1] = [3]; b = &s; break; } let n = b[0]; }",
    );
}

#[test]
fn slice_live_across_mutation_of_array_is_rejected() {
    borrow_rejected(
        "fn main() { let mut s: [u8; 2] = [1, 2]; let b: &[u8] = &s; s[0] = 5; let x = b[0]; }",
    );
    borrow_rejected(
        "fn main() { let mut s: [u8; 2] = [1, 2]; let b: &[u8] = &s; s = [3, 4]; let n = b.len(); }",
    );
    borrow_rejected(
        "fn touch(a: &mut [u8; 2]) {} fn main() { let mut s: [u8; 2] = [1, 2]; let b: &[u8] = &s;
         let m = &mut s; touch(m); let x = b[1]; }",
    );
    // Once the slice is dead, mutation is fine.
    accepted(
        "fn main() { let mut s: [u8; 2] = [1, 2]; let b: &[u8] = &s; let x = b[0]; s[0] = x; }",
    );
}

#[test]
fn copied_array_keeps_slice_and_moved_holder_kills_it() {
    // `[u8; N]` is Copy: passing it by value copies and the slice stays valid.
    accepted(
        "fn take(a: [u8; 2]) {} fn main() { let s: [u8; 2] = [1, 2]; let b: &[u8] = &s; take(s); let x = b[0]; }",
    );
    // Storage that ends while a slice derived from it is still used.
    borrow_rejected(
        "fn main() { let b: &[u8]; { let s: [u8; 2] = [1, 2]; let r: &[u8; 2] = &s; b = r; } let n = b.len(); }",
    );
}

#[test]
fn slice_escape_into_return_is_rejected() {
    let error = rejected("fn f<'a>(b: &'a [u8]) -> &'a [u8] { b }");
    assert!(error.contains("unsupported"), "{error}");
    let error = rejected("fn f(b: &[u8], c: &[u8]) -> &[u8] { c }");
    assert!(error.contains("unsupported"), "{error}");
}

#[test]
fn unused_and_unreachable_slice_bodies_still_validate() {
    let error = rejected(
        "fn unused(b: &[u8]) -> u8 { let s: [u8; 1] = [1]; let c: &[u8]; { let t: [u8; 1] = [2]; c = &t; } c[0] }
         fn main() {}",
    );
    assert!(!error.is_empty());
    // Dead slice operations are still type-validated before publication.
    rejected("fn main() { return; let s: [u8; 1] = [1]; let b: &[u8] = &s; let x: bool = b[0]; }");
    rejected("fn unused(b: &[u8]) { let n: u8 = b.len(); } fn main() {}");
}

#[test]
fn large_byte_arrays_unsize_to_slices_past_the_old_4096_limit() {
    // Adler2 NMAX-boundary inputs (5552 and 22208 bytes) must check; the
    // literal array limit stays finite and is enforced exactly.
    accepted(&with_main(
        "fn main() { let p: [u8; 5552] = [255; 5552]; let s = adler2_adapted_update(&p, 1);
         let q: [u8; 22208] = [255; 22208]; let t = adler2_adapted_update(&q, s); }",
    ));
    let limit = super::super::ir::MAX_ARRAY_LEN;
    let error = rejected(&format!(
        "fn main() {{ let p: [u8; {}] = [0; {}]; }}",
        limit + 1,
        limit + 1
    ));
    assert!(error.contains("limit"), "{error}");
}
