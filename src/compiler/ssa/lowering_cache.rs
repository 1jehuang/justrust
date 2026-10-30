//! Persistent, authority-free SSA lowering bodies. Callee identities always come
//! from the current sealed CFG, never from the persistent payload.
use super::{SsaSignature, SsaTy, cache_codec, lower, meter, store::Owned, verify};
use crate::compiler::{cfg, ir, link_cache, native_cache_key, source::SourceOptions};
use crate::persistent_cache::{KeyBuilder, LocalCache, MAX_ENTRY_BYTES};
use meter::Meter;
use std::mem::size_of;

fn bound(function: &cfg::Function, meter: &Meter<'_>) -> Result<usize, String> {
    // Conservative per-input wire bound, including all-local block parameters,
    // edges, materialized operands, source locations and panic text. This is
    // only an admission bound, not a claim that a decoded body is valid.
    let mut bytes = meter::add(65536, meter::mul(function.params.len(), 8)?)?;
    bytes = meter::add(bytes, meter::mul(function.locals.len(), 64)?)?;
    bytes = meter::add(
        bytes,
        meter::mul(
            meter::mul(function.blocks.len(), function.locals.len())?,
            64,
        )?,
    )?;
    for (index, block) in function.blocks.iter().enumerate() {
        meter.work(meter::add(block.statements.len(), 1)?)?;
        bytes = meter::add(
            bytes,
            meter::mul(meter::add(block.statements.len(), 1)?, 256)?,
        )?;
        for statement in 0..block.statements.len() {
            if let Some(location) = function.runtime_sites.get(cfg::BlockId(index), statement) {
                // At most three materialized operands/instructions per statement.
                bytes = meter::add(bytes, meter::mul(meter::add(location.path.len(), 32)?, 4)?)?;
            }
        }
        match &block.terminator.value {
            cfg::Terminator::Call {
                signature, args, ..
            } => {
                bytes = meter::add(
                    bytes,
                    meter::mul(meter::add(signature.params.len(), args.len())?, 64)?,
                )?;
            }
            cfg::Terminator::Panic(text) => {
                bytes = meter::add(bytes, meter::add(text.message.len(), text.location.len())?)?;
            }
            _ => {}
        }
    }
    Ok(bytes.min(MAX_ENTRY_BYTES))
}

fn load(
    cache: &LocalCache,
    key: &[u8; 32],
    bound: usize,
    meter: &Meter<'_>,
) -> Result<Option<Owned<Vec<u8>>>, String> {
    let Some(size) = cache.payload_size(key, bound) else {
        return Ok(None);
    };
    let lease = meter.lease(size)?;
    let Some(bytes) = cache.load_bounded(key, size) else {
        return Ok(None);
    };
    meter.work(bytes.len())?;
    let capacity = bytes.capacity();
    Owned::adopt(lease, bytes, capacity, meter).map(Some)
}

fn callee_bytes(callees: &Vec<lower::SsaCallee>) -> Result<usize, String> {
    let mut bytes = meter::mul(callees.capacity(), size_of::<lower::SsaCallee>())?;
    for callee in callees {
        bytes = meter::add(bytes, callee.symbol.capacity())?;
        bytes = meter::add(
            bytes,
            meter::mul(callee.signature.params.capacity(), size_of::<SsaTy>())?,
        )?;
    }
    Ok(bytes)
}

fn callees(
    function: &cfg::Function,
    resolve: &dyn Fn(&ir::CallTarget) -> Result<String, String>,
    meter: &Meter<'_>,
) -> Result<Option<Owned<Vec<lower::SsaCallee>>>, String> {
    let mut count = 0;
    let mut params = 0;
    // Reject inexact reconstruction before allocating. Calls are visited in
    // precisely the same block order as lower::Ctx::block.
    for block in &function.blocks {
        meter.work(1)?;
        if let cfg::Terminator::Call {
            target, signature, ..
        } = &block.terminator.value
        {
            meter.work(signature.params.len())?;
            if !matches!(target, ir::Callee::Direct(_))
                || signature
                    .params
                    .iter()
                    .any(|ty| lower::scalar(ty).is_none())
                || lower::scalar(&signature.result).is_none()
            {
                return Ok(None);
            }
            count = meter::add(count, 1)?;
            params = meter::add(params, signature.params.len())?;
        }
    }
    let preflight = meter::add(
        meter::mul(count, size_of::<lower::SsaCallee>())?,
        meter::mul(params, size_of::<SsaTy>())?,
    )?;
    let result = Owned::build(
        meter,
        preflight,
        || {
            let mut out: Vec<lower::SsaCallee> = Vec::new();
            out.try_reserve_exact(count)
                .map_err(|_| "SSA lowering cache allocation failed".to_string())?;
            for block in &function.blocks {
                let cfg::Terminator::Call {
                    target: ir::Callee::Direct(target),
                    signature,
                    ..
                } = &block.terminator.value
                else {
                    continue;
                };
                let mut duplicate = false;
                for candidate in &out {
                    if lower::call_target_eq(&candidate.target, target, meter)? {
                        duplicate = true;
                        break;
                    }
                }
                if duplicate {
                    continue;
                }
                meter.work(signature.params.len())?;
                let mut params = Vec::new();
                params
                    .try_reserve_exact(signature.params.len())
                    .map_err(|_| "SSA lowering cache allocation failed".to_string())?;
                for ty in &signature.params {
                    params.push(lower::scalar(ty).ok_or("validated scalar call changed")?);
                }
                out.push(lower::SsaCallee {
                    // Resolver pays for symbol allocation just as in real lowering.
                    symbol: resolve(target)?,
                    target: target.clone(),
                    signature: SsaSignature {
                        params,
                        ret: lower::scalar(&signature.result)
                            .ok_or("validated scalar result changed")?,
                    },
                });
            }
            Ok(out)
        },
        callee_bytes,
    )?;
    Ok(Some(result))
}

pub(crate) fn lower(
    function: &cfg::Function,
    resolve: &dyn Fn(&ir::CallTarget) -> Result<String, String>,
    meter: &Meter<'_>,
    options: &SourceOptions,
) -> Result<(lower::Lowering, link_cache::Outcome), String> {
    use link_cache::Outcome;
    let real = || lower::lower_with_overflow(function, resolve, meter, options.overflow_checks);
    let Some(cache) = options.local_cache.as_ref() else {
        return Ok((real()?, Outcome::Disabled));
    };
    let Some(compiler) = options.compiler_image.as_ref() else {
        return Ok((real()?, Outcome::Ineligible));
    };
    let compiler = compiler.implementation_digest();
    let Some(fingerprint) = native_cache_key::key(
        function,
        resolve,
        options.overflow_checks,
        options.panic_unwind,
        compiler,
        meter,
    )?
    else {
        return Ok((real()?, Outcome::Ineligible));
    };
    let mut key = KeyBuilder::new(b"ssa-lowering/v1", compiler);
    key.field(&fingerprint);
    let key = key.finish();
    let bound = bound(function, meter)?;
    if let Some(bytes) = load(cache, &key, bound, meter)?
        && let Some(candidate) = cache_codec::decode(&bytes.value, meter)?
        && lower::cache_eligible(function, meter)?
        && candidate.value.ret == lower::scalar(&function.result).unwrap_or(SsaTy::Unit)
        && lower::scalar(&function.result).is_some()
        && candidate.value.params.len() == function.params.len()
        && candidate
            .value
            .params
            .iter()
            .zip(&function.params)
            .all(|(ty, local)| {
                function
                    .locals
                    .get(local.0)
                    .and_then(|local| lower::scalar(&local.ty))
                    == Some(*ty)
            })
        && let Some(callees) = callees(function, resolve, meter)?
    {
        let verified = verify::verify_metered(
            &candidate.value,
            &|callee| {
                callees
                    .value
                    .get(callee.0 as usize)
                    .map(|callee| &callee.signature)
            },
            meter,
        );
        match verified {
            Err(error) if error.is_resource() => return Err(error.to_string()),
            Err(_) => {}
            Ok(_) => {
                let result = Owned::combine(candidate, callees, meter, |function, callees| {
                    lower::Lowered { function, callees }
                })?;
                return Ok((lower::Lowering::Lowered(Box::new(result)), Outcome::Hit));
            }
        }
    }
    let result = real()?;
    let outcome = if let lower::Lowering::Lowered(lowered) = &result {
        let bytes = cache_codec::encode(&lowered.value.function, meter)?;
        if bytes.value.len() <= bound && cache.store(&key, &bytes.value).is_ok() {
            Outcome::MissStored
        } else {
            Outcome::MissNotStored
        }
    } else {
        Outcome::Ineligible
    };
    Ok((result, outcome))
}
