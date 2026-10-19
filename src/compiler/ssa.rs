//! SSA core for the graph-native optimizer. See OPTIMIZATION.md.
//!
//! Pure library: every function here is a deterministic computation over its
//! arguments. There is no graph state, shared mutable state, static, lock or
//! global budget. Graph nodes (`SSA`, `LOCAL_OPT`, `FRAGMENT`) will wrap these
//! functions once compiler facts are typed graph outputs (OPTIMIZATION.md 8a).
//!
//! Form: a CFG of blocks with block parameters instead of phi nodes. Block 0 is
//! the entry and its parameters are the function parameters. Every instruction
//! defines exactly one value. Value and block IDs are local to one function and
//! carry no meaning: `hash::canonical` renumbers them, so equal bodies hash
//! equally regardless of construction order.
//!
//! Integer semantics follow Rust: `Overflow::Trap` is the overflow-checks
//! behavior, `Overflow::Wrap` the unchecked one. Division and remainder trap on
//! zero and on signed `MIN / -1` in both modes, as in Rust. Shift amounts may
//! have any integer type. The interpreter (`interp`) and constant folding
//! (`opt`) share one evaluator (`eval`), so folding cannot disagree with it.

pub mod eval;
pub mod hash;
pub mod interp;
pub mod opt;
pub mod verify;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Value(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Block(pub u32);

/// Callee reference. Inside the graph this becomes a canonical item identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FuncRef(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IntTy {
    bits: u8,
    signed: bool,
}

impl IntTy {
    pub const I8: Self = Self::of(8, true);
    pub const I16: Self = Self::of(16, true);
    pub const I32: Self = Self::of(32, true);
    pub const I64: Self = Self::of(64, true);
    pub const I128: Self = Self::of(128, true);
    pub const U8: Self = Self::of(8, false);
    pub const U16: Self = Self::of(16, false);
    pub const U32: Self = Self::of(32, false);
    pub const U64: Self = Self::of(64, false);
    pub const U128: Self = Self::of(128, false);

    const fn of(bits: u8, signed: bool) -> Self {
        Self { bits, signed }
    }

    /// Only Rust's fixed integer widths are representable.
    pub fn new(bits: u8, signed: bool) -> Option<Self> {
        matches!(bits, 8 | 16 | 32 | 64 | 128).then_some(Self { bits, signed })
    }

    pub fn bits(self) -> u8 {
        self.bits
    }

    pub fn signed(self) -> bool {
        self.signed
    }

    pub fn mask(self) -> u128 {
        if self.bits >= 128 {
            u128::MAX
        } else {
            (1u128 << self.bits) - 1
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    Unit,
    Bool,
    Int(IntTy),
}

impl Ty {
    /// Largest valid raw bit pattern for a constant of this type.
    pub fn max_bits(self) -> u128 {
        match self {
            Ty::Unit => 0,
            Ty::Bool => 1,
            Ty::Int(int) => int.mask(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Overflow {
    Wrap,
    Trap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

impl BinOp {
    /// Operators whose result depends on the overflow mode. All others must
    /// use `Overflow::Wrap` in canonical form.
    pub fn has_overflow_mode(self) -> bool {
        matches!(
            self,
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Shl | BinOp::Shr
        )
    }

    pub fn is_shift(self) -> bool {
        matches!(self, BinOp::Shl | BinOp::Shr)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// Runtime panic reasons. Messages match stock Rust.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanicKind {
    AddOverflow,
    SubOverflow,
    MulOverflow,
    NegOverflow,
    ShlOverflow,
    ShrOverflow,
    DivByZero,
    RemByZero,
    DivOverflow,
    RemOverflow,
    Unreachable,
}

impl PanicKind {
    pub fn message(self) -> &'static str {
        match self {
            PanicKind::AddOverflow => "attempt to add with overflow",
            PanicKind::SubOverflow => "attempt to subtract with overflow",
            PanicKind::MulOverflow => "attempt to multiply with overflow",
            PanicKind::NegOverflow => "attempt to negate with overflow",
            PanicKind::ShlOverflow => "attempt to shift left with overflow",
            PanicKind::ShrOverflow => "attempt to shift right with overflow",
            PanicKind::DivByZero => "attempt to divide by zero",
            PanicKind::RemByZero => "attempt to calculate the remainder with a divisor of zero",
            PanicKind::DivOverflow => "attempt to divide with overflow",
            PanicKind::RemOverflow => "attempt to calculate the remainder with overflow",
            PanicKind::Unreachable => "internal error: entered unreachable code",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InstKind {
    /// `bits` is the raw pattern, at most `ty.max_bits()`.
    Const {
        ty: Ty,
        bits: u128,
    },
    Binary {
        op: BinOp,
        overflow: Overflow,
        lhs: Value,
        rhs: Value,
    },
    Unary {
        op: UnOp,
        overflow: Overflow,
        value: Value,
    },
    Cmp {
        op: CmpOp,
        lhs: Value,
        rhs: Value,
    },
    /// Rust `as` from an integer or bool to an integer type.
    Cast {
        to: IntTy,
        value: Value,
    },
    Select {
        cond: Value,
        then: Value,
        otherwise: Value,
    },
    Call {
        callee: FuncRef,
        args: Vec<Value>,
    },
}

impl InstKind {
    /// Operands in a fixed order, used by verification, hashing and DCE.
    pub fn operands(&self) -> Vec<Value> {
        match self {
            InstKind::Const { .. } => Vec::new(),
            InstKind::Binary { lhs, rhs, .. } | InstKind::Cmp { lhs, rhs, .. } => {
                vec![*lhs, *rhs]
            }
            InstKind::Unary { value, .. } | InstKind::Cast { value, .. } => vec![*value],
            InstKind::Select {
                cond,
                then,
                otherwise,
            } => vec![*cond, *then, *otherwise],
            InstKind::Call { args, .. } => args.clone(),
        }
    }

    pub fn operands_mut(&mut self) -> Vec<&mut Value> {
        match self {
            InstKind::Const { .. } => Vec::new(),
            InstKind::Binary { lhs, rhs, .. } | InstKind::Cmp { lhs, rhs, .. } => {
                vec![lhs, rhs]
            }
            InstKind::Unary { value, .. } | InstKind::Cast { value, .. } => vec![value],
            InstKind::Select {
                cond,
                then,
                otherwise,
            } => vec![cond, then, otherwise],
            InstKind::Call { args, .. } => args.iter_mut().collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Inst {
    pub result: Value,
    pub kind: InstKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    pub block: Block,
    pub args: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Terminator {
    Jump(Target),
    Branch {
        cond: Value,
        then: Target,
        otherwise: Target,
    },
    Return(Value),
    Panic(PanicKind),
}

impl Terminator {
    /// Successor targets in canonical order (then before otherwise).
    pub fn targets(&self) -> Vec<&Target> {
        match self {
            Terminator::Jump(target) => vec![target],
            Terminator::Branch {
                then, otherwise, ..
            } => vec![then, otherwise],
            Terminator::Return(_) | Terminator::Panic(_) => Vec::new(),
        }
    }

    pub fn targets_mut(&mut self) -> Vec<&mut Target> {
        match self {
            Terminator::Jump(target) => vec![target],
            Terminator::Branch {
                then, otherwise, ..
            } => vec![then, otherwise],
            Terminator::Return(_) | Terminator::Panic(_) => Vec::new(),
        }
    }

    /// Every value read by the terminator, including jump arguments.
    pub fn operands(&self) -> Vec<Value> {
        let mut values = match self {
            Terminator::Branch { cond, .. } => vec![*cond],
            Terminator::Return(value) => vec![*value],
            Terminator::Jump(_) | Terminator::Panic(_) => Vec::new(),
        };
        for target in self.targets() {
            values.extend_from_slice(&target.args);
        }
        values
    }

    pub fn operands_mut(&mut self) -> Vec<&mut Value> {
        match self {
            Terminator::Jump(target) => target.args.iter_mut().collect(),
            Terminator::Branch {
                cond,
                then,
                otherwise,
            } => {
                let mut values = vec![cond];
                values.extend(then.args.iter_mut());
                values.extend(otherwise.args.iter_mut());
                values
            }
            Terminator::Return(value) => vec![value],
            Terminator::Panic(_) => Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BlockData {
    pub params: Vec<(Value, Ty)>,
    pub insts: Vec<Inst>,
    pub term: Terminator,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Signature {
    pub params: Vec<Ty>,
    pub ret: Ty,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Function {
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub blocks: Vec<BlockData>,
}

impl Function {
    pub fn signature(&self) -> Signature {
        Signature {
            params: self.params.clone(),
            ret: self.ret,
        }
    }

    pub fn block(&self, block: Block) -> Option<&BlockData> {
        self.blocks.get(block.0 as usize)
    }

    /// One past the largest value ID defined in the body.
    pub fn value_bound(&self) -> usize {
        let mut bound = 0usize;
        for data in &self.blocks {
            for (value, _) in &data.params {
                bound = bound.max(value.0 as usize + 1);
            }
            for inst in &data.insts {
                bound = bound.max(inst.result.0 as usize + 1);
            }
        }
        bound
    }

    pub fn inst_count(&self) -> usize {
        self.blocks.iter().map(|data| data.insts.len()).sum()
    }

    /// Blocks reachable from the entry in reverse postorder. Successors are
    /// visited in canonical order, so the result depends only on structure.
    /// Out-of-range targets are skipped; the verifier reports them.
    pub fn reverse_postorder(&self) -> Vec<Block> {
        let count = self.blocks.len();
        if count == 0 {
            return Vec::new();
        }
        let mut visited = vec![false; count];
        let mut postorder = Vec::with_capacity(count);
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        visited[0] = true;
        while let Some(&mut (block, ref mut next)) = stack.last_mut() {
            let targets = self.blocks[block].term.targets();
            if let Some(target) = targets.get(*next) {
                *next += 1;
                let successor = target.block.0 as usize;
                if successor < count && !visited[successor] {
                    visited[successor] = true;
                    stack.push((successor, 0));
                }
            } else {
                postorder.push(Block(block as u32));
                stack.pop();
            }
        }
        postorder.reverse();
        postorder
    }
}

/// A set of functions addressed by `FuncRef` index. A test and interpreter
/// convenience. In the graph, callees are separate typed outputs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    pub functions: Vec<Function>,
}

impl Module {
    pub fn function(&self, callee: FuncRef) -> Option<&Function> {
        self.functions.get(callee.0 as usize)
    }

    pub fn signature(&self, callee: FuncRef) -> Option<Signature> {
        self.function(callee).map(Function::signature)
    }
}

/// Incremental construction with fresh IDs. New blocks end in an
/// `Unreachable` panic until terminated.
pub struct Builder {
    function: Function,
    next_value: u32,
}

impl Builder {
    pub fn new(params: Vec<Ty>, ret: Ty) -> (Self, Vec<Value>) {
        let mut builder = Builder {
            function: Function {
                params: params.clone(),
                ret,
                blocks: Vec::new(),
            },
            next_value: 0,
        };
        let (_, values) = builder.block(&params);
        (builder, values)
    }

    fn fresh(&mut self) -> Value {
        let value = Value(self.next_value);
        self.next_value += 1;
        value
    }

    pub fn block(&mut self, params: &[Ty]) -> (Block, Vec<Value>) {
        let block = Block(self.function.blocks.len() as u32);
        let params: Vec<(Value, Ty)> = params.iter().map(|ty| (self.fresh(), *ty)).collect();
        let values = params.iter().map(|(value, _)| *value).collect();
        self.function.blocks.push(BlockData {
            params,
            insts: Vec::new(),
            term: Terminator::Panic(PanicKind::Unreachable),
        });
        (block, values)
    }

    pub fn inst(&mut self, block: Block, kind: InstKind) -> Value {
        let result = self.fresh();
        self.function.blocks[block.0 as usize]
            .insts
            .push(Inst { result, kind });
        result
    }

    pub fn constant(&mut self, block: Block, ty: Ty, bits: u128) -> Value {
        self.inst(block, InstKind::Const { ty, bits })
    }

    pub fn terminate(&mut self, block: Block, term: Terminator) {
        self.function.blocks[block.0 as usize].term = term;
    }

    pub fn finish(self) -> Function {
        self.function
    }
}

pub fn jump(block: Block, args: Vec<Value>) -> Terminator {
    Terminator::Jump(Target { block, args })
}

pub fn target(block: Block, args: Vec<Value>) -> Target {
    Target { block, args }
}
