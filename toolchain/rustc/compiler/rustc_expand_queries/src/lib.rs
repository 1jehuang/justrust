#![allow(internal_features, reason = "proc macro internals")]
#![feature(proc_macro_internals)]

use rustc_middle::ty::TyCtxt;

mod attr;
mod declarative;
mod derive;

/// Records the external inputs of one proc-macro expansion; on drop, an
/// expansion the recorder could not model is made non-reusable.
pub(crate) struct InputRecording<'tcx> {
    tcx: TyCtxt<'tcx>,
}

impl<'tcx> InputRecording<'tcx> {
    pub(crate) fn begin(tcx: TyCtxt<'tcx>) -> Self {
        #[cfg(unix)]
        rustc_data_structures::jr_macro_inputs::begin();
        // The recorder only observes the current thread.
        #[cfg(unix)]
        if tcx.sess.opts.unstable_opts.proc_macro_execution_strategy
            != rustc_session::config::ProcMacroExecutionStrategy::SameThread
        {
            rustc_data_structures::jr_macro_inputs::mark_impure("cross-thread execution");
        }
        InputRecording { tcx }
    }
}

impl Drop for InputRecording<'_> {
    fn drop(&mut self) {
        #[cfg(unix)]
        let impure = rustc_data_structures::jr_macro_inputs::end().impure.is_some();
        #[cfg(not(unix))]
        let impure = true;
        if impure {
            let _ = self.tcx.jr_proc_macro_impure(());
        }
    }
}

pub fn setup_callbacks() {
    rustc_expand::declarative_cache::EXPAND_DECLARATIVE_MACRO_CACHED
        .swap(&(declarative::expand_declarative_macro_cached as _));
    rustc_expand::proc_macro::EXPAND_DERIVE_MACRO_CACHED
        .swap(&(derive::expand_derive_macro_cached as _));
    rustc_expand::proc_macro::EXPAND_ATTR_MACRO_CACHED
        .swap(&(attr::expand_attr_macro_cached as _));
}

pub fn provide(providers: &mut rustc_middle::query::Providers) {
    providers.declarative_macro_expansion = declarative::declarative_macro_expansion;
    providers.derive_macro_expansion = derive::derive_macro_expansion;
    providers.attr_macro_expansion = attr::attr_macro_expansion;
}
