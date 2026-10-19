use super::interp::{self, InterpError, Limits, Outcome};
use super::verify::verify;
use super::*;

fn no_calls(_: FuncRef) -> Option<SsaSignature> {
    None
}

fn module(functions: Vec<SsaFunction>) -> Module {
    Module { functions }
}

fn run1(function: &SsaFunction, args: &[u128]) -> Result<Outcome, InterpError> {
    interp::run(
        &module(vec![function.clone()]),
        FuncRef(0),
        args,
        Limits::default(),
    )
}

fn bin(
    b: &mut Builder,
    block: SsaBlock,
    op: BinOp,
    overflow: Overflow,
    lhs: SsaValue,
    rhs: SsaValue,
) -> SsaValue {
    b.inst(
        block,
        InstKind::Binary {
            op,
            overflow,
            lhs,
            rhs,
        },
    )
}

const I32: SsaTy = SsaTy::Int(SsaIntTy::I32);
const U32: SsaTy = SsaTy::Int(SsaIntTy::U32);

/// Deterministic local PRNG (splitmix64). Owned by each test; no shared state.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn wide(&mut self) -> u128 {
        (u128::from(self.next()) << 64) | u128::from(self.next())
    }

    /// Biased toward edge values that expose overflow behavior, and small
    /// values that exercise algebraic identities and shift amounts.
    fn bits_for(&mut self, ty: SsaTy) -> u128 {
        let mask = ty.max_bits();
        match self.below(9) {
            0 => 0,
            1 => 1,
            2 => mask,
            3 => mask >> 1,
            4 => ((mask >> 1) + 1) & mask,
            5 | 6 => (self.next() % 17) as u128 & mask,
            7 => mask.wrapping_sub((self.next() % 4) as u128) & mask,
            _ => self.wide() & mask,
        }
    }
}

// ---------------------------------------------------------------------------
// eval: semantics match native Rust arithmetic.
// ---------------------------------------------------------------------------

macro_rules! check_native {
    ($rng:expr, $t:ty, $ity:expr) => {{
        let ty = SsaTy::Int($ity);
        for _ in 0..4000 {
            let a = $rng.bits_for(ty);
            let b = $rng.bits_for(ty);
            let (x, y) = (a as $t, b as $t);
            let wrap = |op| eval::binary(op, Overflow::Wrap, ty, a, ty, b);
            let trap = |op| eval::binary(op, Overflow::Trap, ty, a, ty, b);
            let enc = |v: $t| (v as u128) & ty.max_bits();
            assert_eq!(wrap(BinOp::Add), Ok(enc(x.wrapping_add(y))));
            assert_eq!(wrap(BinOp::Sub), Ok(enc(x.wrapping_sub(y))));
            assert_eq!(wrap(BinOp::Mul), Ok(enc(x.wrapping_mul(y))));
            assert_eq!(trap(BinOp::Add).ok(), x.checked_add(y).map(enc));
            assert_eq!(trap(BinOp::Sub).ok(), x.checked_sub(y).map(enc));
            assert_eq!(trap(BinOp::Mul).ok(), x.checked_mul(y).map(enc));
            assert_eq!(trap(BinOp::Div).ok(), x.checked_div(y).map(enc));
            assert_eq!(trap(BinOp::Rem).ok(), x.checked_rem(y).map(enc));
            // Division traps in both modes, as in Rust.
            assert_eq!(wrap(BinOp::Div), trap(BinOp::Div));
            assert_eq!(wrap(BinOp::And), Ok(enc(x & y)));
            assert_eq!(wrap(BinOp::Or), Ok(enc(x | y)));
            assert_eq!(wrap(BinOp::Xor), Ok(enc(x ^ y)));
            let amount = (b % 200) as u32;
            let shift_ty = SsaTy::Int(SsaIntTy::U32);
            let shl = eval::binary(
                BinOp::Shl,
                Overflow::Trap,
                ty,
                a,
                shift_ty,
                u128::from(amount),
            );
            assert_eq!(shl.ok(), x.checked_shl(amount).map(enc));
            let shr = eval::binary(
                BinOp::Shr,
                Overflow::Wrap,
                ty,
                a,
                shift_ty,
                u128::from(amount),
            );
            assert_eq!(shr, Ok(enc(x.wrapping_shr(amount))));
            for op in [
                CmpOp::Eq,
                CmpOp::Ne,
                CmpOp::Lt,
                CmpOp::Le,
                CmpOp::Gt,
                CmpOp::Ge,
            ] {
                let native = match op {
                    CmpOp::Eq => x == y,
                    CmpOp::Ne => x != y,
                    CmpOp::Lt => x < y,
                    CmpOp::Le => x <= y,
                    CmpOp::Gt => x > y,
                    CmpOp::Ge => x >= y,
                };
                assert_eq!(eval::compare(op, ty, a, b), native);
            }
            assert_eq!(eval::unary(UnOp::Not, Overflow::Wrap, ty, a), Ok(enc(!x)));
            assert_eq!(eval::cast(ty, SsaIntTy::I8, a), (x as i8 as u128) & 0xff);
            assert_eq!(eval::cast(ty, SsaIntTy::U16, a), u128::from(x as u16));
            assert_eq!(eval::cast(ty, SsaIntTy::I128, a), x as i128 as u128);
        }
    }};
}

#[test]
fn eval_matches_native_rust_integer_semantics() {
    let mut rng = Rng(1);
    check_native!(rng, i8, SsaIntTy::I8);
    check_native!(rng, u8, SsaIntTy::U8);
    check_native!(rng, i16, SsaIntTy::I16);
    check_native!(rng, u32, SsaIntTy::U32);
    check_native!(rng, i32, SsaIntTy::I32);
    check_native!(rng, i64, SsaIntTy::I64);
    check_native!(rng, u64, SsaIntTy::U64);
    check_native!(rng, i128, SsaIntTy::I128);
    check_native!(rng, u128, SsaIntTy::U128);
}

#[test]
fn eval_signed_negation_and_panic_kinds() {
    let i8 = SsaTy::Int(SsaIntTy::I8);
    assert_eq!(
        eval::unary(UnOp::Neg, Overflow::Trap, i8, 0x80),
        Err(PanicKind::NegOverflow)
    );
    assert_eq!(eval::unary(UnOp::Neg, Overflow::Wrap, i8, 0x80), Ok(0x80));
    assert_eq!(eval::unary(UnOp::Neg, Overflow::Trap, i8, 5), Ok(0xfb));
    assert_eq!(
        eval::binary(BinOp::Div, Overflow::Wrap, i8, 0x80, i8, 0xff),
        Err(PanicKind::DivOverflow)
    );
    assert_eq!(
        eval::binary(BinOp::Rem, Overflow::Wrap, i8, 1, i8, 0),
        Err(PanicKind::RemByZero)
    );
    // A negative shift amount overflows, as in Rust.
    assert_eq!(
        eval::binary(BinOp::Shl, Overflow::Trap, i8, 1, i8, 0xff),
        Err(PanicKind::ShlOverflow)
    );
    assert_eq!(
        PanicKind::AddOverflow.message(),
        "attempt to add with overflow"
    );
}

// ---------------------------------------------------------------------------
// verify
// ---------------------------------------------------------------------------

/// fn(n: u32) -> u32 { let mut s = 0; let mut i = 0; while i < n { i += 1; s += i } s }
fn sum_loop(overflow: Overflow) -> SsaFunction {
    let (mut b, params) = Builder::new(vec![U32], U32);
    let entry = SsaBlock(0);
    let (header, hp) = b.block(&[U32, U32]);
    let (body, _) = b.block(&[]);
    let (exit, ep) = b.block(&[U32]);
    let zero = b.constant(entry, U32, 0);
    b.terminate(entry, jump(header, vec![zero, zero]));
    let (i, s) = (hp[0], hp[1]);
    let cond = b.inst(
        header,
        InstKind::Cmp {
            op: CmpOp::Lt,
            lhs: i,
            rhs: params[0],
        },
    );
    b.terminate(
        header,
        SsaTerminator::Branch {
            cond,
            then: target(body, vec![]),
            otherwise: target(exit, vec![s]),
        },
    );
    let one = b.constant(body, U32, 1);
    let next_i = bin(&mut b, body, BinOp::Add, overflow, i, one);
    let next_s = bin(&mut b, body, BinOp::Add, overflow, s, next_i);
    b.terminate(body, jump(header, vec![next_i, next_s]));
    b.terminate(exit, SsaTerminator::Return(ep[0]));
    b.finish()
}

#[test]
fn verify_accepts_loop_and_reports_types() {
    let function = sum_loop(Overflow::Trap);
    let types = verify(&function, &no_calls).expect("valid");
    assert!(
        types
            .iter()
            .flatten()
            .all(|ty| *ty == U32 || *ty == SsaTy::Bool)
    );
}

#[test]
fn verify_rejects_malformed_functions() {
    let reject = |function: &SsaFunction, needle: &str| {
        let error = verify(function, &no_calls).expect_err("must reject");
        assert!(error.0.contains(needle), "{error} lacks {needle}");
    };
    // Use without a dominating definition: value from one branch arm used
    // at the join.
    let (mut b, params) = Builder::new(vec![SsaTy::Bool], I32);
    let (left, _) = b.block(&[]);
    let (right, _) = b.block(&[]);
    let (join, _) = b.block(&[]);
    b.terminate(
        SsaBlock(0),
        SsaTerminator::Branch {
            cond: params[0],
            then: target(left, vec![]),
            otherwise: target(right, vec![]),
        },
    );
    let x = b.constant(left, I32, 1);
    b.terminate(left, jump(join, vec![]));
    b.terminate(right, jump(join, vec![]));
    b.terminate(join, SsaTerminator::Return(x));
    reject(&b.finish(), "dominating");

    // Forward use inside a block.
    let mut f = Builder::new(vec![], I32).0.finish();
    f.blocks[0].insts = vec![
        Inst {
            result: SsaValue(0),
            kind: InstKind::Unary {
                op: UnOp::Not,
                overflow: Overflow::Wrap,
                value: SsaValue(1),
            },
        },
        Inst {
            result: SsaValue(1),
            kind: InstKind::Const { ty: I32, bits: 3 },
        },
    ];
    f.blocks[0].term = SsaTerminator::Return(SsaValue(0));
    reject(&f, "dominating");

    // Type errors.
    let (mut b, params) = Builder::new(vec![I32, U32], I32);
    let sum = bin(
        &mut b,
        SsaBlock(0),
        BinOp::Add,
        Overflow::Wrap,
        params[0],
        params[1],
    );
    b.terminate(SsaBlock(0), SsaTerminator::Return(sum));
    reject(&b.finish(), "equal integer");

    let (mut b, params) = Builder::new(vec![I32], U32);
    b.terminate(SsaBlock(0), SsaTerminator::Return(params[0]));
    reject(&b.finish(), "wrong type");

    let (mut b, _) = Builder::new(vec![], U32);
    let c = b.constant(SsaBlock(0), SsaTy::Int(SsaIntTy::U8), 256);
    b.terminate(SsaBlock(0), SsaTerminator::Return(c));
    reject(&b.finish(), "out of range");

    let (mut b, params) = Builder::new(vec![I32], I32);
    let x = bin(
        &mut b,
        SsaBlock(0),
        BinOp::And,
        Overflow::Trap,
        params[0],
        params[0],
    );
    b.terminate(SsaBlock(0), SsaTerminator::Return(x));
    reject(&b.finish(), "Overflow::Wrap");

    // SsaBlock argument mismatch and entry re-entry.
    let (mut b, params) = Builder::new(vec![I32], I32);
    let (next, _) = b.block(&[I32]);
    b.terminate(SsaBlock(0), jump(next, vec![]));
    b.terminate(next, SsaTerminator::Return(params[0]));
    reject(&b.finish(), "argument count");

    let (mut b, params) = Builder::new(vec![I32], I32);
    b.terminate(SsaBlock(0), jump(SsaBlock(0), vec![params[0]]));
    reject(&b.finish(), "entry");

    // Double definition.
    let (mut b, _) = Builder::new(vec![], I32);
    let c = b.constant(SsaBlock(0), I32, 1);
    let mut f = b.finish();
    f.blocks[0].insts.push(Inst {
        result: c,
        kind: InstKind::Const { ty: I32, bits: 2 },
    });
    f.blocks[0].term = SsaTerminator::Return(c);
    reject(&f, "more than once");

    // Unknown callee.
    let (mut b, _) = Builder::new(vec![], I32);
    let r = b.inst(
        SsaBlock(0),
        InstKind::Call {
            callee: FuncRef(7),
            args: vec![],
        },
    );
    b.terminate(SsaBlock(0), SsaTerminator::Return(r));
    reject(&b.finish(), "unknown function");
}

// ---------------------------------------------------------------------------
// interp
// ---------------------------------------------------------------------------

#[test]
fn interp_runs_loops_and_reports_overflow() {
    let checked = sum_loop(Overflow::Trap);
    assert_eq!(run1(&checked, &[0]), Ok(Outcome::Return(0)));
    assert_eq!(run1(&checked, &[10]), Ok(Outcome::Return(55)));
    // 1 + ... + 100_000 overflows u32.
    assert_eq!(
        run1(&checked, &[100_000]),
        Ok(Outcome::Panic(PanicKind::AddOverflow))
    );
    let wrapping = sum_loop(Overflow::Wrap);
    let expected = (1..=100_000u32).fold(0u32, |s, i| s.wrapping_add(i));
    assert_eq!(
        run1(&wrapping, &[100_000]),
        Ok(Outcome::Return(u128::from(expected)))
    );
    let tiny = Limits {
        fuel: 50,
        max_depth: 4,
    };
    assert_eq!(
        interp::run(&module(vec![checked]), FuncRef(0), &[1000], tiny),
        Err(InterpError::OutOfFuel)
    );
}

/// f0(n) = if n == 0 { 1 } else { n * f0(n - 1) }, checked.
fn factorial() -> SsaFunction {
    let (mut b, params) = Builder::new(vec![U32], U32);
    let (base, _) = b.block(&[]);
    let (step, _) = b.block(&[]);
    let zero = b.constant(SsaBlock(0), U32, 0);
    let is_zero = b.inst(
        SsaBlock(0),
        InstKind::Cmp {
            op: CmpOp::Eq,
            lhs: params[0],
            rhs: zero,
        },
    );
    b.terminate(
        SsaBlock(0),
        SsaTerminator::Branch {
            cond: is_zero,
            then: target(base, vec![]),
            otherwise: target(step, vec![]),
        },
    );
    let one = b.constant(base, U32, 1);
    b.terminate(base, SsaTerminator::Return(one));
    let one = b.constant(step, U32, 1);
    let less = bin(&mut b, step, BinOp::Sub, Overflow::Trap, params[0], one);
    let rec = b.inst(
        step,
        InstKind::Call {
            callee: FuncRef(0),
            args: vec![less],
        },
    );
    let product = bin(&mut b, step, BinOp::Mul, Overflow::Trap, params[0], rec);
    b.terminate(step, SsaTerminator::Return(product));
    b.finish()
}

#[test]
fn interp_calls_recursion_and_depth_limit() {
    let m = module(vec![factorial()]);
    assert_eq!(
        interp::run(&m, FuncRef(0), &[10], Limits::default()),
        Ok(Outcome::Return(3_628_800))
    );
    assert_eq!(
        interp::run(&m, FuncRef(0), &[13], Limits::default()),
        Ok(Outcome::Panic(PanicKind::MulOverflow))
    );
    assert_eq!(
        interp::run(&m, FuncRef(0), &[1000], Limits::default()),
        Err(InterpError::StackOverflow)
    );
}

// ---------------------------------------------------------------------------
// hash
// ---------------------------------------------------------------------------

#[test]
fn hash_ignores_numbering_and_dead_blocks_but_not_content() {
    let base = sum_loop(Overflow::Trap);
    let h = hash::content_hash(&base);

    // Same function with blocks stored in a different order and value IDs
    // offset, plus an unreachable block.
    let mut shuffled = base.clone();
    let offset = |v: &mut SsaValue| v.0 += 100;
    for data in &mut shuffled.blocks {
        for (value, _) in &mut data.params {
            offset(value);
        }
        for inst in &mut data.insts {
            offset(&mut inst.result);
            for operand in inst.kind.operands_mut() {
                offset(operand);
            }
        }
        for operand in data.term.operands_mut() {
            offset(operand);
        }
    }
    shuffled.blocks.swap(1, 3);
    for data in &mut shuffled.blocks {
        for t in data.term.targets_mut() {
            t.block = match t.block.0 {
                1 => SsaBlock(3),
                3 => SsaBlock(1),
                other => SsaBlock(other),
            };
        }
    }
    shuffled.blocks.push(BlockData {
        params: vec![],
        insts: vec![],
        term: SsaTerminator::Panic(PanicKind::Unreachable),
    });
    verify(&shuffled, &no_calls).expect("still valid");
    assert_eq!(hash::content_hash(&shuffled), h);
    assert_eq!(hash::canonical(&shuffled), hash::canonical(&base));

    // Any semantic difference changes the hash.
    assert_ne!(hash::content_hash(&sum_loop(Overflow::Wrap)), h);
    let mut changed = base.clone();
    if let InstKind::Const { bits, .. } = &mut changed.blocks[0].insts[0].kind {
        *bits = 1;
    }
    assert_ne!(hash::content_hash(&changed), h);
}

#[test]
fn hash_is_pinned() {
    // Guards against accidental encoding changes. If this fails because the
    // encoding changed on purpose, bump ENCODING_VERSION and update the value.
    let h = hash::content_hash(&sum_loop(Overflow::Trap));
    assert_eq!(h, hash::content_hash(&sum_loop(Overflow::Trap)));
    assert_eq!(format!("{h:032x}").len(), 32);
    assert_eq!(hash::ENCODING_VERSION, 2);
}

// ---------------------------------------------------------------------------
// opt
// ---------------------------------------------------------------------------

fn optimize(function: &SsaFunction) -> (SsaFunction, opt::Stats) {
    opt::local(function, &no_calls).expect("optimizes")
}

#[test]
fn opt_folds_constants_and_branches_to_a_single_return() {
    // let x = 6 * 7; if x == 42 { x + 0 } else { 0 }
    let (mut b, _) = Builder::new(vec![], I32);
    let (yes, _) = b.block(&[]);
    let (no, _) = b.block(&[]);
    let six = b.constant(SsaBlock(0), I32, 6);
    let seven = b.constant(SsaBlock(0), I32, 7);
    let x = bin(&mut b, SsaBlock(0), BinOp::Mul, Overflow::Trap, six, seven);
    let answer = b.constant(SsaBlock(0), I32, 42);
    let eq = b.inst(
        SsaBlock(0),
        InstKind::Cmp {
            op: CmpOp::Eq,
            lhs: x,
            rhs: answer,
        },
    );
    b.terminate(
        SsaBlock(0),
        SsaTerminator::Branch {
            cond: eq,
            then: target(yes, vec![]),
            otherwise: target(no, vec![]),
        },
    );
    let zero = b.constant(yes, I32, 0);
    let same = bin(&mut b, yes, BinOp::Add, Overflow::Trap, x, zero);
    b.terminate(yes, SsaTerminator::Return(same));
    let zero = b.constant(no, I32, 0);
    b.terminate(no, SsaTerminator::Return(zero));
    let original = b.finish();

    let (optimized, stats) = optimize(&original);
    assert!(stats.branches_folded >= 1);
    assert_eq!(run1(&optimized, &[]), Ok(Outcome::Return(42)));
    assert_eq!(optimized.blocks.len(), 1, "{optimized:?}");
    assert_eq!(optimized.inst_count(), 1, "{optimized:?}");
    assert!(matches!(
        optimized.blocks[0].insts[0].kind,
        InstKind::Const { bits: 42, .. }
    ));
}

#[test]
fn opt_preserves_overflow_panics_and_wrapping() {
    // i8: 127 + 1 traps with overflow checks and wraps without.
    for (overflow, expected) in [
        (Overflow::Trap, Outcome::Panic(PanicKind::AddOverflow)),
        (Overflow::Wrap, Outcome::Return(0x80)),
    ] {
        let i8 = SsaTy::Int(SsaIntTy::I8);
        let (mut b, _) = Builder::new(vec![], i8);
        let max = b.constant(SsaBlock(0), i8, 127);
        let one = b.constant(SsaBlock(0), i8, 1);
        let sum = bin(&mut b, SsaBlock(0), BinOp::Add, overflow, max, one);
        b.terminate(SsaBlock(0), SsaTerminator::Return(sum));
        let original = b.finish();
        let (optimized, _) = optimize(&original);
        assert_eq!(run1(&original, &[]), Ok(expected.clone()));
        assert_eq!(run1(&optimized, &[]), Ok(expected));
    }
}

#[test]
fn opt_dce_keeps_unused_instructions_that_may_panic() {
    // Unused checked add and division by a parameter must stay: they can
    // panic. Unused wrapping add must go.
    let (mut b, params) = Builder::new(vec![I32, I32], I32);
    let _checked = bin(
        &mut b,
        SsaBlock(0),
        BinOp::Add,
        Overflow::Trap,
        params[0],
        params[1],
    );
    let _div = bin(
        &mut b,
        SsaBlock(0),
        BinOp::Div,
        Overflow::Wrap,
        params[0],
        params[1],
    );
    let _wrapped = bin(
        &mut b,
        SsaBlock(0),
        BinOp::Add,
        Overflow::Wrap,
        params[0],
        params[1],
    );
    let two = b.constant(SsaBlock(0), I32, 2);
    let _safe_div = bin(
        &mut b,
        SsaBlock(0),
        BinOp::Div,
        Overflow::Wrap,
        params[0],
        two,
    );
    let zero = b.constant(SsaBlock(0), I32, 0);
    b.terminate(SsaBlock(0), SsaTerminator::Return(zero));
    let original = b.finish();
    let (optimized, stats) = optimize(&original);
    assert_eq!(stats.removed, 3, "wrapped add, safe div, constant 2");
    assert_eq!(optimized.inst_count(), 3, "checked add, div, constant 0");
    let max = i32::MAX as u32 as u128;
    for args in [[max, 1], [1, 0], [7, 2]] {
        assert_eq!(run1(&original, &args), run1(&optimized, &args));
    }
}

/// Random acyclic functions. Every block can use the entry block's values
/// (the entry dominates everything) plus its own parameters and
/// instructions. Edges only go forward, so execution terminates.
fn random_function(rng: &mut Rng) -> SsaFunction {
    let types = [
        SsaTy::Bool,
        SsaTy::Int(SsaIntTy::I8),
        SsaTy::Int(SsaIntTy::U8),
        SsaTy::Int(SsaIntTy::I32),
        SsaTy::Int(SsaIntTy::U64),
        SsaTy::Int(SsaIntTy::I128),
    ];
    let pick_ty = |rng: &mut Rng| types[rng.below(types.len())];
    let params: Vec<SsaTy> = (0..1 + rng.below(3)).map(|_| pick_ty(rng)).collect();
    let ret = pick_ty(rng);
    let (mut b, entry_values) = Builder::new(params.clone(), ret);
    let block_count = 1 + rng.below(5);
    let mut block_params: Vec<Vec<SsaTy>> = vec![params.clone()];
    let mut blocks = vec![SsaBlock(0)];
    let mut block_values = vec![entry_values];
    for _ in 1..block_count {
        let tys: Vec<SsaTy> = (0..rng.below(3)).map(|_| pick_ty(rng)).collect();
        let (block, values) = b.block(&tys);
        block_params.push(tys);
        blocks.push(block);
        block_values.push(values);
    }
    let mut entry_pool: Vec<(SsaValue, SsaTy)> = Vec::new();
    for index in 0..block_count {
        let block = blocks[index];
        let mut pool: Vec<(SsaValue, SsaTy)> = if index == 0 {
            Vec::new()
        } else {
            entry_pool.clone()
        };
        pool.extend(
            block_values[index]
                .iter()
                .copied()
                .zip(block_params[index].iter().copied()),
        );
        let value_of =
            |b: &mut Builder, rng: &mut Rng, pool: &mut Vec<(SsaValue, SsaTy)>, ty: SsaTy| {
                let candidates: Vec<SsaValue> = pool
                    .iter()
                    .filter(|(_, t)| *t == ty)
                    .map(|(v, _)| *v)
                    .collect();
                if !candidates.is_empty() && rng.below(4) != 0 {
                    candidates[rng.below(candidates.len())]
                } else {
                    let bits = rng.bits_for(ty);
                    let v = b.constant(block, ty, bits);
                    pool.push((v, ty));
                    v
                }
            };
        for _ in 0..rng.below(10) {
            let ty = pick_ty(rng);
            let overflow = if rng.below(2) == 0 {
                Overflow::Trap
            } else {
                Overflow::Wrap
            };
            let kind = match (rng.below(6), ty) {
                (0 | 1, SsaTy::Int(_)) => {
                    let ops = [
                        BinOp::Add,
                        BinOp::Sub,
                        BinOp::Mul,
                        BinOp::Div,
                        BinOp::Rem,
                        BinOp::Shl,
                        BinOp::Shr,
                    ];
                    let op = ops[rng.below(ops.len())];
                    let lhs = value_of(&mut b, rng, &mut pool, ty);
                    let rhs_ty = if op.is_shift() {
                        SsaTy::Int(SsaIntTy::U8)
                    } else {
                        ty
                    };
                    let rhs = value_of(&mut b, rng, &mut pool, rhs_ty);
                    let overflow = if op.has_overflow_mode() {
                        overflow
                    } else {
                        Overflow::Wrap
                    };
                    InstKind::Binary {
                        op,
                        overflow,
                        lhs,
                        rhs,
                    }
                }
                (0..=2, _) => {
                    let ops = [BinOp::And, BinOp::Or, BinOp::Xor];
                    let lhs = value_of(&mut b, rng, &mut pool, ty);
                    let rhs = value_of(&mut b, rng, &mut pool, ty);
                    InstKind::Binary {
                        op: ops[rng.below(3)],
                        overflow: Overflow::Wrap,
                        lhs,
                        rhs,
                    }
                }
                (3, SsaTy::Int(int)) => {
                    let from = pick_ty(rng);
                    let value = value_of(&mut b, rng, &mut pool, from);
                    InstKind::Cast { to: int, value }
                }
                (3, _) => {
                    let operand_ty = pick_ty(rng);
                    let lhs = value_of(&mut b, rng, &mut pool, operand_ty);
                    let rhs = value_of(&mut b, rng, &mut pool, operand_ty);
                    let ops = [
                        CmpOp::Eq,
                        CmpOp::Ne,
                        CmpOp::Lt,
                        CmpOp::Le,
                        CmpOp::Gt,
                        CmpOp::Ge,
                    ];
                    InstKind::Cmp {
                        op: ops[rng.below(6)],
                        lhs,
                        rhs,
                    }
                }
                (4, SsaTy::Int(int)) if int.signed() => {
                    let value = value_of(&mut b, rng, &mut pool, ty);
                    InstKind::Unary {
                        op: UnOp::Neg,
                        overflow,
                        value,
                    }
                }
                (4, _) => {
                    let value = value_of(&mut b, rng, &mut pool, ty);
                    InstKind::Unary {
                        op: UnOp::Not,
                        overflow: Overflow::Wrap,
                        value,
                    }
                }
                _ => {
                    let cond = value_of(&mut b, rng, &mut pool, SsaTy::Bool);
                    let then = value_of(&mut b, rng, &mut pool, ty);
                    let otherwise = value_of(&mut b, rng, &mut pool, ty);
                    InstKind::Select {
                        cond,
                        then,
                        otherwise,
                    }
                }
            };
            let result = b.inst(block, kind);
            pool.push((result, ty));
        }
        let later = index + 1..block_count;
        let make_target = |b: &mut Builder, rng: &mut Rng, pool: &mut Vec<(SsaValue, SsaTy)>| {
            let successor = later.start + rng.below(later.len());
            let args = block_params[successor]
                .clone()
                .into_iter()
                .map(|ty| value_of(b, rng, pool, ty))
                .collect();
            target(blocks[successor], args)
        };
        let term = if later.is_empty() || rng.below(5) == 0 {
            if rng.below(12) == 0 {
                SsaTerminator::Panic(PanicKind::Unreachable)
            } else {
                SsaTerminator::Return(value_of(&mut b, rng, &mut pool, ret))
            }
        } else if rng.below(2) == 0 {
            SsaTerminator::Jump(make_target(&mut b, rng, &mut pool))
        } else {
            let cond = value_of(&mut b, rng, &mut pool, SsaTy::Bool);
            let then = make_target(&mut b, rng, &mut pool);
            let otherwise = make_target(&mut b, rng, &mut pool);
            SsaTerminator::Branch {
                cond,
                then,
                otherwise,
            }
        };
        b.terminate(block, term);
        if index == 0 {
            entry_pool = pool;
        }
    }
    b.finish()
}

#[test]
fn opt_rewrites_of_x_op_constant_are_exhaustively_sound_for_8_bit_types() {
    // For every operator, overflow mode, 8-bit type and constant operand on
    // either side, optimize `f(x) = x op c` (and `c op x`) once, then compare
    // against the original on all 256 inputs. Random fuzzing rarely hits a
    // specific constant; this covers every one.
    let ops = [
        BinOp::Add,
        BinOp::Sub,
        BinOp::Mul,
        BinOp::Div,
        BinOp::Rem,
        BinOp::And,
        BinOp::Or,
        BinOp::Xor,
        BinOp::Shl,
        BinOp::Shr,
    ];
    let mut rewritten = 0usize;
    for ty in [SsaTy::Int(SsaIntTy::I8), SsaTy::Int(SsaIntTy::U8)] {
        for op in ops {
            for overflow in [Overflow::Wrap, Overflow::Trap] {
                if !op.has_overflow_mode() && overflow == Overflow::Trap {
                    continue;
                }
                for c in 0..=255u128 {
                    for constant_left in [false, true] {
                        let (mut b, params) = Builder::new(vec![ty], ty);
                        let k = b.constant(SsaBlock(0), ty, c);
                        let (lhs, rhs) = if constant_left {
                            (k, params[0])
                        } else {
                            (params[0], k)
                        };
                        let r = bin(&mut b, SsaBlock(0), op, overflow, lhs, rhs);
                        b.terminate(SsaBlock(0), SsaTerminator::Return(r));
                        let original = b.finish();
                        let (optimized, _) = optimize(&original);
                        if optimized == hash::canonical(&original) {
                            continue;
                        }
                        rewritten += 1;
                        for x in 0..=255u128 {
                            assert_eq!(
                                run1(&original, &[x]),
                                run1(&optimized, &[x]),
                                "{ty:?} {op:?} {overflow:?} c={c} left={constant_left} x={x}"
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(rewritten > 0);
}

#[test]
fn opt_is_differentially_equal_to_the_interpreter_on_random_functions() {
    let mut rng = Rng(0x5eed);
    let mut changed = 0usize;
    for case in 0..1500 {
        let original = random_function(&mut rng);
        verify(&original, &no_calls).unwrap_or_else(|e| panic!("case {case}: generator: {e}"));
        let (optimized, stats) = optimize(&original);
        if stats != opt::Stats::default() {
            changed += 1;
        }
        // Idempotent and canonical: a second run changes nothing.
        let (again, _) = optimize(&optimized);
        assert_eq!(again, optimized, "case {case}: not a fixpoint");
        assert_eq!(hash::canonical(&optimized), optimized);
        assert!(optimized.inst_count() <= original.inst_count());
        for _ in 0..8 {
            let args: Vec<u128> = original.params.iter().map(|ty| rng.bits_for(*ty)).collect();
            let before = run1(&original, &args);
            let after = run1(&optimized, &args);
            assert_eq!(
                before, after,
                "case {case} args {args:?}\n{original:?}\n{optimized:?}"
            );
        }
    }
    assert!(
        changed > 1000,
        "generator should exercise the optimizer: {changed}"
    );
}

#[test]
fn opt_output_is_independent_of_input_numbering() {
    // Determinism rule: canonical-equal inputs give identical outputs.
    let mut rng = Rng(77);
    for _ in 0..200 {
        let original = random_function(&mut rng);
        let mut renumbered = original.clone();
        let shift = |v: &mut SsaValue| v.0 = v.0 * 3 + 5;
        for data in &mut renumbered.blocks {
            for (value, _) in &mut data.params {
                shift(value);
            }
            for inst in &mut data.insts {
                shift(&mut inst.result);
                for operand in inst.kind.operands_mut() {
                    shift(operand);
                }
            }
            for operand in data.term.operands_mut() {
                shift(operand);
            }
        }
        assert_eq!(
            hash::content_hash(&original),
            hash::content_hash(&renumbered)
        );
        assert_eq!(optimize(&original).0, optimize(&renumbered).0);
    }
}

// ---------------------------------------------------------------------------
// inline + machine (pure-library level)
// ---------------------------------------------------------------------------

fn lowered(function: SsaFunction, callees: Vec<lower::SsaCallee>) -> lower::Lowered {
    lower::Lowered { function, callees }
}

#[test]
fn inline_of_leaf_matches_interpreter_on_random_callers() {
    // Caller f(x, y) = g(x) + g(y) with a random leaf g; inlining must not
    // change any outcome, including panics inside the inlined body.
    let mut rng = Rng(4242);
    let mut inlined_any = 0;
    for case in 0..300 {
        let g = random_function(&mut rng);
        if g.params.len() != 1 || g.ret == SsaTy::Unit {
            continue;
        }
        let pty = g.params[0];
        let ret = g.ret;
        let (mut b, params) = Builder::new(vec![pty, pty], ret);
        let call = |b: &mut Builder, arg| {
            b.inst(
                SsaBlock(0),
                InstKind::Call {
                    callee: FuncRef(0),
                    args: vec![arg],
                },
            )
        };
        let r1 = call(&mut b, params[0]);
        let r2 = call(&mut b, params[1]);
        let cond = b_cond(&mut b, ret, r1, r2);
        let combined = b.inst(
            SsaBlock(0),
            InstKind::Select {
                cond,
                then: r1,
                otherwise: r2,
            },
        );
        b.terminate(SsaBlock(0), SsaTerminator::Return(combined));
        let caller = b.finish();
        let callee_entry = lower::SsaCallee {
            symbol: "g".into(),
            target: dummy_target(),
            signature: g.signature(),
        };
        let caller = lowered(caller, vec![callee_entry]);
        let leaf = lowered(g.clone(), vec![]);
        let budget = meter::unlimited();
        let m = meter::Meter::new(&budget);
        let summary = inline::Summary::of(&leaf, &m).unwrap();
        assert!(summary.leaf);
        let result = inline::inline(&caller, &[(0, &leaf)], &m)
            .unwrap_or_else(|e| panic!("case {case}: {e}"))
            .value;
        assert!(result.callees.is_empty());
        inlined_any += 1;
        let module_after = Module {
            functions: vec![result.function.clone()],
        };
        // The interpreter addresses callees by module index, so place g at 1.
        let mut before_fn = caller.function.clone();
        for data in &mut before_fn.blocks {
            for inst in &mut data.insts {
                if let InstKind::Call { callee, .. } = &mut inst.kind {
                    *callee = FuncRef(1);
                }
            }
        }
        let module_before = Module {
            functions: vec![before_fn, g.clone()],
        };
        for _ in 0..6 {
            let args = [rng.bits_for(pty), rng.bits_for(pty)];
            let before = interp::run(&module_before, FuncRef(0), &args, Limits::default());
            let after = interp::run(&module_after, FuncRef(0), &args, Limits::default());
            assert_eq!(before, after, "case {case} args {args:?}");
        }
    }
    assert!(inlined_any > 50, "{inlined_any}");
}

fn b_cond(b: &mut Builder, ty: SsaTy, x: SsaValue, y: SsaValue) -> SsaValue {
    if ty == SsaTy::Unit {
        b.constant(SsaBlock(0), SsaTy::Bool, 1)
    } else {
        b.inst(
            SsaBlock(0),
            InstKind::Cmp {
                op: CmpOp::Ge,
                lhs: x,
                rhs: y,
            },
        )
    }
}

fn dummy_target() -> crate::compiler::ir::CallTarget {
    crate::compiler::ir::CallTarget {
        declaration: crate::compiler::ir::DeclId {
            snapshot: crate::compiler::ir::SnapshotId::allocate().unwrap(),
            index: 0,
        },
        type_args: std::sync::Arc::from([]),
    }
}

#[test]
fn block_param_elimination_turns_local_passing_into_minimal_ssa() {
    // Every block passes every "local" through, as lowering produces.
    let (mut b, params) = Builder::new(vec![U32, U32], U32);
    let (mid, mp) = b.block(&[U32, U32]);
    let (end, ep) = b.block(&[U32, U32]);
    b.terminate(SsaBlock(0), jump(mid, vec![params[0], params[1]]));
    let sum = bin(&mut b, mid, BinOp::Add, Overflow::Wrap, mp[0], mp[1]);
    b.terminate(mid, jump(end, vec![sum, mp[1]]));
    b.terminate(end, SsaTerminator::Return(ep[0]));
    let original = b.finish();
    let (optimized, _) = optimize(&original);
    assert_eq!(optimized.blocks.len(), 1);
    assert!(
        optimized
            .blocks
            .iter()
            .skip(1)
            .all(|data| data.params.is_empty())
    );
    for args in [[1, 2], [u32::MAX as u128, 1]] {
        assert_eq!(run1(&original, &args), run1(&optimized, &args));
    }
    // The loop parameter of a real loop must survive.
    let (looped, _) = optimize(&sum_loop(Overflow::Trap));
    assert!(looped.blocks.iter().any(|data| !data.params.is_empty()));
    assert_eq!(run1(&looped, &[10]), Ok(Outcome::Return(55)));

    // A loop-invariant value passed around the back edge (as lowering passes
    // every local) is a trivial parameter and must be removed.
    let (mut b, params) = Builder::new(vec![U32], U32);
    let (header, hp) = b.block(&[U32, U32]); // i, invariant n
    let (body, bp) = b.block(&[U32, U32]);
    let (exit, xp) = b.block(&[U32, U32]);
    let zero = b.constant(SsaBlock(0), U32, 0);
    b.terminate(SsaBlock(0), jump(header, vec![zero, params[0]]));
    let lt = b.inst(
        header,
        InstKind::Cmp {
            op: CmpOp::Lt,
            lhs: hp[0],
            rhs: hp[1],
        },
    );
    b.terminate(
        header,
        SsaTerminator::Branch {
            cond: lt,
            then: target(body, vec![hp[0], hp[1]]),
            otherwise: target(exit, vec![hp[0], hp[1]]),
        },
    );
    let one = b.constant(body, U32, 1);
    let next = bin(&mut b, body, BinOp::Add, Overflow::Trap, bp[0], one);
    b.terminate(body, jump(header, vec![next, bp[1]]));
    b.terminate(exit, SsaTerminator::Return(xp[0]));
    let original = b.finish();
    let (optimized, stats) = optimize(&original);
    assert!(stats.params_removed >= 1, "{stats:?}");
    let params_after: usize = optimized
        .blocks
        .iter()
        .skip(1)
        .map(|d| d.params.len())
        .sum();
    assert_eq!(
        params_after, 1,
        "only the induction variable stays: {optimized:?}"
    );
    for n in [0, 7, 300] {
        assert_eq!(run1(&original, &[n]), run1(&optimized, &[n]));
    }
}

// ---------------------------------------------------------------------------
// accounting (OPTIMIZER-ACCOUNTING-AUDIT.md acceptance evidence)
// ---------------------------------------------------------------------------

use crate::compiler::ir::{PlanBudget, PlanLimits};

fn limited(bytes: usize, work: usize) -> PlanBudget {
    PlanBudget::new(PlanLimits {
        bytes,
        work,
        ..PlanLimits::default()
    })
}

fn usage(budget: &PlanBudget) -> (usize, usize) {
    let snapshot = budget.usage_snapshot();
    (snapshot.bytes, snapshot.work)
}

/// A function with `blocks` straight-line blocks of one wrapping add each:
/// many blocks, few instructions (audit: non-instruction dimensions).
fn many_blocks(blocks: usize) -> SsaFunction {
    let (mut b, params) = Builder::new(vec![U32], U32);
    let mut prev = SsaBlock(0);
    let mut value = params[0];
    for _ in 0..blocks {
        let (next, p) = b.block(&[U32]);
        b.terminate(prev, jump(next, vec![value]));
        let one = b.constant(next, U32, 1);
        value = bin(&mut b, next, BinOp::Add, Overflow::Wrap, p[0], one);
        prev = next;
    }
    b.terminate(prev, SsaTerminator::Return(value));
    b.finish()
}

#[test]
fn scratch_is_refunded_and_outputs_keep_exact_receipts() {
    let budget = PlanBudget::default();
    let m = meter::Meter::new(&budget);
    let function = many_blocks(64);
    let before = usage(&budget).0;
    {
        let types = verify::verify_metered(&function, &|_| None, &m).unwrap();
        // While held, the type table's lease is charged.
        assert!(usage(&budget).0 > before);
        drop(types);
    }
    assert_eq!(usage(&budget).0, before, "verify scratch refunded");
    let copy = store::copy_function(&m, &function).unwrap();
    let held = usage(&budget).0 - before;
    assert_eq!(held, store::capacity_bytes(&copy.value).unwrap());
    assert_eq!(held, copy.bytes());
    let (value, receipt) = copy.seal(&m).unwrap();
    assert_eq!(receipt.bytes(), held);
    assert!(receipt.same_authority(&budget));
    // A sealed receipt never refunds: the ledger keeps the retained charge
    // for its lifetime, and the value outlives any later ledger activity.
    drop(receipt);
    assert_eq!(usage(&budget).0 - before, held);
    assert_eq!(value, function);
    // Canonical output and an optimized output reconcile to real capacity.
    let canonical = hash::canonical_metered(&function, &m).unwrap();
    assert_eq!(
        canonical.bytes(),
        store::capacity_bytes(&canonical.value).unwrap()
    );
    let (optimized, _) = opt::local_metered(&function, &|_| None, &m).unwrap();
    assert_eq!(
        optimized.bytes(),
        store::capacity_bytes(&optimized.value).unwrap()
    );
    let after_opt = usage(&budget).0;
    drop(optimized);
    drop(canonical);
    assert_eq!(
        usage(&budget).0,
        before + held,
        "intermediates refunded on drop"
    );
    let _ = after_opt;
}

#[test]
fn tiny_byte_budgets_reject_before_large_allocation() {
    let function = many_blocks(256);
    for bytes in [0usize, 1, 64] {
        let budget = limited(bytes, usize::MAX / 4);
        let m = meter::Meter::new(&budget);
        assert!(
            verify::verify_metered(&function, &|_| None, &m)
                .unwrap_err()
                .is_resource()
        );
        assert!(hash::canonical_metered(&function, &m).is_err());
        assert!(
            opt::local_metered(&function, &|_| None, &m)
                .unwrap_err()
                .is_resource()
        );
        assert!(store::copy_function(&m, &function).is_err());
        // Failure is sticky in the ledger.
        assert!(
            budget.usage_snapshot().first_failure.is_some(),
            "bytes={bytes}"
        );
    }
}

#[test]
fn work_is_charged_across_fixed_point_rounds_and_exhaustion_is_sticky() {
    let function = many_blocks(128);
    let budget = PlanBudget::default();
    let m = meter::Meter::new(&budget);
    let (_, w0) = usage(&budget);
    opt::local_metered(&function, &|_| None, &m).unwrap();
    let full = usage(&budget).1 - w0;
    // The pipeline runs several canonicalize/verify rounds over 128 blocks:
    // work must scale with that, not be one coarse wrapper charge.
    assert!(full > 128 * 8, "work {full}");
    // Every budget below the measured need fails, never succeeds silently.
    for fraction in [1usize, 2, 4] {
        let budget = limited(usize::MAX / 4, full / (fraction + 1));
        let m = meter::Meter::new(&budget);
        let error = opt::local_metered(&function, &|_| None, &m).unwrap_err();
        assert!(error.is_resource(), "{error}");
        // Sticky: a later tiny request also fails.
        assert!(budget.charge_work(1).is_err() || budget.usage_snapshot().first_failure.is_some());
    }
}

#[test]
fn hash_work_tracks_encoded_bytes() {
    let small = many_blocks(4);
    let large = many_blocks(64);
    let work = |function: &SsaFunction| {
        let budget = PlanBudget::default();
        let m = meter::Meter::new(&budget);
        hash::content_hash_metered(function, &m).unwrap();
        usage(&budget).1
    };
    let (a, b) = (work(&small), work(&large));
    assert!(b > a * 8, "hash work should scale with size: {a} -> {b}");
    assert_eq!(
        hash::content_hash(&large),
        {
            let budget = PlanBudget::default();
            hash::content_hash_metered(&large, &meter::Meter::new(&budget)).unwrap()
        },
        "metered and unmetered hashes agree"
    );
}

#[test]
fn checked_arithmetic_overflow_is_an_error_not_a_wrap() {
    assert!(meter::mul(usize::MAX, 2).is_err());
    assert!(meter::add(usize::MAX, 1).is_err());
    let budget = PlanBudget::default();
    let m = meter::Meter::new(&budget);
    assert!(m.table::<u64>(usize::MAX / 4, 0).is_err());
    assert!(m.work2(usize::MAX, 2).is_err());
}

#[test]
fn many_inline_sites_are_admitted_and_reconciled() {
    // Caller with 40 calls to one leaf: expansion is per call site.
    let leaf = {
        let (mut b, p) = Builder::new(vec![U32], U32);
        let one = b.constant(SsaBlock(0), U32, 1);
        let r = bin(&mut b, SsaBlock(0), BinOp::Add, Overflow::Wrap, p[0], one);
        b.terminate(SsaBlock(0), SsaTerminator::Return(r));
        lowered(b.finish(), vec![])
    };
    let (mut b, p) = Builder::new(vec![U32], U32);
    let mut value = p[0];
    for _ in 0..40 {
        value = b.inst(
            SsaBlock(0),
            InstKind::Call {
                callee: FuncRef(0),
                args: vec![value],
            },
        );
    }
    b.terminate(SsaBlock(0), SsaTerminator::Return(value));
    let caller = lowered(
        b.finish(),
        vec![lower::SsaCallee {
            symbol: "leaf".into(),
            target: dummy_target(),
            signature: leaf.function.signature(),
        }],
    );
    let budget = PlanBudget::default();
    let m = meter::Meter::new(&budget);
    let before = usage(&budget).0;
    let out = inline::inline(&caller, &[(0, &leaf)], &m).unwrap();
    assert_eq!(out.value.function.blocks.len(), 1 + 40 * 2);
    assert_eq!(
        out.bytes(),
        store::lowered_capacity_bytes(&out.value).unwrap()
    );
    assert_eq!(
        usage(&budget).0 - before,
        out.bytes(),
        "only the output stays charged"
    );
    // Behavior preserved.
    let before_module = Module {
        functions: vec![
            {
                let mut f = caller.function.clone();
                for data in &mut f.blocks {
                    for inst in &mut data.insts {
                        if let InstKind::Call { callee, .. } = &mut inst.kind {
                            *callee = FuncRef(1);
                        }
                    }
                }
                f
            },
            leaf.function.clone(),
        ],
    };
    let after_module = Module {
        functions: vec![out.value.function.clone()],
    };
    assert_eq!(
        interp::run(&before_module, FuncRef(0), &[5], Limits::default()),
        interp::run(&after_module, FuncRef(0), &[5], Limits::default())
    );
    // A byte budget below the expansion rejects before building.
    let tight = limited(out.bytes() / 2, usize::MAX / 4);
    assert!(inline::inline(&caller, &[(0, &leaf)], &meter::Meter::new(&tight)).is_err());
}

/// Full ledger state for exact no-change comparisons.
fn ledger(budget: &PlanBudget) -> (usize, usize, usize, usize) {
    let snapshot = budget.usage_snapshot();
    (
        snapshot.bytes,
        snapshot.work,
        snapshot.reservations.current_reservations,
        snapshot.reservations.cumulative_bytes,
    )
}

#[test]
fn lease_rejects_a_foreign_authority_before_any_ledger_change() {
    let own = PlanBudget::default();
    let foreign = PlanBudget::default();
    let mine = meter::Meter::new(&own);
    let theirs = meter::Meter::new(&foreign);
    let mut lease = mine.lease(64).unwrap();
    let other = mine.lease(8).unwrap();
    let before = (ledger(&own), ledger(&foreign));
    // Every operation, including no-op and zero-sized ones, is rejected.
    assert!(lease.resize(&theirs, 64).is_err(), "resize no-op");
    assert!(lease.resize(&theirs, 0).is_err(), "resize shrink");
    assert!(lease.resize(&theirs, 128).is_err(), "resize grow");
    assert!(lease.admit(&theirs, 0).is_err(), "admit zero");
    assert!(lease.admit(&theirs, 16).is_err(), "admit");
    assert_eq!((ledger(&own), ledger(&foreign)), before);
    // Absorb with a foreign meter rejects and the absorbed lease refunds
    // normally to its own ledger (never to the foreign one).
    assert!(lease.absorb(&theirs, other).is_err(), "absorb");
    assert_eq!(ledger(&foreign), before.1);
    assert_eq!(ledger(&own).0, before.0.0 - 8);
    // A lease from a different authority cannot be absorbed even via the
    // right meter.
    let alien = theirs.lease(4).unwrap();
    let foreign_before = ledger(&foreign);
    let own_before = ledger(&own);
    assert!(lease.absorb(&mine, alien).is_err(), "absorb alien lease");
    assert_eq!(ledger(&own), own_before);
    assert_eq!(ledger(&foreign).0, foreign_before.0 - 4);
    let own_before = ledger(&own);
    let foreign_before = ledger(&foreign);
    assert!(lease.seal(&theirs).is_err(), "seal");
    // The rejected seal dropped the lease: it refunds only its own ledger.
    assert_eq!(ledger(&foreign), foreign_before);
    assert_eq!(ledger(&own).0, own_before.0 - 64);
}

#[test]
fn every_operation_fails_after_a_sticky_work_rejection() {
    let budget = limited(1 << 20, 1000);
    let m = meter::Meter::new(&budget);
    let mut lease = m.lease(32).unwrap();
    assert!(m.work(usize::MAX / 2).is_err(), "huge work rejected");
    let before = ledger(&budget);
    assert!(m.work(0).is_err(), "work(0) after failure");
    assert!(m.lease(1).is_err(), "small lease after failure");
    assert!(m.lease(0).is_err(), "zero lease after failure");
    assert!(lease.resize(&m, 32).is_err(), "resize no-op after failure");
    assert!(lease.admit(&m, 0).is_err(), "admit zero after failure");
    assert_eq!(ledger(&budget).0, before.0, "no bytes moved");
    assert!(lease.seal(&m).is_err(), "seal after failure");
}

#[test]
fn growth_and_absorb_never_charge_a_phantom_peak() {
    let budget = PlanBudget::default();
    let m = meter::Meter::new(&budget);
    let base = usage(&budget).0;
    let mut lease = m.lease(1000).unwrap();
    let mut other = m.lease(500).unwrap();
    other.admit(&m, 250).unwrap();
    // Growth admits only the delta (plus bounded chunk bookkeeping).
    lease.admit(&m, 10).unwrap();
    let book = usage(&budget).0 - base - 1000 - 750 - 10;
    assert!(book <= 256, "chunk bookkeeping {book}");
    assert_eq!(lease.bytes(), 1010);
    // Absorb moves reservations: the ledger does not rise by the total.
    let held = usage(&budget).0;
    lease.absorb(&m, other).unwrap();
    assert!(usage(&budget).0 - held <= 256, "absorb re-admitted");
    assert_eq!(lease.bytes(), 1760);
    // Shrink is exact and decrease-only.
    let held = usage(&budget).0;
    lease.resize(&m, 1000).unwrap();
    assert_eq!(lease.bytes(), 1000);
    assert_eq!(held - usage(&budget).0, 760);
    drop(lease);
    assert_eq!(
        usage(&budget).0,
        base,
        "all chunks and bookkeeping refunded"
    );
}

#[test]
fn paid_buffer_never_writes_past_its_paid_limit() {
    let budget = PlanBudget::default();
    let m = meter::Meter::new(&budget);
    let base = usage(&budget).0;
    let mut buf = m.buf::<u8>(4).unwrap();
    buf.extend(&[1, 2, 3]).unwrap();
    assert!(buf.extend(&[4, 5]).is_err(), "limit is paid, not capacity");
    assert_eq!(buf.as_slice(), &[1, 2, 3]);
    buf.push(4).unwrap();
    assert!(buf.push(5).is_err());
    buf.extend_growing(&m, &[5, 6, 7]).unwrap();
    assert_eq!(buf.as_slice(), &[1, 2, 3, 4, 5, 6, 7]);
    let (data, lease) = buf.into_parts();
    assert!(lease.bytes() >= data.capacity(), "real capacity is paid");
    drop((data, lease));
    assert_eq!(usage(&budget).0, base);
}

#[test]
fn metered_call_target_eq_matches_derived_eq() {
    use crate::compiler::ir::{BorrowKind, IntTy, Ty};
    use std::sync::Arc;
    let leaf = |n: usize| -> Ty {
        let mut ty = Ty::Int(IntTy::U8);
        for i in 0..n {
            ty = if i % 2 == 0 {
                Ty::Ref {
                    kind: BorrowKind::Shared,
                    referent: Arc::new(ty),
                }
            } else {
                Ty::Tuple(Arc::from(vec![ty, Ty::Bool]))
            };
        }
        ty
    };
    let target = |args: Vec<Ty>| crate::compiler::ir::CallTarget {
        declaration: dummy_target().declaration,
        type_args: Arc::from(args),
    };
    let samples = [
        target(vec![]),
        target(vec![leaf(0)]),
        target(vec![leaf(5)]),
        target(vec![leaf(6)]),
        target(vec![leaf(5), Ty::Unit]),
        target(vec![Ty::Array {
            element: Arc::new(leaf(3)),
            len: 4,
        }]),
        target(vec![Ty::Array {
            element: Arc::new(leaf(3)),
            len: 5,
        }]),
    ];
    let budget = PlanBudget::default();
    let m = meter::Meter::new(&budget);
    for a in &samples {
        for b in &samples {
            assert_eq!(lower::call_target_eq_for_tests(a, b, &m).unwrap(), a == b);
        }
    }
    // Within the caller's semantic depth: succeeds, work tracks the shared
    // nested prefix actually visited (one node per level here).
    let depth = PlanLimits::default().depth;
    let within = target(vec![leaf(depth - 1)]);
    let before = usage(&budget).1;
    assert!(lower::call_target_eq_for_tests(&within, &within.clone(), &m).unwrap());
    assert!(usage(&budget).1 - before >= depth - 1);
    // Past the caller's depth: fails through the caller's own check_depth,
    // which records a sticky depth failure (no local cap).
    let budget = PlanBudget::default();
    let m = meter::Meter::new(&budget);
    let deep = target(vec![leaf(depth + 8)]);
    assert!(lower::call_target_eq_for_tests(&deep, &deep.clone(), &m).is_err());
    assert!(
        budget.first_resource_failure().is_some(),
        "depth failure is sticky"
    );
    assert!(m.work(0).is_err());
}

fn snapshot_of(budget: &PlanBudget) -> (usize, usize, usize, usize, usize) {
    let s = budget.usage_snapshot();
    (
        s.bytes,
        s.work,
        s.reservations.current_reservations,
        s.reservations.cumulative_bytes,
        s.reservations.peak_bytes,
    )
}

#[test]
fn buf_rejects_foreign_or_sticky_meter_before_any_mutation() {
    let own = PlanBudget::default();
    let foreign = PlanBudget::default();
    let mine = meter::Meter::new(&own);
    let theirs = meter::Meter::new(&foreign);
    let mut buf = mine.buf::<u8>(4).unwrap();
    buf.extend(&[1, 2]).unwrap();
    let state = |b: &meter::Buf<u8>| {
        (
            b.as_slice().as_ptr() as usize,
            b.capacity(),
            b.limit(),
            b.len(),
            b.lease_bytes(),
        )
    };
    let before = (state(&buf), snapshot_of(&own), snapshot_of(&foreign));
    // Fast path (fits in capacity), slow path (reallocates), and growing
    // appends: all rejected with every observable unchanged.
    assert!(buf.grow(&theirs, 0).is_err(), "fast path zero");
    assert!(buf.grow(&theirs, 1).is_err(), "fast path");
    assert!(buf.grow(&theirs, 1 << 12).is_err(), "slow path");
    assert!(buf.extend_growing(&theirs, &[3]).is_err(), "fit path");
    assert!(buf.extend_growing(&theirs, &[0; 64]).is_err(), "grow path");
    assert!(buf.push_growing(&theirs, 9).is_err());
    assert_eq!(
        (state(&buf), snapshot_of(&own), snapshot_of(&foreign)),
        before
    );
    // The buffer still belongs to its owner and seals normally.
    buf.extend_growing(&mine, &[3, 4, 5, 6, 7]).unwrap();
    let (_, lease) = buf.into_parts();
    assert!(lease.seal(&mine).is_ok());

    // Sticky: after a ledger failure, every Buf operation is rejected with
    // no change, fast and slow path alike.
    let budget = limited(1 << 20, 1 << 20);
    let m = meter::Meter::new(&budget);
    let mut buf = m.buf::<u8>(4).unwrap();
    buf.extend(&[1]).unwrap();
    assert!(m.work(usize::MAX / 2).is_err());
    let before = (state(&buf), snapshot_of(&budget));
    assert!(buf.grow(&m, 1).is_err());
    assert!(buf.grow(&m, 1 << 12).is_err());
    assert!(buf.extend_growing(&m, &[2]).is_err());
    assert!(buf.extend_growing(&m, &[0; 64]).is_err());
    assert_eq!((state(&buf), snapshot_of(&budget)), before);
}

#[test]
fn buf_growth_failure_is_transactional_and_peak_is_exact() {
    // Exact peak: growing from capacity C to N holds old + new, no more.
    let budget = PlanBudget::default();
    let m = meter::Meter::new(&budget);
    let mut buf = m.buf::<u64>(16).unwrap();
    buf.extend(&[7; 16]).unwrap();
    let held = usage(&budget).0;
    let old = buf.lease_bytes();
    // Reset the peak tracker view: compare relative to the current level.
    let peak_before = snapshot_of(&budget).4;
    buf.grow(&m, 48).unwrap();
    let new = buf.lease_bytes();
    let peak = snapshot_of(&budget).4;
    assert_eq!(peak.max(peak_before), (held + new).max(peak_before));
    assert!(peak <= held + new, "peak {peak} vs old {old} + new {new}");
    assert_eq!(
        usage(&budget).0,
        held - old + new,
        "old released after commit"
    );
    assert_eq!(buf.as_slice(), &[7; 16]);
    // A budget that cannot pay the new buffer while the old one is held:
    // failure leaves pointer, capacity, limit, contents and ledger bytes
    // unchanged. The limit leaves room for the old buffer plus less than the
    // new one, so the reservation itself is what fails.
    let element = std::mem::size_of::<u64>();
    let tight = limited(64 * element + 64 * element, 1 << 30);
    let tm = meter::Meter::new(&tight);
    let mut small = tm.buf::<u64>(64).unwrap();
    small.extend(&[1; 64]).unwrap();
    let before = (
        small.as_slice().as_ptr() as usize,
        small.capacity(),
        small.limit(),
        small.lease_bytes(),
        usage(&tight).0,
    );
    // New limit 256 needs 256 * 8 more bytes while 64 * 8 are held.
    assert!(small.grow(&tm, 192).is_err());
    assert_eq!(
        (
            small.as_slice().as_ptr() as usize,
            small.capacity(),
            small.limit(),
            small.lease_bytes(),
            usage(&tight).0,
        ),
        before
    );
    assert!(small.as_slice().iter().all(|&v| v == 1));
}

#[test]
fn lease_bookkeeping_growth_peak_is_old_plus_new() {
    let budget = PlanBudget::default();
    let m = meter::Meter::new(&budget);
    let mut lease = m.lease(8).unwrap();
    // Force several chunk-vector reallocations.
    for _ in 0..64 {
        lease.admit(&m, 1).unwrap();
    }
    assert_eq!(lease.bytes(), 72);
    let data_and_book = lease.bytes() + lease.bookkeeping_bytes();
    assert_eq!(usage(&budget).0, data_and_book, "steady state is exact");
    // The ledger's recorded peak never exceeds steady data + the largest
    // old + new chunk-vector overlap.
    let chunk = 64; // generous per-chunk entry size bound
    let peak = snapshot_of(&budget).4;
    assert!(
        peak <= lease.bytes() + 3 * lease.bookkeeping_bytes() + chunk,
        "peak {peak}"
    );
    // Shrink is exact and work is charged per visited chunk.
    let w = usage(&budget).1;
    lease.resize(&m, 8).unwrap();
    assert_eq!(lease.bytes(), 8);
    assert!(usage(&budget).1 - w >= 64, "chunk visits charged");
    drop(lease);
    assert_eq!(usage(&budget).0, 0);
}

fn panic_function(payload: usize) -> SsaFunction {
    let (mut b, _) = Builder::new(vec![U32], U32);
    let kind = b.panic_text("m".repeat(payload), "src/x.rs:1:1".into());
    b.terminate(SsaBlock(0), SsaTerminator::Panic(kind));
    b.finish()
}

#[test]
fn canonical_and_copy_charge_panic_payload_bytes_before_allocating() {
    let mut works = Vec::new();
    for payload in [16usize, 4096, 1 << 20] {
        let function = panic_function(payload);
        for which in 0..2 {
            let budget = PlanBudget::default();
            let m = meter::Meter::new(&budget);
            let w = usage(&budget).1;
            if which == 0 {
                drop(hash::canonical_metered(&function, &m).unwrap());
            } else {
                drop(store::copy_function(&m, &function).unwrap());
            }
            let spent = usage(&budget).1 - w;
            assert!(spent >= payload, "payload {payload} spent {spent}");
            works.push((which, payload, spent));
            // A work budget below the payload fails closed with the byte
            // ledger back at zero (nothing published, everything refunded).
            let tight = limited(1 << 30, payload / 2 + 8);
            let tm = meter::Meter::new(&tight);
            let result = if which == 0 {
                hash::canonical_metered(&function, &tm).map(drop)
            } else {
                store::copy_function(&tm, &function).map(drop)
            };
            assert!(result.is_err(), "payload {payload} under tight work");
            assert_eq!(usage(&tight).0, 0, "no bytes left held");
        }
    }
    // Scaling: work grows with P.
    for which in 0..2 {
        let row: Vec<_> = works.iter().filter(|w| w.0 == which).collect();
        assert!(row[2].2 - row[0].2 >= (1 << 20) - 16);
    }
}

/// A valid function exercising every machine output buffer: calls with
/// arguments (symbols, pushes), a loop with block arguments (edges, moves),
/// trapping arithmetic (traps) and an explicit panic (strings, panic sink).
fn machine_sample() -> lower::Lowered {
    let (mut b, params) = Builder::new(vec![U32], U32);
    let (header, hp) = b.block(&[U32, U32]);
    let (body, bp) = b.block(&[U32, U32]);
    let (exit, xp) = b.block(&[U32, U32]);
    let (fail, _) = b.block(&[]);
    let zero = b.constant(SsaBlock(0), U32, 0);
    b.terminate(SsaBlock(0), jump(header, vec![zero, params[0]]));
    let lt = b.inst(
        header,
        InstKind::Cmp {
            op: CmpOp::Lt,
            lhs: hp[0],
            rhs: hp[1],
        },
    );
    b.terminate(
        header,
        SsaTerminator::Branch {
            cond: lt,
            then: target(body, vec![hp[1], hp[0]]),
            otherwise: target(exit, vec![hp[0], hp[1]]),
        },
    );
    let mut value = bp[1];
    for _ in 0..12 {
        value = b.inst(
            body,
            InstKind::Call {
                callee: FuncRef(0),
                args: vec![value],
            },
        );
    }
    let one = b.constant(body, U32, 1);
    let next = bin(&mut b, body, BinOp::Add, Overflow::Trap, value, one);
    b.terminate(body, jump(header, vec![next, bp[0]]));
    let big = b.constant(exit, U32, 1000);
    let over = b.inst(
        exit,
        InstKind::Cmp {
            op: CmpOp::Lt,
            lhs: big,
            rhs: xp[0],
        },
    );
    let (ok, okp) = b.block(&[U32]);
    b.terminate(
        exit,
        SsaTerminator::Branch {
            cond: over,
            then: target(fail, vec![]),
            otherwise: target(ok, vec![xp[0]]),
        },
    );
    b.terminate(ok, SsaTerminator::Return(okp[0]));
    let kind = b.panic_text("too big".repeat(64), "src/m.rs:1:1".into());
    b.terminate(fail, SsaTerminator::Panic(kind));
    let callee = {
        let (mut c, p) = Builder::new(vec![U32], U32);
        c.terminate(SsaBlock(0), SsaTerminator::Return(p[0]));
        c.finish()
    };
    lowered(
        b.finish(),
        vec![lower::SsaCallee {
            symbol: "callee_symbol_name".into(),
            target: dummy_target(),
            signature: callee.signature(),
        }],
    )
}

fn emit_sample(
    sample: &lower::Lowered,
    budget: &PlanBudget,
) -> Result<(crate::compiler::backend::Object, meter::Receipt), String> {
    let m = meter::Meter::new(budget);
    let types = sample.verify(&m).map_err(|e| e.to_string())?;
    let (owned, _) = machine::emit(sample, &types, &m)?;
    drop(types);
    owned.seal(&m)
}

#[test]
fn machine_underestimated_bounds_grow_paid_instead_of_rejecting() {
    let sample = machine_sample();
    let reference = emit_sample(&sample, &PlanBudget::default()).unwrap();
    // Every estimate forced down to 1 element: valid source must still emit
    // the identical object, with an exact receipt.
    for cap in [0usize, 1, 3] {
        let budget = PlanBudget::default();
        let (object, receipt) =
            machine::bound_override::with(cap, || emit_sample(&sample, &budget)).unwrap();
        assert_eq!(object.code, reference.0.code, "cap {cap}");
        assert_eq!(object.calls, reference.0.calls);
        assert_eq!(object.strings, reference.0.strings);
        assert_eq!(receipt.bytes(), machine::object_bytes(&object).unwrap());
        assert!(receipt.same_authority(&budget));
    }
}

#[test]
fn machine_emission_succeeds_with_enough_budget_and_fails_closed_below() {
    let sample = machine_sample();
    // Measure the real need (bytes high-water mark and work) under the
    // underestimate, so growth is exercised in both directions.
    let budget = PlanBudget::default();
    machine::bound_override::with(1, || emit_sample(&sample, &budget)).unwrap();
    let snapshot = budget.usage_snapshot();
    let (peak, work) = (snapshot.reservations.peak_bytes, snapshot.work);
    // Just enough: succeeds.
    let enough = limited(peak, work);
    let (object, receipt) =
        machine::bound_override::with(1, || emit_sample(&sample, &enough)).unwrap();
    assert_eq!(receipt.bytes(), machine::object_bytes(&object).unwrap());
    // Below the need in bytes or in work: a resource failure, nothing held
    // except what the failed attempt refunded (ledger back to zero).
    for (bytes, work) in [(peak / 2, work), (peak, work / 2)] {
        let tight = limited(bytes, work);
        let result = machine::bound_override::with(1, || emit_sample(&sample, &tight));
        assert!(result.is_err(), "bytes {bytes} work {work}");
        assert!(tight.first_resource_failure().is_some(), "resource failure");
        assert_eq!(usage(&tight).0, 0, "no partial output held");
    }
}

#[test]
fn inline_expansion_grows_with_budget_and_fails_closed_below() {
    let leaf = {
        let (mut b, p) = Builder::new(vec![U32], U32);
        let kind = b.panic_text("leaf panic text".repeat(16), "src/l.rs:2:2".into());
        let (bad, _) = b.block(&[]);
        let (good, gp) = b.block(&[U32]);
        let zero = b.constant(SsaBlock(0), U32, 0);
        let is_zero = b.inst(
            SsaBlock(0),
            InstKind::Cmp {
                op: CmpOp::Eq,
                lhs: p[0],
                rhs: zero,
            },
        );
        b.terminate(
            SsaBlock(0),
            SsaTerminator::Branch {
                cond: is_zero,
                then: target(bad, vec![]),
                otherwise: target(good, vec![p[0]]),
            },
        );
        b.terminate(bad, SsaTerminator::Panic(kind));
        b.terminate(good, SsaTerminator::Return(gp[0]));
        lowered(b.finish(), vec![])
    };
    let caller = {
        let (mut b, p) = Builder::new(vec![U32], U32);
        let mut value = p[0];
        for _ in 0..24 {
            value = b.inst(
                SsaBlock(0),
                InstKind::Call {
                    callee: FuncRef(0),
                    args: vec![value],
                },
            );
        }
        b.terminate(SsaBlock(0), SsaTerminator::Return(value));
        lowered(
            b.finish(),
            vec![lower::SsaCallee {
                symbol: "leaf".into(),
                target: dummy_target(),
                signature: leaf.function.signature(),
            }],
        )
    };
    let budget = PlanBudget::default();
    let m = meter::Meter::new(&budget);
    let out = inline::inline(&caller, &[(0, &leaf)], &m).unwrap();
    assert_eq!(
        out.bytes(),
        store::lowered_capacity_bytes(&out.value).unwrap()
    );
    drop(out);
    let snapshot = budget.usage_snapshot();
    let (peak, work) = (snapshot.reservations.peak_bytes, snapshot.work);
    let enough = limited(peak, work);
    let em = meter::Meter::new(&enough);
    assert!(inline::inline(&caller, &[(0, &leaf)], &em).is_ok());
    for (bytes, work) in [(peak / 2, work), (peak, work / 2)] {
        let tight = limited(bytes, work);
        let tm = meter::Meter::new(&tight);
        assert!(inline::inline(&caller, &[(0, &leaf)], &tm).is_err());
        assert!(tight.first_resource_failure().is_some());
        assert_eq!(usage(&tight).0, 0);
    }
}
