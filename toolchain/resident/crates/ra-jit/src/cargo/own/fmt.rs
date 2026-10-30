//! `cargo blaze fmt`: blaze's formatter (`crates/blaze-fmt`), run in this process. Given `.rs`
//! files it is rustfmt (`blaze fmt src/main.rs`); otherwise it is cargo-fmt over the
//! workspace's targets, with cargo-fmt's options, `--check` diffs and exit codes.

/// `args` are those after `fmt`. Returns the status to exit with.
pub(super) fn command(args: &[String]) -> i32 {
    if names_files(args) {
        blaze_fmt::cli::main(args.to_vec())
    } else {
        blaze_fmt::cargo_fmt::main(args.to_vec())
    }
}

/// Whether `args` name source files before `--`. cargo-fmt takes no positional argument, and
/// none of its values (a package, a manifest, a message format) ends in `.rs`.
fn names_files(args: &[String]) -> bool {
    args.iter().take_while(|it| *it != "--").any(|it| !it.starts_with('-') && it.ends_with(".rs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn files_are_rustfmt_and_the_rest_cargo_fmt() {
        assert!(names_files(&words("src/main.rs")));
        assert!(names_files(&words("--check --edition 2021 a.rs b.rs")));
        assert!(!names_files(&words("--check -p ripgrep")));
        assert!(!names_files(&words("--all -- --config max_width=60 x.rs")));
        assert!(!names_files(&words("--manifest-path a/Cargo.toml")));
        assert!(!names_files(&[]));
    }
}
