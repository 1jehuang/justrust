//! Shared slice CFG lowering, ownership and borrow analysis.
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
        .position(|function| {
            let path = &function.name;
            path.anchor() == crate::compiler::semantic_path::PathAnchor::Crate
                && path.len() == 1
                && program.names.spelling(path.segments()[0]).unwrap() == name
        })
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
    // Whole descriptor moves are legal, including the unique-slice transport
    // introduced by MUTABLE-SLICES.md. A projection is still not a snapshot.
    f.validate().unwrap();
    for block in &mut f.blocks {
        for statement in &mut block.statements {
            if let Statement::Assign {
                value:
                    Rvalue::SliceIndex {
                        slice: Operand::Move(place),
                        ..
                    },
                ..
            } = &mut statement.value
            {
                place.projection.push(Projection::Deref);
            }
        }
    }
    assert!(f.validate().is_err(), "projected slice carrier accepted");
    for block in &f.blocks {
        for statement in &block.statements {
            if let Statement::Assign {
                value: Rvalue::SliceIndex { slice, .. },
                ..
            } = &statement.value
            {
                assert_eq!(
                    f.slice_operand_ty(
                        slice,
                        ir::FactsCtx::Concrete,
                        &mut crate::compiler::traits::TraitScratch::default(),
                    ),
                    Err("CFG slice operand must copy a whole local".into())
                );
            }
        }
    }
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
fn slice_returns_preserve_declared_lifetime_and_reject_ambiguous_elision() {
    accepted(
        "fn f<'a>(b: &'a [u8]) -> &'a [u8] { b } fn main() { let a = [1u8]; let s = f(&a); let n = s.len(); }",
    );
    let error = rejected("fn f(b: &[u8], c: &[u8]) -> &[u8] { c } fn main() {}");
    assert!(error.contains("missing lifetime specifier"), "{error}");
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

#[test]
fn typed_slice_index_returns_exact_independent_copy_element_values() {
    for element in [
        "i32",
        "i128",
        "u128",
        "f64",
        "bool",
        "(i32,bool,i128)",
        "[i64;2]",
    ] {
        let function = lowered(
            &format!("fn f(s:&[{element}],i:usize)->{element}{{s[i]}}"),
            "f",
        );
        function.validate().unwrap();
        let expected = function.locals[function.params[0].0]
            .ty
            .shared_slice_element()
            .unwrap();
        assert_eq!(&function.result, expected);
        let mut reads = 0;
        for statement in function.blocks.iter().flat_map(|block| &block.statements) {
            if let Statement::Assign {
                destination,
                value: Rvalue::SliceIndex { slice, index },
            } = &statement.value
            {
                reads += 1;
                assert_eq!(function.place_ty(destination).unwrap(), expected);
                assert_eq!(function.operand_ty(index).unwrap(), Ty::Int(IntTy::Usize));
                let (Operand::Copy(handle), Operand::Copy(subscript)) = (slice, index) else {
                    panic!("whole copied snapshots required")
                };
                assert!(handle.projection.is_empty() && subscript.projection.is_empty());
                assert_ne!(destination.local, handle.local);
                assert_ne!(handle.local, subscript.local);
            }
        }
        assert_eq!(reads, 1, "{element}");
        assert!(
            super::super::ownership::analyze(&function).passed(),
            "{element}"
        );
    }
}

#[test]
fn typed_array_unsizing_keeps_element_identity_and_rejects_byte_reinterpretation() {
    let mut function = lowered(
        "fn f()->usize{let data:[i128;2]=[1,2];let view:&[i128]=&data;view.len()}",
        "f",
    );
    let (destination, reference) = function
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .find_map(|statement| match &statement.value {
            Statement::Assign {
                destination,
                value: Rvalue::ArrayToSlice { reference },
            } => Some((destination.clone(), reference.clone())),
            _ => None,
        })
        .unwrap();
    let array = function.operand_ty(&reference).unwrap();
    let (element, len) = array.shared_array_element_len().unwrap();
    assert_eq!(len, 2);
    assert_eq!(
        function
            .place_ty(&destination)
            .unwrap()
            .shared_slice_element(),
        Some(element)
    );
    assert_eq!(function.loan_sites.len(), 1);
    function.validate().unwrap();
    function.locals[destination.local.0].ty = Ty::shared_byte_slice_ty();
    assert!(function.validate().is_err());
}

#[test]
fn noncopy_slice_length_is_legal_but_forged_whole_element_copy_is_not() {
    let mut function = lowered("struct Cell{n:i32}fn f(s:&[Cell])->usize{s.len()}", "f");
    function.validate().unwrap();
    assert!(super::super::ownership::analyze(&function).passed());
    let mut changed = 0;
    for statement in function
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.statements)
    {
        if let Statement::Assign { value, .. } = &mut statement.value
            && let Rvalue::SliceLen { slice } = value
        {
            *value = Rvalue::SliceIndex {
                slice: slice.clone(),
                index: Operand::Constant(Constant::Int(IntValue::usize(0))),
            };
            changed += 1;
        }
    }
    assert_eq!(changed, 1);
    assert!(function.validate().unwrap_err().contains("non-Copy"));
    let error = rejected("struct Cell{n:i32}fn f(s:&[Cell])->i32{s[0].n}fn main(){}");
    assert!(error.contains("unsupported"), "{error}");
}

#[test]
fn typed_slice_generic_copy_readers_and_noncopy_len_keep_real_closed_boundaries() {
    accepted(
        "fn first<T:Copy>(s:&[T])->T{s[0]}fn main(){let data:[(i32,bool,i128);2]=[(1,true,2),(3,false,4)];let value=first::<(i32,bool,i128)>(&data);let wide:[u128;1]=[7];let n=first::<u128>(&wide);}",
    );
    accepted(
        "struct Cell{n:i32}fn size<T>(s:&[T])->usize{s.len()}fn main(){let data:[Cell;2]=[Cell{n:1},Cell{n:2}];let n=size::<Cell>(&data);}",
    );
    let error = rejected(
        "fn first<T>(s:&[T])->T{s[0]}fn main(){let data:[i32;1]=[1];let value=first::<i32>(&data);}",
    );
    assert!(!error.is_empty());
    accepted(
        "fn size<T>(s:&[T])->usize{s.len()}fn main(){let n=1;let data:[&i32;1]=[&n];let len=size::<&i32>(&data);}",
    );
}

#[test]
fn typed_slice_origins_current_cell_last_use_and_backing_lifetime_remain_authoritative() {
    borrow_rejected(
        "fn main(){let mut data:[i32;2]=[1,2];let view:&[i32]=&data;data[0]=3;let value=view[0];}",
    );
    borrow_rejected(
        "fn main(){let view:&[i128];{let data:[i128;1]=[1];view=&data;}let value=view[0];}",
    );
    accepted(
        "fn main(){let mut first:[i32;2]=[1,2];let second:[i32;1]=[3];let mut view:&[i32]=&first;let old=view[0];view=&second;first[0]=old;let value=view[0];}",
    );
    accepted(
        "fn main(){let mut data:[i128;1]=[1];let view:&[i128]=&data;let value=view[0];data[0]=value;}",
    );
}
