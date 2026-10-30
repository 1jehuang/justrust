//! CFG lowering of inherent primitive methods (PRIMITIVE-METHODS.md).
//!
//! Every non-native intrinsic is expanded into existing checked rvalues:
//! comparisons, `+ - *` (which trap on overflow exactly like source arithmetic),
//! integer casts and `Switch` edges. Operands are evaluated once, in order,
//! into snapshots before any expansion, so no operation re-evaluates a source.
//! Only `Rvalue::Intrinsic` rows with `Intrinsic::is_native` reach the backend.
use super::*;
use ir::{CharValue, Intrinsic, IntrinsicOp, ScalarTy};

impl Lowerer<'_> {
    pub(super) fn lower_intrinsic(
        &mut self,
        destination: &Place,
        intrinsic: Intrinsic,
        args: &[ExprId],
        origin: Option<ExprId>,
    ) -> Result<(), String> {
        let mut operands = Vec::with_capacity(args.len());
        for arg in args {
            operands.push(self.expression(*arg)?);
        }
        let diverges = args.iter().any(|arg| {
            self.source
                .exprs
                .get(arg.0)
                .is_some_and(|expr| expr.ty == Ty::Never)
        });
        if intrinsic.is_native() || diverges {
            // A diverged operand already ended this path. Keep one typed row
            // with no expansion, like other operations over Never inputs.
            self.assign(
                destination.clone(),
                Rvalue::Intrinsic {
                    intrinsic,
                    args: operands,
                },
                origin,
            );
            return Ok(());
        }
        let x = operands[0].clone();
        let y = operands.get(1).cloned();
        // Expansion temporaries are born on branch edges but cleaned up with
        // the enclosing scope. Hoist each StorageLive to the expansion entry
        // so every temporary is definitely live on every path to cleanup.
        let entry = self.current;
        let entry_len = self.blocks[entry.0].statements.len();
        let first_block = self.blocks.len();
        let first_local = self.result.locals.len();
        self.expand_intrinsic(destination, intrinsic, x, y, origin)?;
        let fresh = |statement: &Located<Statement>| matches!(statement.value, Statement::StorageLive(local) if local.0 >= first_local);
        let mut hoisted = Vec::new();
        for index in std::iter::once(entry.0).chain(first_block..self.blocks.len()) {
            let start = if index == entry.0 { entry_len } else { 0 };
            let statements = &mut self.blocks[index].statements;
            let mut kept = Vec::with_capacity(statements.len());
            for (position, statement) in statements.drain(..).enumerate() {
                if position >= start && fresh(&statement) {
                    hoisted.push(statement);
                } else {
                    kept.push(statement);
                }
            }
            *statements = kept;
        }
        let statements = &mut self.blocks[entry.0].statements;
        let tail = statements.split_off(entry_len);
        statements.extend(hoisted);
        statements.extend(tail);
        Ok(())
    }

    fn expand_intrinsic(
        &mut self,
        destination: &Place,
        intrinsic: Intrinsic,
        x: Operand,
        y: Option<Operand>,
        origin: Option<ExprId>,
    ) -> Result<(), String> {
        use IntrinsicOp::*;
        match (intrinsic.op, intrinsic.receiver) {
            (RemEuclid, ScalarTy::Int(ty)) => {
                let y = y.ok_or("rem_euclid arity")?;
                // The checked remainder preserves division-by-zero and MIN/-1
                // traps. Never take abs(y): the signed minimum has no positive
                // representation, although its Euclidean remainder is valid.
                let rem = self.arith(ty, BinaryOp::Rem, x, y.clone(), origin);
                if !ty.is_signed() {
                    self.assign(destination.clone(), Rvalue::Use(rem), origin);
                } else {
                    let negative = self.compare(BinaryOp::Lt, rem.clone(), int(ty, 0), origin);
                    let adjust = self.block();
                    let keep = self.block();
                    let join = self.block();
                    self.branch(negative, adjust, keep, origin)?;
                    self.current = keep;
                    self.assign(destination.clone(), Rvalue::Use(rem.clone()), origin);
                    self.terminate(Terminator::Goto(join), origin)?;
                    self.current = adjust;
                    let divisor_negative =
                        self.compare(BinaryOp::Lt, y.clone(), int(ty, 0), origin);
                    let subtract = self.block();
                    let add = self.block();
                    self.branch(divisor_negative, subtract, add, origin)?;
                    self.current = subtract;
                    let adjusted = self.arith(ty, BinaryOp::Sub, rem.clone(), y.clone(), origin);
                    self.assign(destination.clone(), Rvalue::Use(adjusted), origin);
                    self.terminate(Terminator::Goto(join), origin)?;
                    self.current = add;
                    let adjusted = self.arith(ty, BinaryOp::Add, rem, y, origin);
                    self.assign(destination.clone(), Rvalue::Use(adjusted), origin);
                    self.terminate(Terminator::Goto(join), origin)?;
                    self.current = join;
                }
            }
            (Min | Max | FloatMin | FloatMax, ScalarTy::Int(_)) => {
                let y = y.ok_or("min/max arity")?;
                let op = if matches!(intrinsic.op, Min) {
                    BinaryOp::Le
                } else {
                    BinaryOp::Ge
                };
                // Ord::min returns self on ties, Ord::max returns other.
                let op = if matches!(intrinsic.op, Max) {
                    BinaryOp::Gt
                } else {
                    op
                };
                let keep = self.compare(op, x.clone(), y.clone(), origin);
                self.select(destination, keep, x, y, origin)?;
            }
            (FloatMin | FloatMax, ScalarTy::Float(ty)) => {
                // IEEE minNum/maxNum as f64::min/max: a NaN operand yields the other.
                let y = y.ok_or("min/max arity")?;
                let op = if matches!(intrinsic.op, FloatMin) {
                    BinaryOp::Lt
                } else {
                    BinaryOp::Gt
                };
                let x_nan = self.compare(BinaryOp::Ne, x.clone(), x.clone(), origin);
                let pick_y = self.block();
                let compare = self.block();
                let join = self.block();
                self.branch(x_nan, pick_y, compare, origin)?;
                self.current = compare;
                let ordered = self.compare(op, x.clone(), y.clone(), origin);
                let y_nan = self.compare(BinaryOp::Ne, y.clone(), y.clone(), origin);
                let keep = self.temporary(Ty::Bool, origin);
                self.assign(
                    keep.clone(),
                    Rvalue::Binary {
                        op: BinaryOp::BitOr,
                        left: ordered,
                        right: y_nan,
                    },
                    origin,
                );
                let pick_x = self.block();
                self.branch(Operand::Copy(keep), pick_x, pick_y, origin)?;
                self.current = pick_x;
                self.assign(destination.clone(), Rvalue::Use(x), origin);
                self.terminate(Terminator::Goto(join), origin)?;
                self.current = pick_y;
                // Equal non-NaN values, or NaN self: return other. For equal
                // values of different sign stock may return either zero.
                let _ = ty;
                self.assign(destination.clone(), Rvalue::Use(y), origin);
                self.terminate(Terminator::Goto(join), origin)?;
                self.current = join;
            }
            (Abs, ScalarTy::Int(ty)) => {
                // Negation traps on MIN exactly like source `-x` (debug overflow).
                let negative = self.compare(BinaryOp::Lt, x.clone(), int(ty, 0), origin);
                let negated = self.temporary(Ty::Int(ty), origin);
                let yes = self.block();
                let no = self.block();
                let join = self.block();
                self.branch(negative, yes, no, origin)?;
                self.current = yes;
                self.assign(
                    negated.clone(),
                    Rvalue::Unary {
                        op: UnaryOp::Neg,
                        value: x.clone(),
                    },
                    origin,
                );
                self.assign(
                    destination.clone(),
                    Rvalue::Use(Operand::Copy(negated)),
                    origin,
                );
                self.terminate(Terminator::Goto(join), origin)?;
                self.current = no;
                self.assign(destination.clone(), Rvalue::Use(x), origin);
                self.terminate(Terminator::Goto(join), origin)?;
                self.current = join;
            }
            (Signum, ScalarTy::Int(ty)) => {
                let negative = self.compare(BinaryOp::Lt, x.clone(), int(ty, 0), origin);
                let positive = self.compare(BinaryOp::Gt, x, int(ty, 0), origin);
                let inner = self.temporary(Ty::Int(ty), origin);
                self.select(&inner, positive, int(ty, 1), int(ty, 0), origin)?;
                self.select(
                    destination,
                    negative,
                    int(ty, ty.mask()),
                    Operand::Copy(inner),
                    origin,
                )?;
            }
            (IsPositive, ScalarTy::Int(ty)) => {
                let value = self.compare_rvalue(BinaryOp::Gt, x, int(ty, 0));
                self.assign(destination.clone(), value, origin);
            }
            (IsNegative, ScalarTy::Int(ty)) => {
                let value = self.compare_rvalue(BinaryOp::Lt, x, int(ty, 0));
                self.assign(destination.clone(), value, origin);
            }
            (IsPowerOfTwo, ScalarTy::Int(ty)) => {
                // x != 0 && x & (x - 1) == 0, without underflow at zero.
                let nonzero = self.compare(BinaryOp::Ne, x.clone(), int(ty, 0), origin);
                let yes = self.block();
                let no = self.block();
                let join = self.block();
                self.branch(nonzero, yes, no, origin)?;
                self.current = yes;
                let less = self.arith(ty, BinaryOp::Sub, x.clone(), int(ty, 1), origin);
                let masked = self.arith(ty, BinaryOp::BitAnd, x, less, origin);
                let value = self.compare_rvalue(BinaryOp::Eq, masked, int(ty, 0));
                self.assign(destination.clone(), value, origin);
                self.terminate(Terminator::Goto(join), origin)?;
                self.current = no;
                self.assign(destination.clone(), bool_value(false), origin);
                self.terminate(Terminator::Goto(join), origin)?;
                self.current = join;
            }
            (Pow, ScalarTy::Int(ty)) => {
                // Square-and-multiply over checked `*`: any overflowing
                // intermediate square is also needed by stock's algorithm only
                // when a later bit remains, so trap exactly when stock does.
                let exp = y.ok_or("pow arity")?;
                let u32_ty = IntTy::U32;
                let acc = self.temporary(Ty::Int(ty), origin);
                let base = self.temporary(Ty::Int(ty), origin);
                let e = self.temporary(Ty::Int(u32_ty), origin);
                self.assign(acc.clone(), Rvalue::Use(int(ty, 1)), origin);
                self.assign(base.clone(), Rvalue::Use(x), origin);
                self.assign(e.clone(), Rvalue::Use(exp), origin);
                let head = self.block();
                let body = self.block();
                let exit = self.block();
                self.terminate(Terminator::Goto(head), origin)?;
                self.current = head;
                let more = self.compare(
                    BinaryOp::Gt,
                    Operand::Copy(e.clone()),
                    int(u32_ty, 0),
                    origin,
                );
                self.branch(more, body, exit, origin)?;
                self.current = body;
                let low = self.arith(
                    u32_ty,
                    BinaryOp::BitAnd,
                    Operand::Copy(e.clone()),
                    int(u32_ty, 1),
                    origin,
                );
                let odd = self.compare(BinaryOp::Ne, low, int(u32_ty, 0), origin);
                let multiply = self.block();
                let after = self.block();
                self.branch(odd, multiply, after, origin)?;
                self.current = multiply;
                let product = self.arith(
                    ty,
                    BinaryOp::Mul,
                    Operand::Copy(acc.clone()),
                    Operand::Copy(base.clone()),
                    origin,
                );
                self.assign(acc.clone(), Rvalue::Use(product), origin);
                self.terminate(Terminator::Goto(after), origin)?;
                self.current = after;
                let shifted = self.arith(
                    u32_ty,
                    BinaryOp::Shr,
                    Operand::Copy(e.clone()),
                    int(u32_ty, 1),
                    origin,
                );
                self.assign(e.clone(), Rvalue::Use(shifted), origin);
                let remaining = self.compare(
                    BinaryOp::Gt,
                    Operand::Copy(e.clone()),
                    int(u32_ty, 0),
                    origin,
                );
                let square = self.block();
                self.branch(remaining, square, head, origin)?;
                self.current = square;
                let squared = self.arith(
                    ty,
                    BinaryOp::Mul,
                    Operand::Copy(base.clone()),
                    Operand::Copy(base.clone()),
                    origin,
                );
                self.assign(base.clone(), Rvalue::Use(squared), origin);
                self.terminate(Terminator::Goto(head), origin)?;
                self.current = exit;
                self.assign(destination.clone(), Rvalue::Use(Operand::Copy(acc)), origin);
            }
            (
                WrappingAdd | WrappingSub | WrappingMul | SaturatingAdd | SaturatingSub,
                ScalarTy::Int(ty),
            ) => {
                // Exact in i128/u128 (operands are at most 64 bits wide), then
                // truncate or clamp back to the declared width.
                let y = y.ok_or("wrapping arity")?;
                let wide = if ty.is_signed() {
                    IntTy::I128
                } else {
                    IntTy::U128
                };
                let a = self.cast(x, wide, origin);
                let b = self.cast(y, wide, origin);
                let op = match intrinsic.op {
                    WrappingAdd | SaturatingAdd => BinaryOp::Add,
                    WrappingSub | SaturatingSub => BinaryOp::Sub,
                    _ => BinaryOp::Mul,
                };
                if matches!(intrinsic.op, WrappingMul) && !ty.is_signed() {
                    // u64 * u64 can exceed u128? No: < 2^128. Exact.
                }
                if matches!(intrinsic.op, SaturatingSub) && !ty.is_signed() {
                    // Unsigned saturating_sub: a < b gives 0, never underflows u128.
                    let below = self.compare(BinaryOp::Lt, a.clone(), b.clone(), origin);
                    let yes = self.block();
                    let no = self.block();
                    let join = self.block();
                    self.branch(below, yes, no, origin)?;
                    self.current = yes;
                    self.assign(destination.clone(), Rvalue::Use(int(ty, 0)), origin);
                    self.terminate(Terminator::Goto(join), origin)?;
                    self.current = no;
                    let diff = self.arith(wide, BinaryOp::Sub, a, b, origin);
                    self.assign(
                        destination.clone(),
                        Rvalue::IntCast {
                            value: diff,
                            to: ty,
                        },
                        origin,
                    );
                    self.terminate(Terminator::Goto(join), origin)?;
                    self.current = join;
                    return Ok(());
                }
                if matches!(intrinsic.op, WrappingSub) && !ty.is_signed() {
                    // a - b mod 2^n == (a + (2^n - b)) mod 2^n, staying nonnegative.
                    let modulus = int(wide, ty.mask());
                    let complement = self.arith(wide, BinaryOp::Sub, modulus, b, origin);
                    let shifted = self.arith(wide, BinaryOp::Add, a, complement, origin);
                    let total = self.arith(wide, BinaryOp::Add, shifted, int(wide, 1), origin);
                    self.assign(
                        destination.clone(),
                        Rvalue::IntCast {
                            value: total,
                            to: ty,
                        },
                        origin,
                    );
                    return Ok(());
                }
                let exact = self.arith(wide, op, a, b, origin);
                if matches!(intrinsic.op, WrappingAdd | WrappingSub | WrappingMul) {
                    // `as` truncation is exactly two's-complement wrapping.
                    self.assign(
                        destination.clone(),
                        Rvalue::IntCast {
                            value: exact,
                            to: ty,
                        },
                        origin,
                    );
                    return Ok(());
                }
                let (min, max) = int_bounds(ty);
                let low = self.compare(BinaryOp::Lt, exact.clone(), widen(wide, ty, min), origin);
                let high = self.compare(BinaryOp::Gt, exact.clone(), widen(wide, ty, max), origin);
                let narrowed = self.temporary(Ty::Int(ty), origin);
                self.assign(
                    narrowed.clone(),
                    Rvalue::IntCast {
                        value: exact,
                        to: ty,
                    },
                    origin,
                );
                let upper = self.temporary(Ty::Int(ty), origin);
                self.select(&upper, high, int(ty, max), Operand::Copy(narrowed), origin)?;
                self.select(destination, low, int(ty, min), Operand::Copy(upper), origin)?;
            }
            (WrappingNeg, ScalarTy::Int(ty)) => {
                // 0 - x in i128, truncated: MIN wraps to MIN.
                let wide = IntTy::I128;
                let a = self.cast(x, wide, origin);
                let exact = self.arith(wide, BinaryOp::Sub, int(wide, 0), a, origin);
                self.assign(
                    destination.clone(),
                    Rvalue::IntCast {
                        value: exact,
                        to: ty,
                    },
                    origin,
                );
            }
            (CountOnes | CountZeros | TrailingZeros, ScalarTy::Int(ty)) => {
                // Bounded bit loop over the unsigned image: n iterations.
                let unsigned = ty.unsigned();
                let bits = self.temporary(Ty::Int(unsigned), origin);
                self.assign(
                    bits.clone(),
                    Rvalue::IntCast {
                        value: x,
                        to: unsigned,
                    },
                    origin,
                );
                let count = self.temporary(Ty::Int(IntTy::U32), origin);
                let index = self.temporary(Ty::Int(IntTy::U32), origin);
                self.assign(count.clone(), Rvalue::Use(int(IntTy::U32, 0)), origin);
                self.assign(index.clone(), Rvalue::Use(int(IntTy::U32, 0)), origin);
                let head = self.block();
                let body = self.block();
                let exit = self.block();
                self.terminate(Terminator::Goto(head), origin)?;
                self.current = head;
                let more = self.compare(
                    BinaryOp::Lt,
                    Operand::Copy(index.clone()),
                    int(IntTy::U32, u128::from(ty.bits())),
                    origin,
                );
                self.branch(more, body, exit, origin)?;
                self.current = body;
                let shifted = self.arith(
                    unsigned,
                    BinaryOp::Shr,
                    Operand::Copy(bits.clone()),
                    Operand::Copy(index.clone()),
                    origin,
                );
                let low = self.arith(
                    unsigned,
                    BinaryOp::BitAnd,
                    shifted,
                    int(unsigned, 1),
                    origin,
                );
                let set = self.compare(BinaryOp::Ne, low, int(unsigned, 0), origin);
                let counted = self.block();
                let next = self.block();
                match intrinsic.op {
                    CountOnes => self.branch(set, counted, next, origin)?,
                    CountZeros => self.branch(set, next, counted, origin)?,
                    // A set bit ends the scan.
                    _ => self.branch(set, exit, counted, origin)?,
                }
                self.current = counted;
                let plus = self.arith(
                    IntTy::U32,
                    BinaryOp::Add,
                    Operand::Copy(count.clone()),
                    int(IntTy::U32, 1),
                    origin,
                );
                self.assign(count.clone(), Rvalue::Use(plus), origin);
                self.terminate(Terminator::Goto(next), origin)?;
                self.current = next;
                let step = self.arith(
                    IntTy::U32,
                    BinaryOp::Add,
                    Operand::Copy(index.clone()),
                    int(IntTy::U32, 1),
                    origin,
                );
                self.assign(index.clone(), Rvalue::Use(step), origin);
                self.terminate(Terminator::Goto(head), origin)?;
                self.current = exit;
                self.assign(
                    destination.clone(),
                    Rvalue::Use(Operand::Copy(count)),
                    origin,
                );
            }
            (op, ScalarTy::Char) => self.lower_char_intrinsic(destination, op, x, origin)?,
            (IsNan, ScalarTy::Float(_)) => {
                let value = self.compare_rvalue(BinaryOp::Ne, x.clone(), x);
                self.assign(destination.clone(), value, origin);
            }
            (IsFinite | IsInfinite, ScalarTy::Float(ty)) => {
                // |x| < inf, and |x| == inf, with the native exact abs kernel.
                let magnitude = self.temporary(Ty::Float(ty), origin);
                self.assign(
                    magnitude.clone(),
                    Rvalue::Intrinsic {
                        intrinsic: Intrinsic {
                            op: FloatAbs,
                            receiver: ScalarTy::Float(ty),
                        },
                        args: vec![x],
                    },
                    origin,
                );
                let infinity = Operand::Constant(Constant::Float(match ty {
                    ir::FloatTy::F64 => ir::FloatValue::f64(f64::INFINITY),
                    ir::FloatTy::F32 => ir::FloatValue::f32(f32::INFINITY),
                }));
                let op = if matches!(intrinsic.op, IsFinite) {
                    BinaryOp::Lt
                } else {
                    BinaryOp::Eq
                };
                let value = self.compare_rvalue(op, Operand::Copy(magnitude), infinity);
                self.assign(destination.clone(), value, origin);
            }
            _ => return Err("primitive method has no lowering for its receiver".into()),
        }
        Ok(())
    }

    fn lower_char_intrinsic(
        &mut self,
        destination: &Place,
        op: IntrinsicOp,
        c: Operand,
        origin: Option<ExprId>,
    ) -> Result<(), String> {
        use IntrinsicOp::*;
        let ranges: &[(u32, u32)] = match op {
            IsAscii => &[(0, 0x7f)],
            IsAsciiDigit => &[(0x30, 0x39)],
            IsAsciiHexdigit => &[(0x30, 0x39), (0x41, 0x46), (0x61, 0x66)],
            IsAsciiAlphabetic => &[(0x41, 0x5a), (0x61, 0x7a)],
            IsAsciiAlphanumeric => &[(0x30, 0x39), (0x41, 0x5a), (0x61, 0x7a)],
            IsAsciiUppercase => &[(0x41, 0x5a)],
            IsAsciiLowercase => &[(0x61, 0x7a)],
            // Rust's ASCII whitespace excludes U+000B.
            IsAsciiWhitespace => &[(0x09, 0x0a), (0x0c, 0x0d), (0x20, 0x20)],
            IsAsciiPunctuation => &[(0x21, 0x2f), (0x3a, 0x40), (0x5b, 0x60), (0x7b, 0x7e)],
            IsAsciiGraphic => &[(0x21, 0x7e)],
            IsAsciiControl => &[(0x00, 0x1f), (0x7f, 0x7f)],
            // Unicode White_Space (char::is_whitespace).
            IsWhitespace => &[
                (0x09, 0x0d),
                (0x20, 0x20),
                (0x85, 0x85),
                (0xa0, 0xa0),
                (0x1680, 0x1680),
                (0x2000, 0x200a),
                (0x2028, 0x2029),
                (0x202f, 0x202f),
                (0x205f, 0x205f),
                (0x3000, 0x3000),
            ],
            // General_Category Cc.
            IsControl => &[(0x00, 0x1f), (0x7f, 0x9f)],
            ToAsciiUppercase | ToAsciiLowercase => {
                let (lo, hi, delta_up) = if matches!(op, ToAsciiUppercase) {
                    (0x61, 0x7a, false)
                } else {
                    (0x41, 0x5a, true)
                };
                let inside = self.in_ranges(&c, &[(lo, hi)], origin)?;
                let yes = self.block();
                let no = self.block();
                let join = self.block();
                self.branch(inside, yes, no, origin)?;
                self.current = yes;
                // ASCII letters only: the shifted value stays an ASCII u8 scalar.
                let code = self.temporary(Ty::Int(IntTy::U8), origin);
                self.assign(
                    code.clone(),
                    Rvalue::CharToInt {
                        value: c.clone(),
                        to: IntTy::U8,
                    },
                    origin,
                );
                let shifted = self.arith(
                    IntTy::U8,
                    if delta_up {
                        BinaryOp::Add
                    } else {
                        BinaryOp::Sub
                    },
                    Operand::Copy(code),
                    int(IntTy::U8, 0x20),
                    origin,
                );
                self.assign(
                    destination.clone(),
                    Rvalue::U8ToChar { value: shifted },
                    origin,
                );
                self.terminate(Terminator::Goto(join), origin)?;
                self.current = no;
                self.assign(destination.clone(), Rvalue::Use(c), origin);
                self.terminate(Terminator::Goto(join), origin)?;
                self.current = join;
                return Ok(());
            }
            _ => return Err("character primitive method lowering invariant".into()),
        };
        let inside = self.in_ranges(&c, ranges, origin)?;
        self.assign(destination.clone(), Rvalue::Use(inside), origin);
        Ok(())
    }

    /// Bool operand: `c` lies in any of `ranges` (inclusive scalar bounds).
    fn in_ranges(
        &mut self,
        c: &Operand,
        ranges: &[(u32, u32)],
        origin: Option<ExprId>,
    ) -> Result<Operand, String> {
        let result = self.temporary(Ty::Bool, origin);
        self.assign(result.clone(), bool_value(false), origin);
        let join = self.block();
        for &(lo, hi) in ranges {
            let above = self.compare(BinaryOp::Ge, c.clone(), char_value(lo), origin);
            let below = self.compare(BinaryOp::Le, c.clone(), char_value(hi), origin);
            let both = self.temporary(Ty::Bool, origin);
            self.assign(
                both.clone(),
                Rvalue::Binary {
                    op: BinaryOp::BitAnd,
                    left: above,
                    right: below,
                },
                origin,
            );
            let hit = self.block();
            let next = self.block();
            self.branch(Operand::Copy(both), hit, next, origin)?;
            self.current = hit;
            self.assign(result.clone(), bool_value(true), origin);
            self.terminate(Terminator::Goto(join), origin)?;
            self.current = next;
        }
        self.terminate(Terminator::Goto(join), origin)?;
        self.current = join;
        Ok(Operand::Copy(result))
    }

    fn compare_rvalue(&self, op: BinaryOp, left: Operand, right: Operand) -> Rvalue {
        Rvalue::Binary { op, left, right }
    }

    fn compare(
        &mut self,
        op: BinaryOp,
        left: Operand,
        right: Operand,
        origin: Option<ExprId>,
    ) -> Operand {
        let local = self.temporary(Ty::Bool, origin);
        self.assign(local.clone(), Rvalue::Binary { op, left, right }, origin);
        Operand::Copy(local)
    }

    fn arith(
        &mut self,
        ty: IntTy,
        op: BinaryOp,
        left: Operand,
        right: Operand,
        origin: Option<ExprId>,
    ) -> Operand {
        let local = self.temporary(Ty::Int(ty), origin);
        self.assign(local.clone(), Rvalue::Binary { op, left, right }, origin);
        Operand::Copy(local)
    }

    fn cast(&mut self, value: Operand, to: IntTy, origin: Option<ExprId>) -> Operand {
        let local = self.temporary(Ty::Int(to), origin);
        self.assign(local.clone(), Rvalue::IntCast { value, to }, origin);
        Operand::Copy(local)
    }

    fn branch(
        &mut self,
        condition: Operand,
        then_block: BlockId,
        else_block: BlockId,
        origin: Option<ExprId>,
    ) -> Result<(), String> {
        self.terminate(
            Terminator::Switch {
                condition,
                then_block,
                else_block,
            },
            origin,
        )
    }

    /// destination = if condition { yes } else { no }
    fn select(
        &mut self,
        destination: &Place,
        condition: Operand,
        yes: Operand,
        no: Operand,
        origin: Option<ExprId>,
    ) -> Result<(), String> {
        let then_block = self.block();
        let else_block = self.block();
        let join = self.block();
        self.branch(condition, then_block, else_block, origin)?;
        self.current = then_block;
        self.assign(destination.clone(), Rvalue::Use(yes), origin);
        self.terminate(Terminator::Goto(join), origin)?;
        self.current = else_block;
        self.assign(destination.clone(), Rvalue::Use(no), origin);
        self.terminate(Terminator::Goto(join), origin)?;
        self.current = join;
        Ok(())
    }
}

fn int(ty: IntTy, bits: u128) -> Operand {
    Operand::Constant(Constant::Int(IntValue {
        ty,
        bits: bits & ty.mask(),
    }))
}

fn bool_value(value: bool) -> Rvalue {
    Rvalue::Use(Operand::Constant(Constant::Bool(value)))
}

fn char_value(value: u32) -> Operand {
    Operand::Constant(Constant::Char(CharValue { value }))
}

/// (MIN, MAX) bit patterns at the declared width.
fn int_bounds(ty: IntTy) -> (u128, u128) {
    if ty.is_signed() {
        let min = 1u128 << (ty.bits() - 1);
        (min, min - 1)
    } else {
        (0, ty.mask())
    }
}

/// Sign- or zero-extend a declared-width bit pattern into the wide type.
fn widen(wide: IntTy, ty: IntTy, bits: u128) -> Operand {
    let value = if ty.is_signed() && bits & (1u128 << (ty.bits() - 1)) != 0 {
        bits | !ty.mask()
    } else {
        bits
    };
    int(wide, value)
}
