//! Inherent primitive methods with exact native semantics (PRIMITIVE-METHODS.md).
//!
//! One closed table maps `(receiver scalar type, method name)` to an operation
//! and its exact signature. Type checking, inference hints, IR validation, CFG
//! validation and the backend all derive parameter and result types from
//! [`Intrinsic::signature`], so no layer can disagree about an operation's type.
use super::{FloatTy, IntTy, Ty};

/// Scalar receiver domain of a primitive method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScalarTy {
    Int(IntTy),
    Float(FloatTy),
    Char,
}

impl ScalarTy {
    pub fn of(ty: &Ty) -> Option<Self> {
        match ty {
            Ty::Int(t) => Some(Self::Int(*t)),
            Ty::Float(t) => Some(Self::Float(*t)),
            Ty::Char => Some(Self::Char),
            _ => None,
        }
    }
    pub fn ty(self) -> Ty {
        match self {
            Self::Int(t) => Ty::Int(t),
            Self::Float(t) => Ty::Float(t),
            Self::Char => Ty::Char,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntrinsicOp {
    // Integers. Arithmetic overflow traps exactly like `+`, `-` and `*`.
    Abs,
    Signum,
    Pow,
    Min,
    Max,
    WrappingAdd,
    WrappingSub,
    WrappingMul,
    WrappingNeg,
    SaturatingAdd,
    SaturatingSub,
    CountOnes,
    CountZeros,
    TrailingZeros,
    IsPowerOfTwo,
    IsPositive,
    IsNegative,
    // Characters (ASCII classification and case mapping, plus exact
    // Unicode White_Space and control-character sets).
    IsAscii,
    IsAsciiDigit,
    IsAsciiHexdigit,
    IsAsciiAlphabetic,
    IsAsciiAlphanumeric,
    IsAsciiUppercase,
    IsAsciiLowercase,
    IsAsciiWhitespace,
    IsAsciiPunctuation,
    IsAsciiGraphic,
    IsAsciiControl,
    ToAsciiUppercase,
    ToAsciiLowercase,
    IsWhitespace,
    IsControl,
    // Floats.
    Sqrt,
    FloatAbs,
    FloatMin,
    FloatMax,
    IsNan,
    IsFinite,
    IsInfinite,
    IsSignNegative,
    IsSignPositive,
}

/// A primitive method resolved against its exact receiver scalar type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Intrinsic {
    pub op: IntrinsicOp,
    pub receiver: ScalarTy,
}

/// Exact parameter list (receiver first) and result of an intrinsic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntrinsicSignature {
    pub params: Vec<Ty>,
    pub result: Ty,
}

impl IntrinsicOp {
    /// Method spellings admitted for some scalar type. A syntactic candidate
    /// only: [`Intrinsic::resolve`] alone decides from the typed receiver.
    pub fn candidate_name(name: &str) -> bool {
        Self::by_name(name).next().is_some()
    }
    fn by_name(name: &str) -> impl Iterator<Item = Self> + '_ {
        TABLE
            .iter()
            .filter(move |(spelling, _)| *spelling == name)
            .map(|(_, op)| *op)
    }
}

const TABLE: &[(&str, IntrinsicOp)] = {
    use IntrinsicOp::*;
    &[
        ("abs", Abs),
        ("abs", FloatAbs),
        ("signum", Signum),
        ("pow", Pow),
        ("min", Min),
        ("min", FloatMin),
        ("max", Max),
        ("max", FloatMax),
        ("wrapping_add", WrappingAdd),
        ("wrapping_sub", WrappingSub),
        ("wrapping_mul", WrappingMul),
        ("wrapping_neg", WrappingNeg),
        ("saturating_add", SaturatingAdd),
        ("saturating_sub", SaturatingSub),
        ("count_ones", CountOnes),
        ("count_zeros", CountZeros),
        ("trailing_zeros", TrailingZeros),
        ("is_power_of_two", IsPowerOfTwo),
        ("is_positive", IsPositive),
        ("is_negative", IsNegative),
        ("is_ascii", IsAscii),
        ("is_ascii_digit", IsAsciiDigit),
        ("is_ascii_hexdigit", IsAsciiHexdigit),
        ("is_ascii_alphabetic", IsAsciiAlphabetic),
        ("is_ascii_alphanumeric", IsAsciiAlphanumeric),
        ("is_ascii_uppercase", IsAsciiUppercase),
        ("is_ascii_lowercase", IsAsciiLowercase),
        ("is_ascii_whitespace", IsAsciiWhitespace),
        ("is_ascii_punctuation", IsAsciiPunctuation),
        ("is_ascii_graphic", IsAsciiGraphic),
        ("is_ascii_control", IsAsciiControl),
        ("to_ascii_uppercase", ToAsciiUppercase),
        ("to_ascii_lowercase", ToAsciiLowercase),
        ("is_whitespace", IsWhitespace),
        ("is_control", IsControl),
        ("sqrt", Sqrt),
        ("is_nan", IsNan),
        ("is_finite", IsFinite),
        ("is_infinite", IsInfinite),
        ("is_sign_negative", IsSignNegative),
        ("is_sign_positive", IsSignPositive),
    ]
};

impl Intrinsic {
    /// Operations with a dedicated native kernel. Every other intrinsic is
    /// lowered by the CFG into existing checked comparisons and arithmetic.
    pub fn is_native(self) -> bool {
        use IntrinsicOp::*;
        matches!(self.op, Sqrt | FloatAbs | IsSignNegative | IsSignPositive)
    }

    /// Wrapping and saturating forms widen to 128 bits, so 128-bit receivers
    /// have no wider exact domain yet.
    pub fn unsupported_reason(self) -> Option<&'static str> {
        use IntrinsicOp::*;
        match (self.op, self.receiver) {
            (
                WrappingAdd | WrappingSub | WrappingMul | WrappingNeg | SaturatingAdd
                | SaturatingSub,
                ScalarTy::Int(t),
            ) if t.bits() == 128 => {
                Some("128-bit wrapping and saturating primitive methods are unsupported")
            }
            _ => None,
        }
    }

    /// Resolve `receiver.name(..)` for an exact scalar receiver type.
    pub fn resolve(receiver: ScalarTy, name: &str) -> Option<Self> {
        IntrinsicOp::by_name(name)
            .map(|op| Self { op, receiver })
            .find(|intrinsic| intrinsic.signature().is_some())
    }

    /// The exact signature, or None when the receiver type has no such method.
    pub fn signature(self) -> Option<IntrinsicSignature> {
        use IntrinsicOp::*;
        let this = self.receiver.ty();
        let (params, result) = match (self.op, self.receiver) {
            (Abs | Signum | WrappingNeg, ScalarTy::Int(t)) if t.is_signed() => (vec![], this),
            (IsPositive | IsNegative, ScalarTy::Int(t)) if t.is_signed() => (vec![], Ty::Bool),
            (IsPowerOfTwo, ScalarTy::Int(t)) if !t.is_signed() => (vec![], Ty::Bool),
            (Pow, ScalarTy::Int(_)) => (vec![Ty::Int(IntTy::U32)], this),
            (
                Min | Max | WrappingAdd | WrappingSub | WrappingMul | SaturatingAdd | SaturatingSub,
                ScalarTy::Int(_),
            ) => (vec![this.clone()], this),
            (CountOnes | CountZeros | TrailingZeros, ScalarTy::Int(_)) => {
                (vec![], Ty::Int(IntTy::U32))
            }
            (
                IsAscii | IsAsciiDigit | IsAsciiHexdigit | IsAsciiAlphabetic | IsAsciiAlphanumeric
                | IsAsciiUppercase | IsAsciiLowercase | IsAsciiWhitespace | IsAsciiPunctuation
                | IsAsciiGraphic | IsAsciiControl | IsWhitespace | IsControl,
                ScalarTy::Char,
            ) => (vec![], Ty::Bool),
            (ToAsciiUppercase | ToAsciiLowercase, ScalarTy::Char) => (vec![], Ty::Char),
            (Sqrt | FloatAbs, ScalarTy::Float(_)) => (vec![], this),
            (FloatMin | FloatMax, ScalarTy::Float(_)) => (vec![this.clone()], this),
            (
                IsNan | IsFinite | IsInfinite | IsSignNegative | IsSignPositive,
                ScalarTy::Float(_),
            ) => (vec![], Ty::Bool),
            _ => return None,
        };
        let mut all = Vec::with_capacity(params.len() + 1);
        all.push(self.receiver.ty());
        all.extend(params);
        Some(IntrinsicSignature {
            params: all,
            result,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_table_row_resolves_for_some_scalar_and_signatures_are_exact() {
        let scalars = [
            ScalarTy::Int(IntTy::I8),
            ScalarTy::Int(IntTy::U64),
            ScalarTy::Int(IntTy::I128),
            ScalarTy::Float(FloatTy::F32),
            ScalarTy::Float(FloatTy::F64),
            ScalarTy::Char,
        ];
        for (name, op) in TABLE {
            assert!(
                scalars
                    .iter()
                    .any(|s| Intrinsic::resolve(*s, name).is_some_and(|i| i.op == *op)),
                "{name} {op:?} unreachable"
            );
        }
        let abs = Intrinsic::resolve(ScalarTy::Int(IntTy::I32), "abs").unwrap();
        assert_eq!(abs.op, IntrinsicOp::Abs);
        assert!(Intrinsic::resolve(ScalarTy::Int(IntTy::U32), "abs").is_none());
        let fabs = Intrinsic::resolve(ScalarTy::Float(FloatTy::F64), "abs").unwrap();
        assert_eq!(fabs.op, IntrinsicOp::FloatAbs);
        let pow = Intrinsic::resolve(ScalarTy::Int(IntTy::I16), "pow")
            .unwrap()
            .signature()
            .unwrap();
        assert_eq!(pow.params, vec![Ty::Int(IntTy::I16), Ty::Int(IntTy::U32)]);
        assert_eq!(pow.result, Ty::Int(IntTy::I16));
        assert!(Intrinsic::resolve(ScalarTy::Char, "max").is_none());
        assert!(!IntrinsicOp::candidate_name("leading_zeros"));
    }
}
