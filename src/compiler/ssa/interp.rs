//! Reference interpreter: the executable meaning of SSA, used as a
//! differential oracle for every pass and for lowering. Semantics come from
//! `eval`, shared with constant folding.
use super::eval;
use super::verify::{self, Types};
use super::{FuncRef, InstKind, Module, PanicKind, SsaTerminator, SsaTy, SsaValue};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Return(u128),
    Panic(PanicKind),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InterpError {
    Invalid(String),
    /// The step budget ran out. Not a program outcome.
    OutOfFuel,
    StackOverflow,
}

/// Per-call interpreter limits, owned by the caller. Not shared state.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub fuel: u64,
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel: 1_000_000,
            max_depth: 256,
        }
    }
}

struct Run<'a> {
    module: &'a Module,
    types: Vec<Types>,
    fuel: u64,
    max_depth: usize,
}

/// Run `entry` with raw argument bit patterns. Every function in the module
/// is verified first; invalid IR is an error, never undefined behavior.
pub fn run(
    module: &Module,
    entry: FuncRef,
    args: &[u128],
    limits: Limits,
) -> Result<Outcome, InterpError> {
    let signature = |callee: FuncRef| module.signature(callee);
    let mut types = Vec::with_capacity(module.functions.len());
    for (index, function) in module.functions.iter().enumerate() {
        types.push(
            verify::verify(function, &signature)
                .map_err(|error| InterpError::Invalid(format!("f{index}: {error}")))?,
        );
    }
    let function = module
        .function(entry)
        .ok_or_else(|| InterpError::Invalid(format!("missing entry f{}", entry.0)))?;
    if function.params.len() != args.len() {
        return Err(InterpError::Invalid("wrong entry argument count".into()));
    }
    for (arg, ty) in args.iter().zip(&function.params) {
        if *arg > ty.max_bits() {
            return Err(InterpError::Invalid("entry argument out of range".into()));
        }
    }
    let mut run = Run {
        module,
        types,
        fuel: limits.fuel,
        max_depth: limits.max_depth,
    };
    run.call(entry, args, 0)
}

impl Run<'_> {
    fn call(
        &mut self,
        callee: FuncRef,
        args: &[u128],
        depth: usize,
    ) -> Result<Outcome, InterpError> {
        if depth >= self.max_depth {
            return Err(InterpError::StackOverflow);
        }
        let module = self.module;
        let function = &module.functions[callee.0 as usize];
        let mut env: Vec<u128> = vec![0; function.value_bound()];
        let mut block = 0usize;
        let mut incoming: Vec<u128> = args.to_vec();
        loop {
            let data = &function.blocks[block];
            for ((value, _), bits) in data.params.iter().zip(&incoming) {
                env[value.0 as usize] = *bits;
            }
            for inst in &data.insts {
                if self.fuel == 0 {
                    return Err(InterpError::OutOfFuel);
                }
                self.fuel -= 1;
                let ty = |value: SsaValue| -> SsaTy {
                    self.types[callee.0 as usize][value.0 as usize].expect("verified")
                };
                let get = |value: SsaValue| env[value.0 as usize];
                let result = match &inst.kind {
                    InstKind::Const { bits, .. } => Ok(*bits),
                    InstKind::Binary {
                        op,
                        overflow,
                        lhs,
                        rhs,
                    } => eval::binary(*op, *overflow, ty(*lhs), get(*lhs), ty(*rhs), get(*rhs)),
                    InstKind::Unary {
                        op,
                        overflow,
                        value,
                    } => eval::unary(*op, *overflow, ty(*value), get(*value)),
                    InstKind::Cmp { op, lhs, rhs } => Ok(u128::from(eval::compare(
                        *op,
                        ty(*lhs),
                        get(*lhs),
                        get(*rhs),
                    ))),
                    InstKind::Cast { to, value } => Ok(eval::cast(ty(*value), *to, get(*value))),
                    InstKind::Select {
                        cond,
                        then,
                        otherwise,
                    } => Ok(if get(*cond) != 0 {
                        get(*then)
                    } else {
                        get(*otherwise)
                    }),
                    InstKind::Call { callee, args } => {
                        let values: Vec<u128> = args.iter().map(|arg| get(*arg)).collect();
                        match self.call(*callee, &values, depth + 1)? {
                            Outcome::Return(bits) => Ok(bits),
                            Outcome::Panic(kind) => Err(kind),
                        }
                    }
                };
                match result {
                    Ok(bits) => env[inst.result.0 as usize] = bits,
                    Err(kind) => return Ok(Outcome::Panic(kind)),
                }
            }
            if self.fuel == 0 {
                return Err(InterpError::OutOfFuel);
            }
            self.fuel -= 1;
            let next = match &data.term {
                SsaTerminator::Return(value) => return Ok(Outcome::Return(env[value.0 as usize])),
                SsaTerminator::Panic(kind) => return Ok(Outcome::Panic(*kind)),
                SsaTerminator::Jump(target) => target,
                SsaTerminator::Branch {
                    cond,
                    then,
                    otherwise,
                } => {
                    if env[cond.0 as usize] != 0 {
                        then
                    } else {
                        otherwise
                    }
                }
            };
            incoming = next.args.iter().map(|arg| env[arg.0 as usize]).collect();
            block = next.block.0 as usize;
        }
    }
}
