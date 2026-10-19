//! Exact storage sizing and fallible construction for SSA outputs.
//!
//! Producers admit an upper bound computed from their input dimensions
//! (`shape_bytes`) BEFORE building, build with exact-capacity fallible
//! reservations, then reconcile the lease to the real capacities
//! (`capacity_bytes`). The reconciled lease is either sealed into the
//! published fact's receipt or dropped (refunded) for intermediates.
use super::lower::{Lowered, SsaCallee};
use super::meter::{self, Lease, Meter, Receipt};
use super::{
    BlockData, Inst, InstKind, SsaFunction, SsaPanicText as PanicText, SsaTarget, SsaTerminator,
};
use std::mem::size_of;

fn bytes<T>(count: usize) -> Result<usize, String> {
    meter::mul(count, size_of::<T>())
}

fn sum(parts: impl IntoIterator<Item = Result<usize, String>>) -> Result<usize, String> {
    parts
        .into_iter()
        .try_fold(0usize, |total, part| meter::add(total, part?))
}

fn inst_extra(inst: &Inst, capacity: bool) -> Result<usize, String> {
    match &inst.kind {
        InstKind::Call { args, .. } => bytes::<super::SsaValue>(if capacity {
            args.capacity()
        } else {
            args.len()
        }),
        _ => Ok(0),
    }
}

fn target_extra(target: &SsaTarget, capacity: bool) -> Result<usize, String> {
    bytes::<super::SsaValue>(if capacity {
        target.args.capacity()
    } else {
        target.args.len()
    })
}

fn function_bytes(function: &SsaFunction, capacity: bool) -> Result<usize, String> {
    let pick = |len: usize, cap: usize| if capacity { cap } else { len };
    let mut total = sum([
        bytes::<super::SsaTy>(pick(function.params.len(), function.params.capacity())),
        bytes::<BlockData>(pick(function.blocks.len(), function.blocks.capacity())),
        bytes::<PanicText>(pick(function.panics.len(), function.panics.capacity())),
    ])?;
    for text in &function.panics {
        total = meter::add(
            total,
            meter::add(
                pick(text.message.len(), text.message.capacity()),
                pick(text.location.len(), text.location.capacity()),
            )?,
        )?;
    }
    for data in &function.blocks {
        total = meter::add(
            total,
            sum([
                bytes::<(super::SsaValue, super::SsaTy)>(pick(
                    data.params.len(),
                    data.params.capacity(),
                )),
                bytes::<Inst>(pick(data.insts.len(), data.insts.capacity())),
            ])?,
        )?;
        for inst in &data.insts {
            total = meter::add(total, inst_extra(inst, capacity)?)?;
        }
        for target in data.term.targets() {
            total = meter::add(total, target_extra(target, capacity)?)?;
        }
    }
    Ok(total)
}

/// Bytes an exactly-sized copy of `function` needs (lengths, not capacities).
pub fn shape_bytes(function: &SsaFunction) -> Result<usize, String> {
    function_bytes(function, false)
}

/// Bytes `function` actually owns (allocated capacities).
pub fn capacity_bytes(function: &SsaFunction) -> Result<usize, String> {
    function_bytes(function, true)
}

fn callee_bytes(callees: &[SsaCallee], capacity: bool) -> Result<usize, String> {
    let mut total = bytes::<SsaCallee>(callees.len())?;
    for callee in callees {
        // Target type arguments are shared immutable Arc storage from the
        // sealed concrete facts: only the outer slot is new storage here.
        let params = &callee.signature.params;
        total = sum([
            Ok(total),
            Ok(if capacity {
                callee.symbol.capacity()
            } else {
                callee.symbol.len()
            }),
            bytes::<super::SsaTy>(if capacity {
                params.capacity()
            } else {
                params.len()
            }),
        ])?;
    }
    Ok(total)
}

pub fn lowered_shape_bytes(lowered: &Lowered) -> Result<usize, String> {
    meter::add(
        shape_bytes(&lowered.function)?,
        callee_bytes(&lowered.callees, false)?,
    )
}

pub fn lowered_capacity_bytes(lowered: &Lowered) -> Result<usize, String> {
    meter::add(
        capacity_bytes(&lowered.function)?,
        meter::add(
            callee_bytes(&lowered.callees, false)?,
            bytes::<SsaCallee>(
                lowered
                    .callees
                    .capacity()
                    .saturating_sub(lowered.callees.len()),
            )?,
        )?,
    )
}

/// A value under construction or held as an intermediate, with the lease
/// paying for its storage. Dropping it refunds; `seal` produces the receipt.
#[derive(Debug)]
pub struct Owned<T> {
    pub value: T,
    lease: Lease,
}

impl<T> Owned<T> {
    /// Admit `preflight` bytes, then build. `measure` reports the real owned
    /// capacity afterwards, and the lease is reconciled to it (reserving the
    /// new total before releasing the preflight charge).
    pub fn build(
        meter: &Meter<'_>,
        preflight: usize,
        build: impl FnOnce() -> Result<T, String>,
        measure: impl FnOnce(&T) -> Result<usize, String>,
    ) -> Result<Self, String> {
        let mut lease = meter.lease(preflight)?;
        let value = build()?;
        let actual = measure(&value)?;
        lease.resize(meter, actual)?;
        Ok(Self { value, lease })
    }

    pub fn bytes(&self) -> usize {
        self.lease.bytes()
    }

    /// Extra bytes outside `value` (an `Arc` header, a fact envelope) that
    /// the published owner also keeps alive.
    pub fn admit(&mut self, meter: &Meter<'_>, bytes: usize) -> Result<(), String> {
        self.lease.admit(meter, bytes)
    }

    pub fn seal(self) -> Result<(T, Receipt), String> {
        Ok((self.value, self.lease.seal()?))
    }

    /// Wrap a value built under an already-admitted lease, reconciling the
    /// lease to the value's real `actual` bytes.
    pub fn adopt(
        mut lease: Lease,
        value: T,
        actual: usize,
        meter: &Meter<'_>,
    ) -> Result<Self, String> {
        lease.resize(meter, actual)?;
        Ok(Self { value, lease })
    }

    /// Combine two paid values into one, merging their leases (the merged
    /// charge is admitted before the parts release).
    pub fn combine<U, V>(
        first: Owned<T>,
        second: Owned<U>,
        meter: &Meter<'_>,
        join: impl FnOnce(T, U) -> V,
    ) -> Result<Owned<V>, String> {
        let Owned {
            value: a,
            mut lease,
        } = first;
        let Owned {
            value: b,
            lease: other,
        } = second;
        lease.absorb(meter, other)?;
        Ok(Owned {
            value: join(a, b),
            lease,
        })
    }
}

fn try_vec<T: Clone>(from: &[T]) -> Result<Vec<T>, String> {
    let mut out = Vec::new();
    out.try_reserve_exact(from.len())
        .map_err(|_| "optimizer allocation failed".to_string())?;
    out.extend_from_slice(from);
    Ok(out)
}

pub fn try_string(from: &str) -> Result<String, String> {
    let mut out = String::new();
    out.try_reserve_exact(from.len())
        .map_err(|_| "optimizer allocation failed".to_string())?;
    out.push_str(from);
    Ok(out)
}

pub fn try_clone_inst(inst: &Inst) -> Result<Inst, String> {
    Ok(Inst {
        result: inst.result,
        kind: match &inst.kind {
            InstKind::Call { callee, args } => InstKind::Call {
                callee: *callee,
                args: try_vec(args)?,
            },
            other => other.clone(),
        },
    })
}

pub fn try_clone_target(target: &SsaTarget) -> Result<SsaTarget, String> {
    Ok(SsaTarget {
        block: target.block,
        args: try_vec(&target.args)?,
    })
}

pub fn try_clone_term(term: &SsaTerminator) -> Result<SsaTerminator, String> {
    Ok(match term {
        SsaTerminator::Jump(target) => SsaTerminator::Jump(try_clone_target(target)?),
        SsaTerminator::Branch {
            cond,
            then,
            otherwise,
        } => SsaTerminator::Branch {
            cond: *cond,
            then: try_clone_target(then)?,
            otherwise: try_clone_target(otherwise)?,
        },
        other => other.clone(),
    })
}

pub fn try_clone_block(data: &BlockData) -> Result<BlockData, String> {
    let mut insts = Vec::new();
    insts
        .try_reserve_exact(data.insts.len())
        .map_err(|_| "optimizer allocation failed".to_string())?;
    for inst in &data.insts {
        insts.push(try_clone_inst(inst)?);
    }
    Ok(BlockData {
        params: try_vec(&data.params)?,
        insts,
        term: try_clone_term(&data.term)?,
    })
}

pub fn try_clone_panics(panics: &[PanicText]) -> Result<Vec<PanicText>, String> {
    let mut out = Vec::new();
    out.try_reserve_exact(panics.len())
        .map_err(|_| "optimizer allocation failed".to_string())?;
    for text in panics {
        out.push(PanicText {
            message: try_string(&text.message)?,
            location: try_string(&text.location)?,
        });
    }
    Ok(out)
}

pub fn try_clone_function(function: &SsaFunction) -> Result<SsaFunction, String> {
    let mut blocks = Vec::new();
    blocks
        .try_reserve_exact(function.blocks.len())
        .map_err(|_| "optimizer allocation failed".to_string())?;
    for data in &function.blocks {
        blocks.push(try_clone_block(data)?);
    }
    Ok(SsaFunction {
        params: try_vec(&function.params)?,
        ret: function.ret,
        blocks,
        panics: try_clone_panics(&function.panics)?,
    })
}

/// Paid fallible copy of a function: admitted from its shape, reconciled to
/// the real capacities, work charged per copied element.
pub fn copy_function(
    meter: &Meter<'_>,
    function: &SsaFunction,
) -> Result<Owned<SsaFunction>, String> {
    let shape = shape_bytes(function)?;
    meter.work(meter::add(function.inst_count(), function.blocks.len())?)?;
    meter.work(shape / 8)?;
    Owned::build(
        meter,
        shape,
        || try_clone_function(function),
        capacity_bytes,
    )
}

pub fn try_clone_callees(callees: &[SsaCallee]) -> Result<Vec<SsaCallee>, String> {
    let mut out = Vec::new();
    out.try_reserve_exact(callees.len())
        .map_err(|_| "optimizer allocation failed".to_string())?;
    for callee in callees {
        out.push(SsaCallee {
            symbol: try_string(&callee.symbol)?,
            target: callee.target.clone(),
            signature: super::SsaSignature {
                params: try_vec(&callee.signature.params)?,
                ret: callee.signature.ret,
            },
        });
    }
    Ok(out)
}

/// Paid copy of callee metadata.
pub fn copy_callees(
    meter: &Meter<'_>,
    callees: &[SsaCallee],
) -> Result<Owned<Vec<SsaCallee>>, String> {
    let shape = callee_bytes(callees, false)?;
    meter.work(meter::add(callees.len(), shape / 8)?)?;
    Owned::build(
        meter,
        shape,
        || try_clone_callees(callees),
        |out| {
            meter::add(
                callee_bytes(out, true)?,
                bytes::<SsaCallee>(out.capacity().saturating_sub(out.len()))?,
            )
        },
    )
}
