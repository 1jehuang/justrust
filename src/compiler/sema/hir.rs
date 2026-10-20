//! Resolved body IR consumed by the semantic checker.
//!
//! Names are already resolved: locals are indices, paths are declaration
//! ids, written types are `ty::Ty` over the item's parameters (with `Hole`
//! for `_` or a missing annotation). Producing this from the parsed AST is
//! the frontend adapter's job.

use super::ty::{DefId, Mutbl, Ty};

pub type ExprId = u32;
pub type LocalId = u32;

#[derive(Clone, Debug)]
pub enum HirLit {
    /// Integer literal with optional suffix type.
    Int(u128, Option<super::ty::IntTy>),
    Float(Option<super::ty::FloatTy>),
    Bool,
    Char,
    Str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Deref,
}

#[derive(Clone, Debug)]
pub enum HirPat {
    Wild,
    Binding {
        local: LocalId,
        by_ref: Option<Mutbl>,
    },
    Tuple(Vec<HirPat>),
    /// Struct/variant pattern: adt, variant index, (field index, pattern).
    Adt {
        adt: DefId,
        variant: u32,
        fields: Vec<(u32, HirPat)>,
    },
    Ref(Mutbl, Box<HirPat>),
    Lit(HirLit),
}

#[derive(Clone, Debug)]
pub enum HirStmt {
    Let {
        pat: HirPat,
        ty: Ty,
        init: Option<ExprId>,
        else_block: Option<ExprId>,
    },
    Expr(ExprId),
}

#[derive(Clone, Debug)]
pub struct HirArm {
    pub pat: HirPat,
    pub guard: Option<ExprId>,
    pub body: ExprId,
}

#[derive(Clone, Debug)]
pub enum HirExpr {
    Lit(HirLit),
    Local(LocalId),
    /// Path to a function or associated function, with written type
    /// arguments (`Hole` where omitted). `self_ty` is the written owner for
    /// `Type::item` paths whose owner type has holes (`Vec::new`).
    Fn {
        def: DefId,
        self_ty: Option<Ty>,
        args: Vec<Ty>,
    },
    /// `Type::name` where `name` must be found by inherent/trait lookup.
    AssocFn {
        self_ty: Ty,
        name: String,
    },
    /// Unit/tuple-struct or variant constructor used as a value or callee.
    Ctor {
        adt: DefId,
        variant: u32,
    },
    Call {
        callee: ExprId,
        args: Vec<ExprId>,
    },
    MethodCall {
        receiver: ExprId,
        name: String,
        turbofish: Vec<Ty>,
        args: Vec<ExprId>,
    },
    Field {
        base: ExprId,
        name: String,
    },
    TupleField {
        base: ExprId,
        index: u32,
    },
    Index {
        base: ExprId,
        index: ExprId,
    },
    Struct {
        adt: DefId,
        variant: u32,
        fields: Vec<(u32, ExprId)>,
        base: Option<ExprId>,
    },
    Tuple(Vec<ExprId>),
    Array(Vec<ExprId>),
    Repeat {
        value: ExprId,
        count: u64,
    },
    Unary(UnOp, ExprId),
    Binary(BinOp, ExprId, ExprId),
    AssignOp(BinOp, ExprId, ExprId),
    Assign(ExprId, ExprId),
    Ref(Mutbl, ExprId),
    Cast(ExprId, Ty),
    Block {
        stmts: Vec<HirStmt>,
        tail: Option<ExprId>,
    },
    If {
        cond: ExprId,
        then: ExprId,
        els: Option<ExprId>,
    },
    /// `if let`/`while let` lower to `Match`.
    Match {
        scrutinee: ExprId,
        arms: Vec<HirArm>,
    },
    Loop {
        body: ExprId,
        label: u32,
    },
    While {
        cond: ExprId,
        body: ExprId,
        label: u32,
    },
    For {
        pat: HirPat,
        iter: ExprId,
        body: ExprId,
        label: u32,
    },
    Break {
        label: u32,
        value: Option<ExprId>,
    },
    Continue,
    Return(Option<ExprId>),
    Closure {
        params: Vec<(HirPat, Ty)>,
        ret: Ty,
        body: ExprId,
        id: u32,
    },
    Range {
        start: Option<ExprId>,
        end: Option<ExprId>,
        inclusive: bool,
    },
}

#[derive(Clone, Debug)]
pub struct HirBody {
    pub exprs: Vec<HirExpr>,
    pub locals: u32,
    /// Parameter patterns with their declared types.
    pub params: Vec<(HirPat, Ty)>,
    pub ret: Ty,
    pub root: ExprId,
}
