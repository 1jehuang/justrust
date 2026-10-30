//! Default-off, workspace-only frontend parallelism experiment.
//!
//! Keep the selected flag identical in command construction and fingerprints:
//! changing the opt-in rebuilds workspace units, not registry dependencies.

use super::{BuildRunner, Unit};

/// Select a flag only for actual nightly/dev rustc, never merely nightly Cargo
/// or a stable compiler with RUSTC_BOOTSTRAP in its environment.
pub(super) fn flag(build_runner: &BuildRunner<'_, '_>, unit: &Unit) -> Option<String> {
    let bcx = build_runner.bcx;
    if !bcx.ws.is_member(&unit.pkg)
        || unit.is_std
        || unit.target.is_custom_build()
        || unit.mode.is_doc()
        || unit.mode.is_doc_test()
        || unit.mode.is_doc_scrape()
        || unit.mode.is_run_custom_build()
    {
        return None;
    }
    let flags = unit
        .rustflags
        .iter()
        .map(String::as_str)
        .chain(unit.profile.rustflags.iter().map(|s| s.as_str()))
        .chain(
            bcx.extra_args_for(unit)
                .into_iter()
                .flatten()
                .map(String::as_str),
        );
    // env_config contains only effective configured values: forced entries or
    // defaults absent from the process snapshot. Use it first to preserve
    // Cargo's existing [env] precedence instead of defining another schema.
    let configured = bcx.gctx.env_config().ok()?.get("JUSTRUST_FRONTEND_THREADS");
    let setting = match configured {
        Some(value) => value.to_str(),
        None => bcx.gctx.get_env("JUSTRUST_FRONTEND_THREADS").ok(),
    };
    select_flag(setting, bcx.rustc().version.pre.as_str(), flags)
}

fn select_flag<'a>(
    setting: Option<&str>,
    rustc_channel: &str,
    flags: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    if !matches!(rustc_channel, "nightly" | "dev") {
        return None;
    }
    // Zero is an explicit off switch, not "all CPUs". Invalid settings fail
    // open without adding flags or breaking a build. Bound matches rustc.
    let threads = setting?.parse::<u8>().ok().filter(|n| *n != 0)?;
    if has_explicit_control(flags) {
        return None;
    }
    Some(format!("-Zthreads={threads}"))
}

/// Both joined and split rustc spellings, including global job limits. Do not
/// override a user limit or create a --jobs-frontend/-Zthreads conflict.
fn has_explicit_control<'a>(flags: impl IntoIterator<Item = &'a str>) -> bool {
    let mut after_z = false;
    for flag in flags {
        if (after_z && (flag == "threads" || flag.starts_with("threads=")))
            || flag == "-Zthreads"
            || flag.starts_with("-Zthreads=")
            || flag == "--jobs"
            || flag.starts_with("--jobs=")
            || flag == "--jobs-frontend"
            || flag.starts_with("--jobs-frontend=")
            || flag == "-j"
            || flag.strip_prefix("-j").is_some_and(|s| {
                !s.is_empty() && (s.starts_with('=') || s.bytes().all(|b| b.is_ascii_digit()))
            })
            // An argument file can carry any of the above. Conservatively
            // defer rather than parsing it with subtly different semantics.
            || flag.starts_with('@')
        {
            return true;
        }
        after_z = flag == "-Z";
    }
    false
}

#[cfg(test)]
mod tests {
    use super::select_flag;

    #[test]
    fn opt_in_and_actual_compiler_channel_are_required() {
        for setting in [
            None,
            Some(""),
            Some("0"),
            Some("256"),
            Some("auto"),
            Some("-1"),
        ] {
            assert_eq!(select_flag(setting, "nightly", []), None);
        }
        for channel in ["", "beta", "beta.1", "stable"] {
            assert_eq!(select_flag(Some("8"), channel, []), None);
        }
        for channel in ["nightly", "dev"] {
            assert_eq!(
                select_flag(Some("8"), channel, []),
                Some("-Zthreads=8".into())
            );
            assert_eq!(
                select_flag(Some("1"), channel, []),
                Some("-Zthreads=1".into())
            );
            assert_eq!(
                select_flag(Some("255"), channel, []),
                Some("-Zthreads=255".into())
            );
        }
    }

    #[test]
    fn explicit_frontend_and_global_job_controls_win() {
        for flags in [
            vec!["-Zthreads=4"],
            vec!["-Z", "threads=4"],
            vec!["-Zthreads", "4"],
            vec!["-Z", "threads", "4"],
            vec!["--jobs-frontend=4"],
            vec!["--jobs-frontend", "4"],
            vec!["--jobs=4"],
            vec!["--jobs", "4"],
            vec!["-j4"],
            vec!["-j=4"],
            vec!["-j", "4"],
            vec!["@rustc-args"],
        ] {
            assert_eq!(
                select_flag(Some("8"), "nightly", flags.clone()),
                None,
                "{flags:?}"
            );
        }
    }

    #[test]
    fn backend_limits_and_unrelated_unstable_flags_do_not_disable_frontend() {
        assert_eq!(
            select_flag(
                Some("8"),
                "nightly",
                [
                    "--jobs-backend=2",
                    "--jobs-linker=2",
                    "-Z",
                    "unstable-options"
                ]
            ),
            Some("-Zthreads=8".into())
        );
    }
}
