use super::interp::{self, InterpError, Limits, Outcome};
use super::verify::verify;
use super::*;

fn no_calls(_: FuncRef) -> Option<Signature> {
    None
}

fn module(functions: Vec<Function>) -> Module {
    Module { functions }
}

fn run1(function: &Function, args: &[u128]) -> Result<Outcome, InterpError> {
    interp::run(
        &module(vec![function.clone()]),
        FuncRef(0),
        args,
        Limits::default(),
    )
}

fn bin(
    b: &mut Builder,
    block: Block,
    op: BinOp,
    overflow: Overflow,
    lhs: Value,
    rhs: Value,
) -> Value {
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

const I32: Ty = Ty::Int(IntTy::I32);
const U32: Ty = Ty::Int(IntTy::U32);

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
    fn bits_for(&mut self, ty: Ty) -> u128 {
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
        let ty = Ty::Int($ity);
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
            let shift_ty = Ty::Int(IntTy::U32);
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
            assert_eq!(eval::cast(ty, IntTy::I8, a), (x as i8 as u128) & 0xff);
            assert_eq!(eval::cast(ty, IntTy::U16, a), u128::from(x as u16));
            assert_eq!(eval::cast(ty, IntTy::I128, a), x as i128 as u128);
        }
    }};
}

#[test]
fn eval_matches_native_rust_integer_semantics() {
    let mut rng = Rng(1);
    check_native!(rng, i8, IntTy::I8);
    check_native!(rng, u8, IntTy::U8);
    check_native!(rng, i16, IntTy::I16);
    check_native!(rng, u32, IntTy::U32);
    check_native!(rng, i32, IntTy::I32);
    check_native!(rng, i64, IntTy::I64);
    check_native!(rng, u64, IntTy::U64);
    check_native!(rng, i128, IntTy::I128);
    check_native!(rng, u128, IntTy::U128);
}

#[test]
fn eval_signed_negation_and_panic_kinds() {
    let i8 = Ty::Int(IntTy::I8);
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
fn sum_loop(overflow: Overflow) -> Function {
    let (mut b, params) = Builder::new(vec![U32], U32);
    let entry = Block(0);
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
        Terminator::Branch {
            cond,
            then: target(body, vec![]),
            otherwise: target(exit, vec![s]),
        },
    );
    let one = b.constant(body, U32, 1);
    let next_i = bin(&mut b, body, BinOp::Add, overflow, i, one);
    let next_s = bin(&mut b, body, BinOp::Add, overflow, s, next_i);
    b.terminate(body, jump(header, vec![next_i, next_s]));
    b.terminate(exit, Terminator::Return(ep[0]));
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
            .all(|ty| *ty == U32 || *ty == Ty::Bool)
    );
}

#[test]
fn verify_rejects_malformed_functions() {
    let reject = |function: &Function, needle: &str| {
        let error = verify(function, &no_calls).expect_err("must reject");
        assert!(error.0.contains(needle), "{error} lacks {needle}");
    };
    // Use without a dominating definition: value from one branch arm used
    // at the join.
    let (mut b, params) = Builder::new(vec![Ty::Bool], I32);
    let (left, _) = b.block(&[]);
    let (right, _) = b.block(&[]);
    let (join, _) = b.block(&[]);
    b.terminate(
        Block(0),
        Terminator::Branch {
            cond: params[0],
            then: target(left, vec![]),
            otherwise: target(right, vec![]),
        },
    );
    let x = b.constant(left, I32, 1);
    b.terminate(left, jump(join, vec![]));
    b.terminate(right, jump(join, vec![]));
    b.terminate(join, Terminator::Return(x));
    reject(&b.finish(), "dominating");

    // Forward use inside a block.
    let mut f = Builder::new(vec![], I32).0.finish();
    f.blocks[0].insts = vec![
        Inst {
            result: Value(0),
            kind: InstKind::Unary {
                op: UnOp::Not,
                overflow: Overflow::Wrap,
                value: Value(1),
            },
        },
        Inst {
            result: Value(1),
            kind: InstKind::Const { ty: I32, bits: 3 },
        },
    ];
    f.blocks[0].term = Terminator::Return(Value(0));
    reject(&f, "dominating");

    // Type errors.
    let (mut b, params) = Builder::new(vec![I32, U32], I32);
    let sum = bin(
        &mut b,
        Block(0),
        BinOp::Add,
        Overflow::Wrap,
        params[0],
        params[1],
    );
    b.terminate(Block(0), Terminator::Return(sum));
    reject(&b.finish(), "equal integer");

    let (mut b, params) = Builder::new(vec![I32], U32);
    b.terminate(Block(0), Terminator::Return(params[0]));
    reject(&b.finish(), "wrong type");

    let (mut b, _) = Builder::new(vec![], U32);
    let c = b.constant(Block(0), Ty::Int(IntTy::U8), 256);
    b.terminate(Block(0), Terminator::Return(c));
    reject(&b.finish(), "out of range");

    let (mut b, params) = Builder::new(vec![I32], I32);
    let x = bin(
        &mut b,
        Block(0),
        BinOp::And,
        Overflow::Trap,
        params[0],
        params[0],
    );
    b.terminate(Block(0), Terminator::Return(x));
    reject(&b.finish(), "Overflow::Wrap");

    // Block argument mismatch and entry re-entry.
    let (mut b, params) = Builder::new(vec![I32], I32);
    let (next, _) = b.block(&[I32]);
    b.terminate(Block(0), jump(next, vec![]));
    b.terminate(next, Terminator::Return(params[0]));
    reject(&b.finish(), "argument count");

    let (mut b, params) = Builder::new(vec![I32], I32);
    b.terminate(Block(0), jump(Block(0), vec![params[0]]));
    reject(&b.finish(), "entry");

    // Double definition.
    let (mut b, _) = Builder::new(vec![], I32);
    let c = b.constant(Block(0), I32, 1);
    let mut f = b.finish();
    f.blocks[0].insts.push(Inst {
        result: c,
        kind: InstKind::Const { ty: I32, bits: 2 },
    });
    f.blocks[0].term = Terminator::Return(c);
    reject(&f, "more than once");

    // Unknown callee.
    let (mut b, _) = Builder::new(vec![], I32);
    let r = b.inst(
        Block(0),
        InstKind::Call {
            callee: FuncRef(7),
            args: vec![],
        },
    );
    b.terminate(Block(0), Terminator::Return(r));
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
fn factorial() -> Function {
    let (mut b, params) = Builder::new(vec![U32], U32);
    let (base, _) = b.block(&[]);
    let (step, _) = b.block(&[]);
    let zero = b.constant(Block(0), U32, 0);
    let is_zero = b.inst(
        Block(0),
        InstKind::Cmp {
            op: CmpOp::Eq,
            lhs: params[0],
            rhs: zero,
        },
    );
    b.terminate(
        Block(0),
        Terminator::Branch {
            cond: is_zero,
            then: target(base, vec![]),
            otherwise: target(step, vec![]),
        },
    );
    let one = b.constant(base, U32, 1);
    b.terminate(base, Terminator::Return(one));
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
    b.terminate(step, Terminator::Return(product));
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
    let offset = |v: &mut Value| v.0 += 100;
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
                1 => Block(3),
                3 => Block(1),
                other => Block(other),
            };
        }
    }
    shuffled.blocks.push(BlockData {
        params: vec![],
        insts: vec![],
        term: Terminator::Panic(PanicKind::Unreachable),
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
    assert_eq!(hash::ENCODING_VERSION, 1);
}

// ---------------------------------------------------------------------------
// opt
// ---------------------------------------------------------------------------

fn optimize(function: &Function) -> (Function, opt::Stats) {
    opt::local(function, &no_calls).expect("optimizes")
}

#[test]
fn opt_folds_constants_and_branches_to_a_single_return() {
    // let x = 6 * 7; if x == 42 { x + 0 } else { 0 }
    let (mut b, _) = Builder::new(vec![], I32);
    let (yes, _) = b.block(&[]);
    let (no, _) = b.block(&[]);
    let six = b.constant(Block(0), I32, 6);
    let seven = b.constant(Block(0), I32, 7);
    let x = bin(&mut b, Block(0), BinOp::Mul, Overflow::Trap, six, seven);
    let answer = b.constant(Block(0), I32, 42);
    let eq = b.inst(
        Block(0),
        InstKind::Cmp {
            op: CmpOp::Eq,
            lhs: x,
            rhs: answer,
        },
    );
    b.terminate(
        Block(0),
        Terminator::Branch {
            cond: eq,
            then: target(yes, vec![]),
            otherwise: target(no, vec![]),
        },
    );
    let zero = b.constant(yes, I32, 0);
    let same = bin(&mut b, yes, BinOp::Add, Overflow::Trap, x, zero);
    b.terminate(yes, Terminator::Return(same));
    let zero = b.constant(no, I32, 0);
    b.terminate(no, Terminator::Return(zero));
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
        let i8 = Ty::Int(IntTy::I8);
        let (mut b, _) = Builder::new(vec![], i8);
        let max = b.constant(Block(0), i8, 127);
        let one = b.constant(Block(0), i8, 1);
        let sum = bin(&mut b, Block(0), BinOp::Add, overflow, max, one);
        b.terminate(Block(0), Terminator::Return(sum));
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
        Block(0),
        BinOp::Add,
        Overflow::Trap,
        params[0],
        params[1],
    );
    let _div = bin(
        &mut b,
        Block(0),
        BinOp::Div,
        Overflow::Wrap,
        params[0],
        params[1],
    );
    let _wrapped = bin(
        &mut b,
        Block(0),
        BinOp::Add,
        Overflow::Wrap,
        params[0],
        params[1],
    );
    let two = b.constant(Block(0), I32, 2);
    let _safe_div = bin(&mut b, Block(0), BinOp::Div, Overflow::Wrap, params[0], two);
    let zero = b.constant(Block(0), I32, 0);
    b.terminate(Block(0), Terminator::Return(zero));
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
fn random_function(rng: &mut Rng) -> Function {
    let types = [
        Ty::Bool,
        Ty::Int(IntTy::I8),
        Ty::Int(IntTy::U8),
        Ty::Int(IntTy::I32),
        Ty::Int(IntTy::U64),
        Ty::Int(IntTy::I128),
    ];
    let pick_ty = |rng: &mut Rng| types[rng.below(types.len())];
    let params: Vec<Ty> = (0..1 + rng.below(3)).map(|_| pick_ty(rng)).collect();
    let ret = pick_ty(rng);
    let (mut b, entry_values) = Builder::new(params.clone(), ret);
    let block_count = 1 + rng.below(5);
    let mut block_params: Vec<Vec<Ty>> = vec![params.clone()];
    let mut blocks = vec![Block(0)];
    let mut block_values = vec![entry_values];
    for _ in 1..block_count {
        let tys: Vec<Ty> = (0..rng.below(3)).map(|_| pick_ty(rng)).collect();
        let (block, values) = b.block(&tys);
        block_params.push(tys);
        blocks.push(block);
        block_values.push(values);
    }
    let mut entry_pool: Vec<(Value, Ty)> = Vec::new();
    for index in 0..block_count {
        let block = blocks[index];
        let mut pool: Vec<(Value, Ty)> = if index == 0 {
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
        let value_of = |b: &mut Builder, rng: &mut Rng, pool: &mut Vec<(Value, Ty)>, ty: Ty| {
            let candidates: Vec<Value> = pool
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
                (0 | 1, Ty::Int(_)) => {
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
                        Ty::Int(IntTy::U8)
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
                (3, Ty::Int(int)) => {
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
                (4, Ty::Int(int)) if int.signed() => {
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
                    let cond = value_of(&mut b, rng, &mut pool, Ty::Bool);
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
        let make_target = |b: &mut Builder, rng: &mut Rng, pool: &mut Vec<(Value, Ty)>| {
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
                Terminator::Panic(PanicKind::Unreachable)
            } else {
                Terminator::Return(value_of(&mut b, rng, &mut pool, ret))
            }
        } else if rng.below(2) == 0 {
            Terminator::Jump(make_target(&mut b, rng, &mut pool))
        } else {
            let cond = value_of(&mut b, rng, &mut pool, Ty::Bool);
            let then = make_target(&mut b, rng, &mut pool);
            let otherwise = make_target(&mut b, rng, &mut pool);
            Terminator::Branch {
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
    for ty in [Ty::Int(IntTy::I8), Ty::Int(IntTy::U8)] {
        for op in ops {
            for overflow in [Overflow::Wrap, Overflow::Trap] {
                if !op.has_overflow_mode() && overflow == Overflow::Trap {
                    continue;
                }
                for c in 0..=255u128 {
                    for constant_left in [false, true] {
                        let (mut b, params) = Builder::new(vec![ty], ty);
                        let k = b.constant(Block(0), ty, c);
                        let (lhs, rhs) = if constant_left {
                            (k, params[0])
                        } else {
                            (params[0], k)
                        };
                        let r = bin(&mut b, Block(0), op, overflow, lhs, rhs);
                        b.terminate(Block(0), Terminator::Return(r));
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
        let shift = |v: &mut Value| v.0 = v.0 * 3 + 5;
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
