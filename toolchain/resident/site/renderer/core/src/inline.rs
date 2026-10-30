//! Inline markup in a text field (content/schema.flecs), as render/inline.ts: `code`, **strong**, *emphasis*,
//! [label](url), [[name]] and [label][name] for a reference under `refs`. Everything else is text, escaped. An
//! unknown reference or a link that is not http(s), / or # is an error.

use std::sync::LazyLock;

use regex_lite::Regex;

use crate::js;
use crate::render::Served;
use crate::world::{Id, World};

pub fn esc(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    for c in t.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

// A constant pattern, compiled once; not state that changes.
static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    let s = js::SPACE_CLASS;
    // JS `.` stops at any line terminator; JS `\w` is ASCII, as regex-lite's is.
    let dot = "[^\n\r\u{2028}\u{2029}]";
    Regex::new(&format!(
        r"`([^`]+)`|\*\*({dot}+?)\*\*|\*({dot}+?)\*|\[\[([\w-]+)\]\]|\[([^\]]+)\]\[([\w-]+)\]|\[([^\]]+)\]\(([^){s}]+)\)"
    ))
    .expect("TOKEN compiles")
});

fn safe_href(h: &str) -> bool {
    h.starts_with("http://")
        || h.starts_with("https://")
        || h.starts_with('/')
        || h.starts_with('#')
}

/// The references a page can name: `refs`'s children, by name.
pub struct Refs<'w> {
    pub world: &'w World,
    pub parent: Option<Id>,
    /// Where the site is served: a link to a site path (`/rfcs/x`) goes under its base.
    pub served: &'w Served,
}

impl Refs<'_> {
    pub fn get(&self, name: &str) -> Option<Id> {
        let p = self.parent?;
        self.world.get(p).children.iter().copied().find(|c| self.world.get(*c).name == name)
    }
}

/// `text` as HTML. `cited` collects the references it names, in order of first use.
pub fn inline(text: &str, refs: &Refs, wher: &str, cited: &mut Vec<Id>) -> Result<String, String> {
    let mut html = String::new();
    let mut last = 0;
    for m in TOKEN.captures_iter(text) {
        let whole = m.get(0).expect("group 0");
        html += &esc(&text[last..whole.start()]);
        last = whole.end();
        let g = |i: usize| m.get(i).map(|x| x.as_str());
        if let Some(code) = g(1) {
            html += &format!("<code>{}</code>", esc(code));
        } else if let Some(strong) = g(2) {
            html += &format!("<strong>{}</strong>", inline(strong, refs, wher, cited)?);
        } else if let Some(em) = g(3) {
            html += &format!("<em>{}</em>", inline(em, refs, wher, cited)?);
        } else if let Some(label) = g(7) {
            let href = g(8).expect("label has href");
            if !safe_href(href) {
                return Err(format!("{wher}: link {href} is not http(s), / or #"));
            }
            html += &format!(
                "<a href=\"{}\">{}</a>",
                esc(&refs.served.href(href)),
                inline(label, refs, wher, cited)?
            );
        } else {
            let name = g(4).or(g(6)).expect("a reference");
            let r = refs.get(name).ok_or_else(|| format!("{wher}: no reference refs.{name}"))?;
            if !cited.contains(&r) {
                cited.push(r);
            }
            let v = refs
                .world
                .get(r)
                .value("Ref")
                .ok_or_else(|| format!("{wher}: refs.{name} is not a Ref"))?;
            let shown = match g(5) {
                Some(label) => inline(label, refs, wher, cited)?,
                None => esc(&js::string(v.get("label"))),
            };
            html += &format!(
                "<a href=\"{}\">{shown}</a>",
                esc(&refs.served.href(&js::string(v.get("href"))))
            );
        }
    }
    Ok(html + &esc(&text[last..]))
}

/// `text` with its inline markup taken out, for the spoken version (`spoken.rs`): `code` keeps its text (`say_code`
/// makes it sayable), **strong** and *emphasis* their words, a link its label, a reference its label (or the given
/// one). Errors as `inline` does.
pub fn plain(text: &str, refs: &Refs, wher: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut last = 0;
    for m in TOKEN.captures_iter(text) {
        let whole = m.get(0).expect("group 0");
        out += &text[last..whole.start()];
        last = whole.end();
        let g = |i: usize| m.get(i).map(|x| x.as_str());
        if let Some(code) = g(1) {
            out += &say_code(code);
        } else if let Some(inner) = g(2).or(g(3)) {
            out += &plain(inner, refs, wher)?;
        } else if let Some(label) = g(7) {
            out += &plain(label, refs, wher)?;
        } else {
            let name = g(4).or(g(6)).expect("a reference");
            let r = refs.get(name).ok_or_else(|| format!("{wher}: no reference refs.{name}"))?;
            out += &match g(5) {
                Some(label) => plain(label, refs, wher)?,
                None => said_label(&js::string(
                    refs.world.get(r).value("Ref").and_then(|v| v.get("label")),
                )),
            };
        }
    }
    Ok(out + &text[last..])
}

/// A reference's label as said in a sentence: an identifier before a colon is dropped (`rust-lang/rust#24292: JoinGuard
/// is unsound ...` is "JoinGuard is unsound ...", `blaze: the function cache` is "the function cache").
fn said_label(label: &str) -> String {
    match label.split_once(": ") {
        Some((head, rest)) if !head.contains(' ') && !rest.is_empty() => rest.to_string(),
        _ => label.to_string(),
    }
}

/// An inline code span as words a voice can say: paths and generics read out (`mem::forget` is "mem forget",
/// `Rc<T>` is "Rc of T", `!Forget` is "not Forget"), punctuation that is only syntax dropped.
pub fn say_code(code: &str) -> String {
    let c = code.trim();
    match c {
        "/" => return "slash".into(),
        "+" => return "plus".into(),
        _ => {}
    }
    // A closure's parameters (`thread::scope(|s| ...)`) are not said; other pipes are alternatives (`build|run`).
    let c = match c.find("(|").and_then(|i| c[i + 2..].find('|').map(|j| (i, i + 2 + j))) {
        Some((open, close)) => format!("{}({}", &c[..open], &c[close + 1..]),
        None => c.to_string(),
    };
    let mut s = c
        .replace("...", " ")
        .replace('|', " or ")
        .replace("::", " ")
        .replace("->", " returns ")
        .replace("#[", " ")
        .replace("&mut ", "mutable reference to ")
        .replace('<', " of ")
        .replace('/', " slash ");
    if let Some(rest) = s.strip_prefix('!') {
        s = format!("not {rest}");
    }
    let s: String = s
        .chars()
        .map(|ch| match ch {
            '_' | '>' | '[' | ']' | '{' | '}' | '(' | ')' | '`' | '&' | '*' | ';' | '#' | '"'
            | '\\' => ' ',
            c => c,
        })
        .collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
