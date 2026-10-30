//! A macro call passed as an `$e:expr` and written as `$e;` is a macro in statement position:
//! every statement it expands to is there, not the first alone.

use std::fmt::Write;

macro_rules! emit {
    ($out:ident, $text:expr) => {
        $out.write_str("[")?;
        $out.write_str($text)?;
        $out.write_char(']')?;
    };
}

macro_rules! when {
    ($flag:expr, $then:expr) => {
        if $flag {
            $then;
        }
    };
    ($name:ident => $value:expr, $then:expr) => {
        if let Some($name) = $value {
            $then;
        }
    };
}

fn describe(mut out: impl Write, bold: bool, faint: bool, font: Option<u8>) -> std::fmt::Result {
    when!(bold, emit!(out, "1"));
    when!(faint, emit!(out, "2"));
    when!(font => font, out.write_fmt(format_args!("{font}"))?);
    Ok(())
}

pub fn run() -> Vec<String> {
    let mut all = Vec::new();
    for (bold, faint, font) in [(true, false, None), (true, true, Some(7)), (false, false, Some(1))] {
        let mut text = String::new();
        describe(&mut text, bold, faint, font).unwrap();
        all.push(text);
    }
    all
}
