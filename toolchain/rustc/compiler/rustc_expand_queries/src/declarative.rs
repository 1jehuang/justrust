//! Disk query for audited literal-only declarative match/transcribe operations.

use rustc_ast::tokenstream::TokenStream;
use rustc_expand::declarative_cache::{DeclarativeCompute, DeclarativeExpansion};
use rustc_middle::ty::{TyCtxt, tls};
use rustc_span::{LocalExpnId, Span};

struct ComputeCtx {
    data: *const (),
    call: unsafe fn(*const ()) -> DeclarativeExpansion,
}
scoped_tls::scoped_thread_local!(static DECLARATIVE_CTX: ComputeCtx);

pub(crate) fn expand_declarative_macro_cached(
    id: LocalExpnId,
    definition: TokenStream,
    input: TokenStream,
    mode: (Span, u16),
    compute: &DeclarativeCompute<'_>,
) -> DeclarativeExpansion {
    // Eligible invocation count is recorded at the caller. This event measures
    // lookup cost separately from actual provider execution below.
    // Store a pointer to the stack reference, not a lifetime-erased fat pointer.
    // The callback can only be used while DECLARATIVE_CTX.set is on the stack.
    unsafe fn call(data: *const ()) -> DeclarativeExpansion {
        let compute = unsafe { &*data.cast::<&DeclarativeCompute<'_>>() };
        compute()
    }
    let ctx = ComputeCtx { data: &compute as *const _ as *const (), call };
    tls::with(|tcx| {
        let _lookup = tcx.sess.prof.generic_activity("jr_declarative_macro_cache_lookup");
        let definition = &*tcx.arena.alloc(definition);
        let input = &*tcx.arena.alloc(input);
        DECLARATIVE_CTX.set(&ctx, || {
            tcx.declarative_macro_expansion((id, definition, input, mode))
                .map(|(index, tokens)| (index, tokens.clone()))
        })
    })
}

pub(crate) fn declarative_macro_expansion<'tcx>(
    tcx: TyCtxt<'tcx>,
    _key: (LocalExpnId, &'tcx TokenStream, &'tcx TokenStream, (Span, u16)),
) -> Option<(u32, &'tcx TokenStream)> {
    let _provider = tcx.sess.prof.generic_activity("jr_declarative_macro_cache_provider");
    // No crate_hash query: local macro definitions are explicit key input, and
    // local crate_hash during expansion would create an expansion/query cycle.
    DECLARATIVE_CTX.with(|ctx| {
        // SAFETY: this synchronous provider is only entered from the scoped
        // callback above. Nothing retains the pointer after set returns.
        unsafe { (ctx.call)(ctx.data) }.map(|(index, tokens)| (index, &*tcx.arena.alloc(tokens)))
    })
}
