//! Support for serializing the dep-graph and reloading it.

// tidy-alphabetical-start
#![deny(missing_docs)]
#![feature(file_buffered)]
#![feature(try_blocks)]
// tidy-alphabetical-end

mod assert_dep_graph;
mod diagnostics;
mod persist;

pub use persist::{
    JR_PERSISTING, copy_cgu_workproduct_to_incr_comp_cache_dir, finalize_session_directory, in_incr_comp_dir_sess,
    in_old_incr_comp_dir_sess, load_query_result_cache, save_work_product_index, setup_dep_graph,
};
use rustc_middle::ty::TyCtxt;
use rustc_middle::util::Providers;

#[allow(missing_docs)]
pub fn provide(providers: &mut Providers) {
    providers.hooks.save_dep_graph =
        |tcx| tcx.sess.time("serialize_dep_graph", || persist::save_dep_graph(tcx));
    providers.queries.jr_proc_macro_inputs = jr_proc_macro_inputs;
    providers.queries.jr_proc_macro_impure = |_, ()| {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        rustc_data_structures::svh::Svh::new(rustc_data_structures::fingerprint::Fingerprint::new(
            nanos as u64,
            (nanos >> 64) as u64 ^ std::process::id() as u64,
        ))
    };
}

/// justrust: a digest of the previous session's proc-macro inputs when each
/// still has its recorded value, otherwise a fresh value that turns every
/// cached expansion red.
fn jr_proc_macro_inputs(tcx: TyCtxt<'_>, (): ()) -> rustc_data_structures::svh::Svh {
    use rustc_data_structures::fingerprint::Fingerprint;
    use rustc_data_structures::svh::Svh;
    let validated = tcx.incr_comp_session.and_then(|session| {
        #[cfg(unix)]
        {
            let old = in_old_incr_comp_dir_sess(
                session,
                rustc_data_structures::jr_macro_inputs::SIDECAR,
            )?;
            rustc_data_structures::jr_macro_inputs::load_and_validate(&old)
        }
        #[cfg(not(unix))]
        {
            let _ = session;
            None
        }
    });
    let epoch = validated.unwrap_or_else(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        nanos ^ (u128::from(std::process::id()) << 96) | 1
    });
    #[cfg(unix)]
    rustc_data_structures::jr_macro_inputs::set_epoch(epoch);
    Svh::new(Fingerprint::new(epoch as u64, (epoch >> 64) as u64))
}
