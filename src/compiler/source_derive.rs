//! Built-in `#[derive(...)]` expansion: generates ordinary Rust source impls for
//! Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default and Debug from a
//! struct or enum declaration. The generated text is inserted after the item
//! during source preparation (as synthetic provenance) and is then parsed,
//! resolved and checked exactly like hand-written impls. Generated code uses
//! fully qualified `<FieldTy as Trait>::method` calls so it never depends on
//! method-probe or operator lowering, and every type parameter receives the
//! derived trait as a bound, as stock rustc does.
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Lifetime(String),
    Punct(char),
    Literal(String),
    Group(char, Vec<Tok>),
}

fn lex(text: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut stack: Vec<(char, Vec<Tok>)> = vec![(' ', Vec::new())];
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let mut depth = 0usize;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            continue;
        }
        let top = &mut stack.last_mut().unwrap().1;
        if c == '_' || c.is_alphanumeric() {
            let start = i;
            while i < chars.len() && (chars[i] == '_' || chars[i].is_alphanumeric()) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if c.is_ascii_digit() {
                top.push(Tok::Literal(word));
            } else if word == "r" && chars.get(i) == Some(&'#') {
                // raw identifier r#name
                let s2 = i + 1;
                i += 1;
                while i < chars.len() && (chars[i] == '_' || chars[i].is_alphanumeric()) {
                    i += 1;
                }
                let name: String = chars[s2..i].iter().collect();
                top.push(Tok::Ident(format!("r#{name}")));
            } else {
                top.push(Tok::Ident(word));
            }
            continue;
        }
        if c == '\'' {
            // lifetime or char literal
            if chars.get(i + 2) == Some(&'\'') || chars.get(i + 1) == Some(&'\\') {
                let start = i;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    if chars[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
                top.push(Tok::Literal(chars[start..i].iter().collect()));
            } else {
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i] == '_' || chars[i].is_alphanumeric()) {
                    i += 1;
                }
                top.push(Tok::Lifetime(chars[start..i].iter().collect()));
            }
            continue;
        }
        if c == '"' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            top.push(Tok::Literal(chars[start..i].iter().collect()));
            continue;
        }
        match c {
            '(' | '[' | '{' => stack.push((c, Vec::new())),
            ')' | ']' | '}' => {
                let (open, body) = stack.pop().ok_or("unbalanced derive input")?;
                if stack.is_empty() {
                    return Err("unbalanced derive input".into());
                }
                stack.last_mut().unwrap().1.push(Tok::Group(open, body));
            }
            _ => top.push(Tok::Punct(c)),
        }
        i += 1;
    }
    if stack.len() != 1 {
        return Err("unbalanced derive input".into());
    }
    Ok(stack.pop().unwrap().1)
}

fn render(tokens: &[Tok], out: &mut String) {
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Tok::Ident(s) | Tok::Literal(s) | Tok::Lifetime(s) => {
                out.push_str(s);
                out.push(' ');
            }
            Tok::Punct(c) => {
                out.push(*c);
                // keep `::`, `->` and `=>` joint; otherwise separate
                let joint = matches!(
                    (c, tokens.get(index + 1)),
                    (':', Some(Tok::Punct(':'))) | ('-', Some(Tok::Punct('>')))
                );
                if !joint {
                    out.push(' ');
                }
            }
            Tok::Group(open, body) => {
                out.push(*open);
                render(body, out);
                out.push(match open {
                    '(' => ')',
                    '[' => ']',
                    _ => '}',
                });
                out.push(' ');
            }
        }
    }
}
fn text(tokens: &[Tok]) -> String {
    let mut out = String::new();
    render(tokens, &mut out);
    out.trim_end().to_string()
}

/// Split by top-level commas, tracking `<...>` nesting (but not `->`).
fn split_commas(tokens: &[Tok]) -> Vec<Vec<Tok>> {
    let mut parts = Vec::new();
    let mut current = Vec::new();
    let mut angle = 0i32;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Tok::Punct('<') => angle += 1,
            Tok::Punct('>') => {
                let arrow =
                    index > 0 && matches!(tokens[index - 1], Tok::Punct('-') | Tok::Punct('='));
                if !arrow {
                    angle -= 1;
                }
            }
            Tok::Punct(',') if angle == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(token.clone());
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

fn strip_attrs_and_vis(mut tokens: &[Tok]) -> (&[Tok], bool) {
    let mut default = false;
    loop {
        match tokens {
            [Tok::Punct('#'), Tok::Group('[', body), rest @ ..] => {
                if matches!(body.as_slice(), [Tok::Ident(name)] if name == "default") {
                    default = true;
                }
                tokens = rest;
            }
            [Tok::Ident(p), Tok::Group('(', _), rest @ ..] if p == "pub" => tokens = rest,
            [Tok::Ident(p), rest @ ..] if p == "pub" => tokens = rest,
            _ => return (tokens, default),
        }
    }
}

#[derive(Debug)]
enum Fields {
    Unit,
    Tuple(Vec<String>),
    Named(Vec<(String, String)>),
}

fn fields(group: Option<&Tok>) -> Result<Fields, String> {
    match group {
        None => Ok(Fields::Unit),
        Some(Tok::Group('(', body)) => Ok(Fields::Tuple(
            split_commas(body)
                .iter()
                .map(|part| text(strip_attrs_and_vis(part).0))
                .collect(),
        )),
        Some(Tok::Group('{', body)) => {
            let mut out = Vec::new();
            for part in split_commas(body) {
                let (part, _) = strip_attrs_and_vis(&part);
                match part {
                    [Tok::Ident(name), Tok::Punct(':'), ty @ ..] if !ty.is_empty() => {
                        out.push((name.clone(), text(ty)));
                    }
                    _ => return Err("unsupported derive input field syntax".into()),
                }
            }
            Ok(Fields::Named(out))
        }
        Some(_) => Err("unsupported derive input fields".into()),
    }
}

struct Variant {
    name: String,
    fields: Fields,
    default: bool,
}

struct Input {
    name: String,
    /// Declared generic parameters as (kind, name, bounds-with-colon text).
    params: Vec<(char, String, String)>,
    where_clause: String,
    body: Body,
}
enum Body {
    Struct(Fields),
    Enum(Vec<Variant>),
}

fn parse(item: &str) -> Result<Input, String> {
    let tokens = lex(item)?;
    let (mut rest, _) = strip_attrs_and_vis(&tokens);
    let kind = match rest {
        [Tok::Ident(k), more @ ..] if k == "struct" || k == "enum" => {
            rest = more;
            k.clone()
        }
        [Tok::Ident(k), ..] if k == "union" => {
            return Err("unsupported: derive on unions is unsupported".into());
        }
        _ => return Err("`derive` may only be applied to `struct`s, `enum`s and `union`s".into()),
    };
    let name = match rest {
        [Tok::Ident(n), more @ ..] => {
            rest = more;
            n.clone()
        }
        _ => return Err("expected derive item name".into()),
    };
    let mut params = Vec::new();
    if let [Tok::Punct('<'), ..] = rest {
        let mut depth = 0i32;
        let mut end = 0;
        for (index, token) in rest.iter().enumerate() {
            match token {
                Tok::Punct('<') => depth += 1,
                Tok::Punct('>') if !matches!(rest[index - 1], Tok::Punct('-')) => {
                    depth -= 1;
                    if depth == 0 {
                        end = index;
                        break;
                    }
                }
                _ => {}
            }
        }
        if end == 0 {
            return Err("unclosed derive generics".into());
        }
        for part in split_commas(&rest[1..end]) {
            let (part, _) = strip_attrs_and_vis(&part);
            // drop `= default` from the parameter (impl generics cannot have defaults)
            let eq = part
                .iter()
                .position(|t| *t == Tok::Punct('='))
                .unwrap_or(part.len());
            let part = &part[..eq];
            match part {
                [Tok::Lifetime(l), bounds @ ..] => params.push(('\'', l.clone(), text(bounds))),
                [Tok::Ident(c), Tok::Ident(n), bounds @ ..] if c == "const" => {
                    params.push(('c', n.clone(), text(bounds)))
                }
                [Tok::Ident(n), bounds @ ..] => params.push(('t', n.clone(), text(bounds))),
                _ => return Err("unsupported derive generic parameter".into()),
            }
        }
        rest = &rest[end + 1..];
    }
    let mut where_clause = String::new();
    let mut take_where = |rest: &mut &[Tok]| {
        if let [Tok::Ident(w), more @ ..] = *rest
            && w == "where"
        {
            let end = more
                .iter()
                .position(|t| matches!(t, Tok::Group('{', _)) || *t == Tok::Punct(';'))
                .unwrap_or(more.len());
            where_clause = text(&more[..end]);
            *rest = &more[end..];
        }
    };
    let body = if kind == "struct" {
        match rest {
            [Tok::Group('(', _), ..] => {
                let group = rest[0].clone();
                rest = &rest[1..];
                take_where(&mut rest);
                Body::Struct(fields(Some(&group))?)
            }
            _ => {
                take_where(&mut rest);
                Body::Struct(fields(
                    rest.first().filter(|t| matches!(t, Tok::Group(..))),
                )?)
            }
        }
    } else {
        take_where(&mut rest);
        let Some(Tok::Group('{', body)) = rest.first() else {
            return Err("expected enum body".into());
        };
        let mut variants = Vec::new();
        for part in split_commas(body) {
            let (part, default) = strip_attrs_and_vis(&part);
            let [Tok::Ident(name), more @ ..] = part else {
                return Err("unsupported derive enum variant".into());
            };
            let group = more
                .first()
                .filter(|t| matches!(t, Tok::Group('(' | '{', _)));
            variants.push(Variant {
                name: name.clone(),
                fields: fields(group)?,
                default,
            });
        }
        Body::Enum(variants)
    };
    Ok(Input {
        name,
        params,
        where_clause,
        body,
    })
}

impl Input {
    fn header(&self, trait_path: &str, bound: Option<&str>) -> String {
        let (g, a) = self.generics(bound);
        let mut out = format!("impl{g} {trait_path} for {}{a}", self.name);
        if !self.where_clause.is_empty() {
            let _ = write!(out, " where {}", self.where_clause);
        }
        out
    }
    /// Declared generics (with `bound` added to every type parameter) and
    /// the matching argument list.
    fn generics(&self, bound: Option<&str>) -> (String, String) {
        let mut generics = Vec::new();
        let mut args = Vec::new();
        for (kind, name, bounds) in &self.params {
            match kind {
                '\'' => {
                    generics.push(format!("{name} {bounds}"));
                    args.push(name.clone());
                }
                'c' => {
                    generics.push(format!("const {name} {bounds}"));
                    args.push(name.clone());
                }
                _ => {
                    let mut g = format!("{name} {bounds}");
                    if let Some(bound) = bound {
                        if bounds.is_empty() {
                            g.push(':');
                        } else {
                            g.push('+');
                        }
                        g.push_str(bound);
                    }
                    generics.push(g);
                    args.push(name.clone());
                }
            }
        }
        if generics.is_empty() {
            (String::new(), String::new())
        } else {
            (
                format!("<{}>", generics.join(", ")),
                format!("<{}>", args.join(", ")),
            )
        }
    }
    fn self_ty(&self) -> String {
        if self.params.is_empty() {
            self.name.clone()
        } else {
            let args: Vec<_> = self.params.iter().map(|p| p.1.clone()).collect();
            format!("{}<{}>", self.name, args.join(", "))
        }
    }
}

/// Pattern binding fields with a prefix (`ref a0, ref a1`), plus the
/// (binding, type) list.
fn pattern(path: &str, fields: &Fields, prefix: &str) -> (String, Vec<(String, String)>) {
    match fields {
        Fields::Unit => (path.to_string(), Vec::new()),
        Fields::Tuple(types) => {
            let binds: Vec<_> = (0..types.len()).map(|i| format!("{prefix}{i}")).collect();
            let pat = format!(
                "{path}({})",
                binds
                    .iter()
                    .map(|b| format!("ref {b}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            (pat, binds.into_iter().zip(types.iter().cloned()).collect())
        }
        Fields::Named(named) => {
            let binds: Vec<_> = (0..named.len()).map(|i| format!("{prefix}{i}")).collect();
            let pat = format!(
                "{path} {{ {} }}",
                named
                    .iter()
                    .zip(&binds)
                    .map(|((n, _), b)| format!("{n}: ref {b}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            (
                pat,
                binds
                    .into_iter()
                    .zip(named.iter().map(|(_, t)| t.clone()))
                    .collect(),
            )
        }
    }
}
fn construct(path: &str, fields: &Fields, values: &[String]) -> String {
    match fields {
        Fields::Unit => path.to_string(),
        Fields::Tuple(_) => format!("{path}({})", values.join(", ")),
        Fields::Named(named) => format!(
            "{path} {{ {} }}",
            named
                .iter()
                .zip(values)
                .map(|((n, _), v)| format!("{n}: {v}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Uniform view: a list of (constructor path, fields) "variants". Structs are
/// one variant named by the type itself.
fn variants(input: &Input) -> Vec<(String, &Fields)> {
    match &input.body {
        Body::Struct(f) => vec![(input.name.clone(), f)],
        Body::Enum(vs) => vs
            .iter()
            .map(|v| (format!("{}::{}", input.name, v.name), &v.fields))
            .collect(),
    }
}

const ORDERING: &str = "std::cmp::Ordering";

fn gen_clone(input: &Input, copy: bool) -> String {
    let header = input.header("Clone", Some("Clone"));
    let ty = input.self_ty();
    if copy && input.params.iter().all(|p| p.0 == '\'') {
        return format!("{header} {{ fn clone(&self) -> {ty} {{ *self }} }}\n");
    }
    let mut arms = String::new();
    for (path, fields) in variants(input) {
        let (pat, binds) = pattern(&path, fields, "f");
        let values: Vec<_> = binds
            .iter()
            .map(|(b, t)| format!("<{t} as Clone>::clone({b})"))
            .collect();
        let _ = write!(arms, "&{pat} => {}, ", construct(&path, fields, &values));
    }
    format!("{header} {{ fn clone(&self) -> {ty} {{ match self {{ {arms} }} }} }}\n")
}

fn gen_eq_chain(binds_a: &[(String, String)], binds_b: &[(String, String)]) -> String {
    let mut out = String::from("true");
    // Stock expands to `*a == *b`, so a field without PartialEq is E0369
    // (binary operation cannot be applied), not E0277.
    for ((a, _), (b, _)) in binds_a.iter().zip(binds_b) {
        let _ = write!(out, " && *{a} == *{b}");
    }
    out
}

/// `Eq` has no methods, so stock asserts every field type is `Eq`
/// (`AssertParamIsEq`). The same obligation is expressed as an ordinary
/// generic requirement function called once per field type from a checker
/// that carries the impl's generics, so missing field impls are E0277 even
/// when the type is never used.
fn gen_eq(input: &Input) -> String {
    let mut out = format!(
        "{} {{}}\n",
        input.header("std::cmp::Eq", Some("std::cmp::Eq"))
    );
    let mut calls = String::new();
    let require = format!(
        "__justrust_derive_eq_require_{}",
        input.name.trim_start_matches("r#")
    );
    for (_, fields) in variants(input) {
        let types: Vec<&String> = match fields {
            Fields::Unit => Vec::new(),
            Fields::Tuple(t) => t.iter().collect(),
            Fields::Named(n) => n.iter().map(|(_, t)| t).collect(),
        };
        for ty in types {
            let _ = write!(calls, "{require}::<{ty}>(); ");
        }
    }
    if calls.is_empty() {
        return out;
    }
    let (g, _) = input.generics(Some("std::cmp::Eq"));
    let where_clause = if input.where_clause.is_empty() {
        String::new()
    } else {
        format!(" where {}", input.where_clause)
    };
    let _ = write!(
        out,
        "fn {require}<X: std::cmp::Eq>() {{}}\nfn __justrust_derive_eq_assert_{}{g}(){where_clause} {{ {calls}}}\n",
        input.name.trim_start_matches("r#")
    );
    out
}

fn gen_partial_eq(input: &Input) -> String {
    let header = input.header("std::cmp::PartialEq", Some("std::cmp::PartialEq"));
    let ty = input.self_ty();
    let vs = variants(input);
    let mut arms = String::new();
    for (path, fields) in &vs {
        let (pa, ba) = pattern(path, fields, "a");
        let (pb, bb) = pattern(path, fields, "b");
        let _ = write!(arms, "(&{pa}, &{pb}) => {}, ", gen_eq_chain(&ba, &bb));
    }
    if vs.len() > 1 {
        arms.push_str("_ => false, ");
    }
    let body = if vs.is_empty() {
        "match *self {}".to_string()
    } else {
        format!("match (self, other) {{ {arms} }}")
    };
    format!("{header} {{ fn eq(&self, other: &{ty}) -> bool {{ {body} }} }}\n")
}

fn variant_index(input: &Input, var: &str) -> String {
    let mut arms = String::new();
    for (index, (path, fields)) in variants(input).iter().enumerate() {
        let pat = match fields {
            Fields::Unit => path.clone(),
            Fields::Tuple(_) => format!("{path}(..)"),
            Fields::Named(_) => format!("{path} {{ .. }}"),
        };
        let _ = write!(arms, "&{pat} => {index}isize, ");
    }
    format!("match {var} {{ {arms} }}")
}

/// Lexicographic comparison; `partial` selects PartialOrd::partial_cmp
/// (Option<Ordering>) instead of Ord::cmp.
fn gen_ord(input: &Input, partial: bool) -> String {
    let (trait_path, method, ret) = if partial {
        (
            "std::cmp::PartialOrd",
            "partial_cmp",
            format!("std::option::Option<{ORDERING}>"),
        )
    } else {
        ("std::cmp::Ord", "cmp", ORDERING.to_string())
    };
    let header = input.header(trait_path, Some(trait_path));
    let ty = input.self_ty();
    let equal = if partial {
        format!("std::option::Option::Some({ORDERING}::Equal)")
    } else {
        format!("{ORDERING}::Equal")
    };
    let wrap = |o: &str| {
        if partial {
            format!("std::option::Option::Some({o})")
        } else {
            o.to_string()
        }
    };
    let vs = variants(input);
    let mut body = String::new();
    if vs.len() > 1 {
        let _ = write!(
            body,
            "let li: isize = {}; let ri: isize = {}; if li < ri {{ return {}; }} if li > ri {{ return {}; }} ",
            variant_index(input, "self"),
            variant_index(input, "other"),
            wrap(&format!("{ORDERING}::Less")),
            wrap(&format!("{ORDERING}::Greater")),
        );
    }
    let mut arms = String::new();
    for (path, fields) in &vs {
        let (pa, ba) = pattern(path, fields, "a");
        let (pb, bb) = pattern(path, fields, "b");
        let mut chain = String::new();
        for ((a, t), (b, _)) in ba.iter().zip(&bb) {
            let _ = write!(
                chain,
                "match <{t} as {trait_path}>::{method}({a}, {b}) {{ {equal} => {{}} c => return c, }} "
            );
        }
        let _ = write!(arms, "(&{pa}, &{pb}) => {{ {chain} {equal} }}, ");
    }
    if vs.len() > 1 {
        let _ = write!(arms, "_ => {equal}, ");
    }
    if vs.is_empty() {
        body.push_str("match *self {}");
    } else {
        let _ = write!(body, "match (self, other) {{ {arms} }}");
    }
    format!("{header} {{ fn {method}(&self, other: &{ty}) -> {ret} {{ {body} }} }}\n")
}

fn gen_hash(input: &Input) -> String {
    let header = input.header("std::hash::Hash", Some("std::hash::Hash"));
    // Fresh method binder for the hasher: never equal to any type, lifetime
    // or const parameter of the input, so a user `H` cannot collide.
    let mut hasher = String::from("__JrHasher");
    while input.params.iter().any(|(_, name, _)| *name == hasher) {
        hasher.push('_');
    }
    let vs = variants(input);
    let mut body = String::new();
    if vs.len() > 1 {
        let _ = write!(
            body,
            "let d: isize = {}; <{hasher} as std::hash::Hasher>::write_isize(&mut *state, d); ",
            variant_index(input, "self")
        );
    }
    let mut arms = String::new();
    for (path, fields) in &vs {
        let (pa, ba) = pattern(path, fields, "a");
        let mut chain = String::new();
        for (a, t) in &ba {
            let _ = write!(chain, "<{t} as std::hash::Hash>::hash({a}, &mut *state); ");
        }
        let _ = write!(arms, "&{pa} => {{ {chain} }}, ");
    }
    if vs.is_empty() {
        body.push_str("match *self {}");
    } else {
        let _ = write!(body, "match self {{ {arms} }}");
    }
    format!(
        "{header} {{ fn hash<{hasher}: std::hash::Hasher>(&self, state: &mut {hasher}) {{ {body} }} }}\n"
    )
}

fn gen_default(input: &Input) -> Result<String, String> {
    let header = input.header("std::default::Default", Some("std::default::Default"));
    let ty = input.self_ty();
    let (path, fields) =
        match &input.body {
            Body::Struct(f) => (input.name.clone(), f),
            Body::Enum(vs) => {
                let defaults: Vec<_> = vs.iter().filter(|v| v.default).collect();
                match defaults.as_slice() {
                    [v] => {
                        if !matches!(v.fields, Fields::Unit) {
                            return Err(
                                "the `#[default]` attribute may only be used on unit enum variants"
                                    .into(),
                            );
                        }
                        (format!("{}::{}", input.name, v.name), &v.fields)
                    }
                    [] => return Err(
                        "no default declared: `#[derive(Default)]` on enum with no `#[default]`"
                            .into(),
                    ),
                    _ => return Err("multiple declared defaults".into()),
                }
            }
        };
    let types: Vec<String> = match fields {
        Fields::Unit => Vec::new(),
        Fields::Tuple(t) => t.clone(),
        Fields::Named(n) => n.iter().map(|(_, t)| t.clone()).collect(),
    };
    let values: Vec<_> = types
        .iter()
        .map(|t| format!("<{t} as std::default::Default>::default()"))
        .collect();
    Ok(format!(
        "{header} {{ fn default() -> {ty} {{ {} }} }}\n",
        construct(&path, fields, &values)
    ))
}

fn gen_debug(input: &Input) -> String {
    let header = input.header("std::fmt::Debug", Some("std::fmt::Debug"));
    let mut arms = String::new();
    for (path, fields) in variants(input) {
        let short = path.rsplit("::").next().unwrap_or(&path).to_string();
        let (pa, ba) = pattern(&path, fields, "a");
        let expr = match fields {
            Fields::Unit => format!("f.write_str(\"{short}\")"),
            Fields::Tuple(_) => {
                let mut e = format!("f.debug_tuple(\"{short}\")");
                for (a, _) in &ba {
                    let _ = write!(e, ".field({a})");
                }
                e + ".finish()"
            }
            Fields::Named(named) => {
                let mut e = format!("f.debug_struct(\"{short}\")");
                for ((name, _), (a, _)) in named.iter().zip(&ba) {
                    let name = name.trim_start_matches("r#");
                    let _ = write!(e, ".field(\"{name}\", {a})");
                }
                e + ".finish()"
            }
        };
        let _ = write!(arms, "&{pa} => {expr}, ");
    }
    let body = if arms.is_empty() {
        "match *self {}".to_string()
    } else {
        format!("match self {{ {arms} }}")
    };
    format!(
        "{header} {{ fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {{ {body} }} }}\n"
    )
}

/// Expands one derive list against its item text. Unknown derive macros are
/// rejected explicitly (no proc-macro support yet).
pub(super) fn expand(item: &str, derives: &[String]) -> Result<String, String> {
    let input = parse(item)?;
    let copy = derives.iter().any(|d| d == "Copy");
    let mut out = String::from("\n");
    for derive in derives {
        let generated = match derive.as_str() {
            "Clone" => gen_clone(&input, copy),
            "Copy" => format!("{} {{}}\n", input.header("Copy", Some("Copy"))),
            "PartialEq" => gen_partial_eq(&input),
            "Eq" => gen_eq(&input),
            "PartialOrd" => gen_ord(&input, true),
            "Ord" => gen_ord(&input, false),
            "Hash" => gen_hash(&input),
            "Default" => gen_default(&input)?,
            "Debug" => gen_debug(&input),
            other => {
                return Err(format!(
                    "unsupported: derive macro `{other}` is unsupported (only built-in derives)"
                ));
            }
        };
        out.push_str(&generated);
    }
    Ok(out)
}

/// Rewrites `#[default]` variant attributes (consumed by derive(Default)) to
/// spaces so later attribute validation does not reject them.
pub(super) fn default_attribute_ranges(item: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let bytes = item.as_bytes();
    let mut i = 0;
    while let Some(off) = item[i..].find("#[default]") {
        let start = i + off;
        // only inside the enum body
        if item[..start].contains('{') && !bytes[..start].ends_with(b"\"") {
            out.push(start..start + "#[default]".len());
        }
        i = start + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_struct_and_enum_impls() {
        let s = expand(
            "struct P<T: Copy> { pub x: T, y: (u8, bool) }",
            &["Clone".into(), "PartialEq".into(), "Debug".into()],
        )
        .unwrap();
        assert!(s.contains("impl<T : Copy+Clone> Clone for P<T>"), "{s}");
        assert!(s.contains("&& *a0 == *b0"), "{s}");
        let e = expand(
            "enum E { A, B(i32, i32), #[default] C { x: i32 } }",
            &["Ord".into(), "Hash".into()],
        )
        .unwrap();
        assert!(e.contains("li < ri"), "{e}");
        assert!(
            e.contains("fn hash<__JrHasher: std::hash::Hasher>(&self, state: &mut __JrHasher)"),
            "{e}"
        );
        assert!(
            e.contains("<__JrHasher as std::hash::Hasher>::write_isize(&mut *state, d)"),
            "{e}"
        );
        assert!(!e.contains("DefaultHasher"), "{e}");
        let clash = expand("struct Q<__JrHasher> { x: __JrHasher }", &["Hash".into()]).unwrap();
        assert!(
            clash.contains("fn hash<__JrHasher_: std::hash::Hasher>"),
            "{clash}"
        );
        assert!(
            expand("enum E { A, #[default] B }", &["Default".into()])
                .unwrap()
                .contains("E::B")
        );
        assert!(expand("enum E { A }", &["Default".into()]).is_err());
        assert!(expand("fn f() {}", &["Clone".into()]).is_err());
        assert!(expand("struct S;", &["Serialize".into()]).is_err());
    }
}
