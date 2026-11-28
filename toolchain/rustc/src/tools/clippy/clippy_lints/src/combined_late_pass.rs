//! A statically-combined late lint pass.
//!
//! Folds clippy's ~300 late passes into one concrete struct, one field per pass.
//! The single `LateLintPass` impl forwards each `check_*` to every field; with
//! concrete types and `#[inline(always)]`, unoverridden methods are DCE'd and the
//! rest become direct calls, so there is no vtable or per-node dynamic dispatch.
//!
//! Mirrors rustc's `declare_combined_late_lint_pass!`, but wraps each field in an
//! `Option` that is set to `None` if a pass can be skipped (determined by the same
//! "lint still needs to run" predicate `rustc_lint::late` uses). Disabled passes
//! are skipped by a branch rather than dropped from a `Vec`, keeping clippy's
//! allow-by-default fast path.

/// Run one field's `check_*`, if that field is active.
///
/// Fully qualified through [`rustc_lint::LateLintPass`] since some passes impl
/// both `EarlyLintPass` and `LateLintPass` with like-named methods, which would
/// be ambiguous on the concrete field type. The args arrive as one `tt` and are
/// re-parsed here so the per-field and per-argument repetitions never share a
/// nesting level.
#[macro_export]
macro_rules! run_combined_late_lint_pass_field {
    ($self:ident, $field:ident, $name:ident, ($($arg:expr),* $(,)?)) => {
        if let Some(pass) = &mut $self.$field {
            if $crate::combined_late_pass::PASS_TIMES {
                let start = std::time::Instant::now();
                rustc_lint::LateLintPass::$name(pass, $($arg),*);
                $crate::combined_late_pass::record_pass_time(stringify!($field), start.elapsed());
            } else {
                rustc_lint::LateLintPass::$name(pass, $($arg),*);
            }
        }
    };
}

/// Per-pass timing is a measurement build only: compile `clippy_lints` with
/// `JR_CLIPPY_PASS_TIMES=1` in the environment. Otherwise this is `false` and the
/// timing branch is removed at compile time, so the shipping pass is unchanged.
pub const PASS_TIMES: bool = option_env!("JR_CLIPPY_PASS_TIMES").is_some();

thread_local! {
    static PASS_TIME_TABLE: std::cell::RefCell<std::collections::BTreeMap<&'static str, (u64, u64)>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

#[inline(never)]
pub fn record_pass_time(pass: &'static str, elapsed: std::time::Duration) {
    PASS_TIME_TABLE.with(|table| {
        let mut table = table.borrow_mut();
        let entry = table.entry(pass).or_insert((0, 0));
        entry.0 += elapsed.as_nanos() as u64;
        entry.1 += 1;
    });
}

/// Print and clear this thread's table: `jr-clippy-pass-time <label> <pass> <nanos> <calls>`.
pub fn dump_pass_times(label: &str) {
    if !PASS_TIMES {
        return;
    }
    PASS_TIME_TABLE.with(|table| {
        let mut rows: Vec<_> = std::mem::take(&mut *table.borrow_mut()).into_iter().collect();
        rows.sort_by_key(|&(name, (nanos, _))| (std::cmp::Reverse(nanos), name));
        for (name, (nanos, calls)) in rows {
            eprintln!("jr-clippy-pass-time {label} {name} {nanos} {calls}");
        }
    });
}

/// Forward one `check_*` method to every field of the combined pass.
#[macro_export]
macro_rules! expand_combined_late_lint_pass_method {
    ([$($field:ident),*], $self:ident, $name:ident, $args:tt) => ({
        $($crate::run_combined_late_lint_pass_field!($self, $field, $name, $args);)*
    })
}

/// Generate the combined `LateLintPass` impl's `check_*` methods, one per method
/// in rustc's late-pass method list.
#[macro_export]
macro_rules! expand_combined_late_lint_pass_methods {
    ($fields:tt, [$($(#[$attr:meta])* fn $name:ident($($param:ident: $arg:ty),*);)*]) => (
        $(#[inline(always)] fn $name(&mut self, cx: &rustc_lint::LateContext<'tcx>, $($param: $arg),*) {
            $crate::expand_combined_late_lint_pass_method!($fields, self, $name, (cx, $($param),*));
        })*
    )
}

/// Declare the combined struct (one optional field per pass) plus its
/// `LintPass`/`LateLintPass` impls. The method list comes from
/// `rustc_lint::late_lint_methods!` so it can't drift from rustc's.
///
/// Each entry is `Field: Type = constructor`; `new`'s params (`tcx`, `conf`, ...)
/// come from the caller so ctor exprs can name them without hygiene trouble.
#[macro_export]
macro_rules! combined_late_lint_pass {
    (
        [$name:ident, ($($pname:ident: $pty:ty),* $(,)?), [$($field:ident: $fty:ty = $ctor:expr,)*]],
        $methods:tt
    ) => {
        #[allow(non_snake_case)]
        pub struct $name<'tcx> {
            _marker: std::marker::PhantomData<&'tcx ()>,
            $($field: Option<$fty>,)*
        }

        impl<'tcx> $name<'tcx> {
            pub fn new<F: Fn(&str, &rustc_lint::LintVec) -> bool>($($pname: $pty,)* is_active: &F) -> Self {
                Self {
                    _marker: std::marker::PhantomData,
                    $($field: is_active(stringify!($field), &<$fty>::lint_vec()).then(|| $ctor),)*
                }
            }
        }

        #[allow(rustc::lint_pass_impl_without_macro)]
        impl<'tcx> rustc_lint::LintPass for $name<'tcx> {
            fn name(&self) -> &'static str {
                stringify!($name)
            }
            fn get_lints(&self) -> rustc_lint::LintVec {
                // Reserve at least one slot per pass up front to skip the early reallocations.
                let mut lints = Vec::with_capacity(${count($field)});
                $(lints.extend(<$fty>::lint_vec());)*
                lints
            }
        }

        impl<'tcx> Drop for $name<'tcx> {
            fn drop(&mut self) {
                $crate::combined_late_pass::dump_pass_times(stringify!($name));
            }
        }

        impl<'tcx> rustc_lint::LateLintPass<'tcx> for $name<'tcx> {
            $crate::expand_combined_late_lint_pass_methods!([$($field),*], $methods);
        }
    };
}
