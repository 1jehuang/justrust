//! The single definition of SSA instruction semantics. Both the interpreter and
//! constant folding call these functions, so a fold can never disagree with
//! execution. Values are raw bit patterns masked to their type's width.
use super::{BinOp, CmpOp, IntTy, Overflow, PanicKind, Ty, UnOp};

/// Sign-extend (signed) or zero-extend (unsigned) a raw pattern to i128 space.
/// For u128 the result is the same bit pattern reinterpreted.
fn widen(int: IntTy, bits: u128) -> i128 {
    let bits = bits & int.mask();
    if int.signed() && int.bits() < 128 {
        let shift = 128 - u32::from(int.bits());
        ((bits << shift) as i128) >> shift
    } else {
        bits as i128
    }
}

fn int_of(ty: Ty) -> Option<IntTy> {
    match ty {
        Ty::Int(int) => Some(int),
        Ty::Bool => Some(IntTy::U8),
        Ty::Unit => None,
    }
}

/// True when the signed mathematical `value` fits in signed `int`.
fn fits_signed(int: IntTy, value: i128) -> bool {
    let bits = u32::from(int.bits());
    if bits == 128 {
        return true;
    }
    let min = -(1i128 << (bits - 1));
    let max = (1i128 << (bits - 1)) - 1;
    value >= min && value <= max
}

fn overflow_panic(op: BinOp) -> PanicKind {
    match op {
        BinOp::Add => PanicKind::AddOverflow,
        BinOp::Sub => PanicKind::SubOverflow,
        BinOp::Mul => PanicKind::MulOverflow,
        BinOp::Shl => PanicKind::ShlOverflow,
        BinOp::Shr => PanicKind::ShrOverflow,
        BinOp::Div => PanicKind::DivOverflow,
        BinOp::Rem => PanicKind::RemOverflow,
        BinOp::And | BinOp::Or | BinOp::Xor => unreachable!("bitwise ops cannot overflow"),
    }
}

/// Add/Sub/Mul: returns the wrapped result and whether it overflowed.
fn arith(op: BinOp, int: IntTy, a: u128, b: u128) -> (u128, bool) {
    let mask = int.mask();
    if int.signed() {
        let (x, y) = (widen(int, a), widen(int, b));
        let (wide, overflowed128) = match op {
            BinOp::Add => x.overflowing_add(y),
            BinOp::Sub => x.overflowing_sub(y),
            BinOp::Mul => x.overflowing_mul(y),
            _ => unreachable!(),
        };
        let overflowed = overflowed128 || !fits_signed(int, wide);
        ((wide as u128) & mask, overflowed)
    } else {
        let (x, y) = (a & mask, b & mask);
        let (wide, overflowed128) = match op {
            BinOp::Add => x.overflowing_add(y),
            BinOp::Sub => x.overflowing_sub(y),
            BinOp::Mul => x.overflowing_mul(y),
            _ => unreachable!(),
        };
        let overflowed = overflowed128 || (int.bits() < 128 && wide > mask);
        (wide & mask, overflowed)
    }
}

/// Evaluate a binary operator. `lhs_ty` is the result type. For shifts the
/// amount has its own integer type `rhs_ty`; otherwise `rhs_ty == lhs_ty`.
pub fn binary(
    op: BinOp,
    overflow: Overflow,
    lhs_ty: Ty,
    a: u128,
    rhs_ty: Ty,
    b: u128,
) -> Result<u128, PanicKind> {
    match op {
        BinOp::And => return Ok(a & b & lhs_ty.max_bits()),
        BinOp::Or => return Ok((a | b) & lhs_ty.max_bits()),
        BinOp::Xor => return Ok((a ^ b) & lhs_ty.max_bits()),
        _ => {}
    }
    let int = int_of(lhs_ty).ok_or(PanicKind::Unreachable)?;
    let mask = int.mask();
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul => {
            let (value, overflowed) = arith(op, int, a, b);
            if overflowed && overflow == Overflow::Trap {
                Err(overflow_panic(op))
            } else {
                Ok(value)
            }
        }
        BinOp::Shl | BinOp::Shr => {
            let amount_int = int_of(rhs_ty).ok_or(PanicKind::Unreachable)?;
            // Rust compares the amount's raw unsigned pattern against the
            // width, so negative amounts overflow.
            let raw = b & amount_int.mask();
            let width = u128::from(int.bits());
            if raw >= width && overflow == Overflow::Trap {
                return Err(overflow_panic(op));
            }
            let amount = (raw & (width - 1)) as u32;
            let value = if op == BinOp::Shl {
                (a << amount) & mask
            } else if int.signed() {
                ((widen(int, a) >> amount) as u128) & mask
            } else {
                (a & mask) >> amount
            };
            Ok(value)
        }
        BinOp::Div | BinOp::Rem => {
            if b & mask == 0 {
                return Err(if op == BinOp::Div {
                    PanicKind::DivByZero
                } else {
                    PanicKind::RemByZero
                });
            }
            if int.signed() {
                let (x, y) = (widen(int, a), widen(int, b));
                let min = if int.bits() == 128 {
                    i128::MIN
                } else {
                    -(1i128 << (int.bits() - 1))
                };
                // Rust panics on MIN / -1 and MIN % -1 regardless of
                // overflow-checks.
                if x == min && y == -1 {
                    return Err(overflow_panic(op));
                }
                let value = if op == BinOp::Div { x / y } else { x % y };
                Ok((value as u128) & mask)
            } else {
                let (x, y) = (a & mask, b & mask);
                Ok(if op == BinOp::Div { x / y } else { x % y })
            }
        }
        BinOp::And | BinOp::Or | BinOp::Xor => unreachable!(),
    }
}

pub fn unary(op: UnOp, overflow: Overflow, ty: Ty, a: u128) -> Result<u128, PanicKind> {
    match op {
        UnOp::Not => Ok(!a & ty.max_bits()),
        UnOp::Neg => {
            let int = int_of(ty).ok_or(PanicKind::Unreachable)?;
            let (value, overflowed) = arith(BinOp::Sub, int, 0, a);
            if overflowed && overflow == Overflow::Trap {
                Err(PanicKind::NegOverflow)
            } else {
                Ok(value)
            }
        }
    }
}

pub fn compare(op: CmpOp, ty: Ty, a: u128, b: u128) -> bool {
    let ordering = match int_of(ty) {
        Some(int) if int.signed() => widen(int, a).cmp(&widen(int, b)),
        Some(int) => (a & int.mask()).cmp(&(b & int.mask())),
        None => std::cmp::Ordering::Equal,
    };
    match op {
        CmpOp::Eq => ordering.is_eq(),
        CmpOp::Ne => ordering.is_ne(),
        CmpOp::Lt => ordering.is_lt(),
        CmpOp::Le => ordering.is_le(),
        CmpOp::Gt => ordering.is_gt(),
        CmpOp::Ge => ordering.is_ge(),
    }
}

/// Rust `as` between integer types, or from bool: extend by the source
/// signedness, then truncate.
pub fn cast(from: Ty, to: IntTy, a: u128) -> u128 {
    let wide = match int_of(from) {
        Some(int) => widen(int, a) as u128,
        None => 0,
    };
    wide & to.mask()
}

/// True when the instruction can never panic for these operands, whatever
/// the overflow mode. Used by DCE to decide removability.
pub fn binary_cannot_panic(
    op: BinOp,
    overflow: Overflow,
    lhs_ty: Ty,
    rhs_ty: Ty,
    rhs: Option<u128>,
) -> bool {
    match op {
        BinOp::And | BinOp::Or | BinOp::Xor => true,
        BinOp::Add | BinOp::Sub | BinOp::Mul => overflow == Overflow::Wrap,
        BinOp::Shl | BinOp::Shr => {
            overflow == Overflow::Wrap
                || match (rhs, int_of(lhs_ty), int_of(rhs_ty)) {
                    (Some(b), Some(int), Some(amount)) => {
                        (b & amount.mask()) < u128::from(int.bits())
                    }
                    _ => false,
                }
        }
        BinOp::Div | BinOp::Rem => match (rhs, int_of(lhs_ty)) {
            (Some(b), Some(int)) => {
                let b = b & int.mask();
                b != 0 && !(int.signed() && b == int.mask())
            }
            _ => false,
        },
    }
}
