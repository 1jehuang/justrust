//! justrust: cached expansion of attribute proc macros, the same mechanism as
//! `derive.rs` (an incremental query whose result is stored on disk).
//!
//! On the Desktop reference loop `#[gpui::test]` runs 724 times per edit and
//! is 0.96 s of the 1.78 s expansion pass, although only the edited test's
//! tokens changed (justrust FINDINGS 29).

use rustc_ast::tokenstream::TokenStream;
use rustc_expand::base::ExtCtxt;
use rustc_middle::ty::{TyCtxt, tls};
use rustc_proc_macro as pm;
use rustc_span::LocalExpnId;

type Client = pm::bridge::client::Client;

struct QueryAttrExpandCtx {
    /// Type-erased `&mut ExtCtxt`.
    expansion_ctx: *mut (),
    client: Client,
}

scoped_tls::scoped_thread_local!(static ATTR_EXPAND_CTX: QueryAttrExpandCtx);

impl QueryAttrExpandCtx {
    fn enter<F: FnOnce() -> R, R>(ecx: &mut ExtCtxt<'_>, client: Client, f: F) -> R {
        let ctx = Self { expansion_ctx: ecx as *mut _ as *mut (), client };
        ATTR_EXPAND_CTX.set(&ctx, f)
    }

    fn with<F, R>(f: F) -> R
    where
        F: for<'a, 'b> FnOnce(&'b mut ExtCtxt<'a>, Client) -> R,
    {
        ATTR_EXPAND_CTX.with(|ctx| {
            // SAFETY: as in `derive.rs`: only reachable while `enter` is on
            // the stack, whose signature keeps the `ExtCtxt` borrow alive.
            let ecx = unsafe { ctx.expansion_ctx.cast::<ExtCtxt<'_>>().as_mut().unwrap() };
            f(ecx, ctx.client)
        })
    }
}

pub(crate) fn expand_attr_macro_cached(
    invoc_id: LocalExpnId,
    annotation: TokenStream,
    annotated: TokenStream,
    ecx: &mut ExtCtxt<'_>,
    client: Client,
) -> Result<TokenStream, ()> {
    tls::with(|tcx| {
        let annotation = &*tcx.arena.alloc(annotation);
        let annotated = &*tcx.arena.alloc(annotated);
        let key = (invoc_id, annotation, annotated);
        QueryAttrExpandCtx::enter(ecx, client, move || tcx.attr_macro_expansion(key).cloned())
    })
}

pub(crate) fn attr_macro_expansion<'tcx>(
    tcx: TyCtxt<'tcx>,
    key: (LocalExpnId, &'tcx TokenStream, &'tcx TokenStream),
) -> Result<&'tcx TokenStream, ()> {
    let (invoc_id, annotation, annotated) = key;

    // Invalidate when the crate defining the proc macro changes.
    let _ = tcx.crate_hash(invoc_id.expn_data().macro_def_id.unwrap().krate);

    QueryAttrExpandCtx::with(|ecx, client| {
        let span = invoc_id.expn_data().call_site;
        rustc_expand::proc_macro::expand_attr_macro(
            ecx,
            span,
            annotation.clone(),
            annotated.clone(),
            client,
        )
        .map(|ts| &*tcx.arena.alloc(ts))
        .map_err(|_| ())
    })
}
