//! rustc's dep-info files, which cargo has it write next to what it builds: rules of make,
//! `target: file file`, then a rule of no files for each file, then comments
//! `# env-dep:NAME=value` for the environment variables the crate read. There is no crate for
//! the format: cargo has its own reader, which is not a library.
//!
//! The escapes are rustc's, which are fewer than make's: a space in a file's name is written
//! `\ `, and nothing else is. `$`, `#` and a backslash anywhere else are themselves, as they
//! are to cargo: a Windows path is full of backslashes, and a directory may be called `$$`.
//! A variable's name and value have a backslash, a newline and a carriage return written as
//! `\\`, `\n` and `\r`, to stay on one line.

/// What a dep-info file says.
#[derive(Default, Debug, PartialEq)]
pub(crate) struct DepInfo {
    /// The files of the first rule, in its order: rustc gives every target the same ones,
    /// and names the root of the crate first.
    pub(crate) files: Vec<String>,
    /// The variables read, with the value each had, `None` for one that was not set.
    pub(crate) env: Vec<(String, Option<String>)>,
}

impl DepInfo {
    /// Reads the text of a dep-info file. What a file cut short has of its last path is not a
    /// path.
    pub(crate) fn parse(text: &str) -> DepInfo {
        let mut info = DepInfo::default();
        let mut ruled = false;
        for line in text.lines() {
            if let Some(variable) = line.strip_prefix("# env-dep:") {
                let (name, value) = match variable.split_once('=') {
                    Some((name, value)) => (name, Some(unescaped_variable(value))),
                    None => (variable, None),
                };
                info.env.push((unescaped_variable(name), value));
                continue;
            }
            if ruled || line.starts_with('#') {
                continue;
            }
            // A target's name is not escaped: its files start after the first `: `, as cargo
            // reads them.
            let Some((_, files)) = line.split_once(": ") else { continue };
            ruled = true;
            info.files = files_of(files);
        }
        info
    }
}

/// The files of a rule, which are apart by a space that has no backslash before it.
fn files_of(rule: &str) -> Vec<String> {
    let mut files = Vec::new();
    let mut file = String::new();
    let mut chars = rule.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&' ') => {
                chars.next();
                file.push(' ');
            }
            // The escape of a space the line was cut before.
            '\\' if chars.peek().is_none() => return files,
            ' ' if !file.is_empty() => files.push(std::mem::take(&mut file)),
            ' ' => {}
            c => file.push(c),
        }
    }
    if !file.is_empty() {
        files.push(file);
    }
    files
}

/// `path` as a file of a rule: as rustc writes it.
pub(crate) fn escaped(path: &str) -> String {
    path.replace(' ', "\\ ")
}

/// A variable's name or value as rustc writes it after `# env-dep:`.
pub(crate) fn escaped_variable(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r")
}

/// A variable's name or value as it was before rustc escaped it. A backslash before anything
/// else is kept.
fn unescaped_variable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let escaped = match (c, chars.peek()) {
            ('\\', Some('\\')) => '\\',
            ('\\', Some('n')) => '\n',
            ('\\', Some('r')) => '\r',
            _ => {
                out.push(c);
                continue;
            }
        };
        chars.next();
        out.push(escaped);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_are_escaped_and_nothing_else() {
        // What rustc 1.99 writes of `/w/a b $$ #c 'q'/m n.rs`, read with `RJ` set to
        // `x\y`, a newline and `z`.
        let text = "/w/a b $$ #c 'q'/out dir.d: /w/a\\ b\\ $$\\ #c\\ 'q'/m\\ n.rs /w/gen.rs\n\n\
                    /w/a b $$ #c 'q'/out dir: /w/a\\ b\\ $$\\ #c\\ 'q'/m\\ n.rs /w/gen.rs\n\n\
                    /w/a\\ b\\ $$\\ #c\\ 'q'/m\\ n.rs:\n/w/gen.rs:\n\n\
                    # env-dep:RJ=x\\\\y\\nz\n# env-dep:UNSET\n# checksum:abc\n";
        let info = DepInfo::parse(text);
        assert_eq!(info.files, ["/w/a b $$ #c 'q'/m n.rs", "/w/gen.rs"]);
        assert_eq!(
            info.env,
            [("RJ".to_owned(), Some("x\\y\nz".to_owned())), ("UNSET".to_owned(), None)]
        );
    }

    #[test]
    fn a_variable_reads_back() {
        let value = "x\\y\nz\r";
        assert_eq!(escaped_variable(value), "x\\\\y\\nz\\r");
        let info =
            DepInfo::parse(&format!("a: /b.rs\n\n# env-dep:V={}\n", escaped_variable(value)));
        assert_eq!(info.env, [("V".to_owned(), Some(value.to_owned()))]);
    }

    #[test]
    fn spaces_together_and_at_the_ends() {
        let info = DepInfo::parse("t.d: \\ lead a\\ \\ b trail\\  last\n");
        assert_eq!(info.files, [" lead", "a  b", "trail ", "last"]);
    }

    #[test]
    fn a_windows_path_keeps_its_backslashes() {
        let info = DepInfo::parse("C:\\t\\a.d: C:\\w\\my\\ app\\src\\lib.rs C:\\w\\#x\\b.rs\n");
        assert_eq!(info.files, ["C:\\w\\my app\\src\\lib.rs", "C:\\w\\#x\\b.rs"]);
    }

    #[test]
    fn what_is_written_reads_back() {
        let path = "/tmp/sp ace/two  spaces/$$/#/x\\y.rs";
        let info = DepInfo::parse(&format!("a: {} /b.rs\n", escaped(path)));
        assert_eq!(info.files, [path, "/b.rs"]);
    }

    #[test]
    fn empty_or_cut_short() {
        assert_eq!(DepInfo::parse(""), DepInfo::default());
        assert_eq!(DepInfo::parse("/t/a.d:"), DepInfo::default());
        assert_eq!(DepInfo::parse("/t/a.d: src/a.rs src/cut\\").files, ["src/a.rs"]);
    }
}
