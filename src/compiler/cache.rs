//! Persistent reuse of authority-free intermediate compiler outputs.
//!
//! Every lookup runs inside its original graph producer, after normal input
//! reads and proof validation. Only plain SSA values and machine bytes cross
//! builds. Callee targets, proof capabilities and paid receipts are always
//! reconstructed from the current graph, never deserialized from storage.
use super::{backend, ir, link_cache::Outcome, ownership, source::SourceOptions, ssa};
use crate::persistent_cache::{KeyBuilder, LocalCache, MAX_ENTRY_BYTES};
use ssa::{meter::Meter, store::Owned};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectStage {
    Native,
    Ssa,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectUse {
    pub stage: ObjectStage,
    pub outcome: Outcome,
}
impl crate::engine::Fact for ObjectUse {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub hits: usize,
    pub misses: usize,
    pub stored: usize,
    pub ineligible: usize,
}
impl Counts {
    pub(super) fn record(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Hit => self.hits += 1,
            Outcome::MissStored => {
                self.misses += 1;
                self.stored += 1;
            }
            Outcome::MissNotStored => self.misses += 1,
            Outcome::Ineligible => self.ineligible += 1,
            Outcome::Disabled => {}
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub native_objects: Counts,
    pub ssa_objects: Counts,
    pub ssa_lowering: Counts,
    pub local_optimization: Counts,
    pub post_inline_optimization: Counts,
}

fn configuration(options: &SourceOptions) -> Option<(&LocalCache, &[u8; 32])> {
    Some((
        options.local_cache.as_ref()?,
        options.compiler_image.as_ref()?.implementation_digest(),
    ))
}
fn unavailable(options: &SourceOptions) -> Outcome {
    if options.local_cache.is_some() {
        Outcome::Ineligible
    } else {
        Outcome::Disabled
    }
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

fn ty(key: &mut KeyBuilder, ty: ssa::SsaTy) {
    match ty {
        ssa::SsaTy::Unit => key.field(&[0]),
        ssa::SsaTy::Bool => key.field(&[1]),
        ssa::SsaTy::Int(int) => key.field(&[2, int.bits(), u8::from(int.signed())]),
    }
}
fn ssa_key(
    lowered: &ssa::lower::Lowered,
    domain: &[u8],
    compiler: &[u8; 32],
    symbols: bool,
    meter: &Meter<'_>,
) -> Result<([u8; 32], usize), String> {
    // Exact encoding, not a non-cryptographic summary or Debug rendering.
    let encoded = ssa::cache_codec::encode(&lowered.function, meter)?;
    let mut key = KeyBuilder::new(domain, compiler);
    key.field(&encoded.value);
    key.field(&(lowered.callees.len() as u64).to_le_bytes());
    for callee in &lowered.callees {
        meter.work(
            callee
                .signature
                .params
                .len()
                .saturating_add(callee.symbol.len())
                .saturating_add(1),
        )?;
        if symbols {
            key.field(callee.symbol.as_bytes());
        }
        key.field(&(callee.signature.params.len() as u64).to_le_bytes());
        for param in &callee.signature.params {
            ty(&mut key, *param);
        }
        ty(&mut key, callee.signature.ret);
    }
    // A per-input admission/eligibility bound. A producer never stores a
    // result it could not read back within this same bound on the next build.
    let bound = encoded
        .value
        .len()
        .saturating_mul(128)
        .saturating_add(65536)
        .min(MAX_ENTRY_BYTES);
    Ok((key.finish(), bound))
}

fn verified(
    function: &ssa::SsaFunction,
    lowered: &ssa::lower::Lowered,
    meter: &Meter<'_>,
) -> Result<bool, String> {
    match ssa::verify::verify_metered(function, &|callee| lowered.signature(callee), meter) {
        Ok(_) => Ok(true),
        Err(error) if error.is_resource() => Err(error.to_string()),
        Err(_) => Ok(false),
    }
}

pub(super) fn local_opt(
    lowered: &ssa::lower::Lowered,
    meter: &Meter<'_>,
    options: &SourceOptions,
) -> Result<(Owned<ssa::lower::Lowered>, Outcome), String> {
    let cached = match configuration(options) {
        Some((cache, compiler)) => Some((
            cache,
            ssa_key(
                lowered,
                b"justrust/ssa-local-opt/v1",
                compiler,
                false,
                meter,
            )?,
        )),
        None => None,
    };
    if let Some((cache, (key, bound))) = &cached
        && let Some(bytes) = load(cache, key, *bound, meter)?
        && let Some(function) = ssa::cache_codec::decode(&bytes.value, meter)?
        && function.value.params == lowered.function.params
        && function.value.ret == lowered.function.ret
        && verified(&function.value, lowered, meter)?
    {
        let callees = ssa::store::copy_callees(meter, &lowered.callees)?;
        let result = Owned::combine(function, callees, meter, |function, callees| {
            ssa::lower::Lowered { function, callees }
        })?;
        return Ok((result, Outcome::Hit));
    }
    let (function, _stats) = ssa::opt::local_metered(
        &lowered.function,
        &|callee| lowered.signature(callee),
        meter,
    )
    .map_err(|error| error.to_string())?;
    let outcome = match cached {
        None => unavailable(options),
        Some((cache, (key, bound))) => {
            let bytes = ssa::cache_codec::encode(&function.value, meter)?;
            if bytes.value.len() <= bound && cache.store(&key, &bytes.value).is_ok() {
                Outcome::MissStored
            } else {
                Outcome::MissNotStored
            }
        }
    };
    let callees = ssa::store::copy_callees(meter, &lowered.callees)?;
    let result = Owned::combine(function, callees, meter, |function, callees| {
        ssa::lower::Lowered { function, callees }
    })?;
    Ok((result, outcome))
}

fn restore_object(
    cache: &LocalCache,
    key: &[u8; 32],
    bound: usize,
    meter: &Meter<'_>,
) -> Result<Option<Owned<backend::Object>>, String> {
    match load(cache, key, bound, meter)? {
        Some(bytes) => super::object_codec::decode(&bytes.value, meter),
        None => Ok(None),
    }
}
fn store_object(
    cache: &LocalCache,
    key: &[u8; 32],
    bound: usize,
    object: &backend::Object,
    meter: &Meter<'_>,
) -> Result<Outcome, String> {
    let Some(encoded) = super::object_codec::encode(object, meter)? else {
        return Ok(Outcome::Ineligible);
    };
    Ok(
        if encoded.value.len() <= bound && cache.store(key, &encoded.value).is_ok() {
            Outcome::MissStored
        } else {
            Outcome::MissNotStored
        },
    )
}

pub(super) fn ssa_object(
    lowered: &ssa::lower::Lowered,
    types: &[Option<ssa::SsaTy>],
    meter: &Meter<'_>,
    options: &SourceOptions,
) -> Result<(Owned<backend::Object>, Outcome), String> {
    let cached = match configuration(options) {
        Some((cache, compiler)) => Some((
            cache,
            ssa_key(
                lowered,
                b"justrust/ssa-machine/x86_64-linux/v1",
                compiler,
                true,
                meter,
            )?,
        )),
        None => None,
    };
    if let Some((cache, (key, bound))) = &cached
        && let Some(object) = restore_object(cache, key, *bound, meter)?
    {
        return Ok((object, Outcome::Hit));
    }
    let (object, _stats) = ssa::machine::emit(lowered, types, meter)?;
    let outcome = match cached {
        Some((cache, (key, bound))) => store_object(cache, &key, bound, &object.value, meter)?,
        None => unavailable(options),
    };
    Ok((object, outcome))
}

pub(super) fn native_object(
    concrete: &ownership::ConcreteCfg,
    resolve: &dyn Fn(&ir::CallTarget) -> Result<String, String>,
    options: &SourceOptions,
    meter: &Meter<'_>,
) -> Result<(backend::Object, Outcome), String> {
    let cached = match configuration(options) {
        Some((cache, compiler)) => super::native_cache_key::key(
            concrete.function(),
            resolve,
            options.overflow_checks,
            options.panic_unwind,
            compiler,
            meter,
        )?
        .map(|key| (cache, key)),
        None => None,
    };
    // The bound scales with this function's own shape. It is an eligibility
    // cap, not a shared compiler budget or a promise of a fixed code size.
    let function = concrete.function();
    let shape = function
        .blocks
        .iter()
        .fold(function.locals.len(), |n, block| {
            n.saturating_add(block.statements.len()).saturating_add(1)
        });
    let bound = shape
        .saturating_mul(16384)
        .saturating_add(65536)
        .min(MAX_ENTRY_BYTES);
    if let Some((cache, key)) = &cached
        && let Some(object) = restore_object(cache, key, bound, meter)?
    {
        let (mut object, receipt) = object.seal(meter)?;
        object.receipt = Some(std::sync::Arc::new(receipt));
        return Ok((object, Outcome::Hit));
    }
    let object = backend::emit_cfg_function_with_profile(
        concrete,
        resolve,
        options.overflow_checks,
        options.panic_unwind,
    )?;
    let outcome = match cached {
        Some((cache, key)) => store_object(cache, &key, bound, &object, meter)?,
        None => unavailable(options),
    };
    Ok((object, outcome))
}
