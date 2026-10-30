//! `#[cache]`, re-exported by `ra-jit-cache` as `ra_jit::cache`.
//!
//! The attribute moves the body of the function into a closure that takes the parameters, and
//! makes the function call that closure through `ra_jit::__private`. Under ra-jit the call
//! looks up what the arguments gave the last time the same code ran with them; built by rustc
//! alone it is a plain call of the closure. The closure is what ra-jit names the code by: its
//! body is the function's, and what it reaches is what the function reaches.

use proc_macro::{Delimiter, Group, Ident, Punct, Spacing, Span, TokenStream, TokenTree};

#[proc_macro_attribute]
pub fn cache(attr: TokenStream, item: TokenStream) -> TokenStream {
    if let Some(first) = attr.into_iter().next() {
        return error(first.span(), "#[ra_jit::cache] takes no arguments", item);
    }
    match expand(item.clone()) {
        Ok(expanded) => expanded,
        Err((span, message)) => error(span, &message, item),
    }
}

type Failed = (Span, String);

/// One parameter: what is written for it, and the name it binds.
struct Param {
    written: Vec<TokenTree>,
    name: Ident,
    ty: Vec<TokenTree>,
}

fn expand(item: TokenStream) -> Result<TokenStream, Failed> {
    let tokens: Vec<TokenTree> = item.into_iter().collect();
    let mut at = 0;
    let mut head: Vec<TokenTree> = Vec::new();
    // Attributes, the visibility and the qualifiers, up to `fn`.
    loop {
        match tokens.get(at) {
            Some(TokenTree::Ident(word)) if word.to_string() == "fn" => break,
            Some(TokenTree::Ident(word)) => {
                let word_text = word.to_string();
                if ["async", "const", "unsafe", "extern"].contains(&word_text.as_str()) {
                    return Err((
                        word.span(),
                        format!("#[ra_jit::cache] cannot cache a `{word_text} fn`"),
                    ));
                }
                head.push(TokenTree::Ident(word.clone()));
            }
            Some(other) => head.push(other.clone()),
            None => return Err((Span::call_site(), "#[ra_jit::cache] goes on a function".into())),
        }
        at += 1;
    }
    head.push(tokens[at].clone());
    at += 1;
    let Some(TokenTree::Ident(name)) = tokens.get(at) else {
        return Err((Span::call_site(), "#[ra_jit::cache] goes on a function".into()));
    };
    let name = name.clone();
    head.push(TokenTree::Ident(name.clone()));
    at += 1;
    if is_punct(tokens.get(at), '<') {
        let mut depth = 0i32;
        loop {
            let Some(token) = tokens.get(at) else {
                return Err((name.span(), "unclosed generic parameters".into()));
            };
            match angle(&tokens, at) {
                Some(true) => depth += 1,
                Some(false) => depth -= 1,
                None => {}
            }
            head.push(token.clone());
            at += 1;
            if depth == 0 {
                break;
            }
        }
    }
    let Some(TokenTree::Group(params)) =
        tokens.get(at).filter(|it| is_group(it, Delimiter::Parenthesis))
    else {
        return Err((name.span(), "expected the parameters of the function".into()));
    };
    let params_group = params.clone();
    at += 1;
    let Some(TokenTree::Group(body)) = tokens.last().filter(|it| is_group(it, Delimiter::Brace))
    else {
        return Err((name.span(), "#[ra_jit::cache] needs a function with a body".into()));
    };
    let body = body.clone();
    let rest = &tokens[at..tokens.len() - 1];
    // `-> R` and `where ..`, as written.
    let (returned, bounds) = {
        let mut returned = Vec::new();
        let mut i = 0;
        if is_punct(rest.first(), '-') && is_punct(rest.get(1), '>') {
            i = 2;
            while i < rest.len() && !is_word(&rest[i], "where") {
                returned.push(rest[i].clone());
                i += 1;
            }
        }
        (returned, rest[i..].to_vec())
    };
    if returned.iter().any(mentions_impl) {
        return Err((
            name.span(),
            "#[ra_jit::cache] cannot cache a function returning `impl Trait`".into(),
        ));
    }
    let params = parse_params(params_group.stream())?;

    let returned_ty: TokenStream =
        if returned.is_empty() { code("()") } else { returned.iter().cloned().collect() };
    let mut closure_params = TokenStream::new();
    let mut tuple_ty = TokenStream::new();
    let mut borrowed = TokenStream::new();
    let mut passed = TokenStream::new();
    for param in &params {
        closure_params.extend(param.written.iter().cloned());
        closure_params.extend(code(","));
        tuple_ty.extend(param.ty.iter().cloned());
        tuple_ty.extend(code(","));
        borrowed.extend(code("&"));
        borrowed.extend([TokenTree::Ident(param.name.clone())]);
        borrowed.extend(code(","));
        passed.extend([TokenTree::Ident(param.name.clone())]);
        passed.extend(code(","));
    }

    // The closure's body is the function's. It names the codec, so that the code that writes
    // the arguments and reads the result back is part of what ra-jit names the code by.
    let mut inner = TokenStream::new();
    inner.extend(code("let _ = ::ra_jit::__private::__ra_jit_codec::<"));
    inner.extend([group(Delimiter::Parenthesis, tuple_ty)]);
    inner.extend(code(","));
    inner.extend(returned_ty.clone());
    inner.extend(code("> as fn();"));
    inner.extend([TokenTree::Group(body.clone())]);

    let mut wrapper = TokenStream::new();
    wrapper.extend(code("let __ra_jit_body = move |"));
    wrapper.extend(closure_params);
    wrapper.extend(code("| ->"));
    wrapper.extend(returned_ty);
    wrapper.extend([group(Delimiter::Brace, inner)]);
    wrapper.extend(code(";"));
    wrapper.extend(code(
        "let __ra_jit_code = ::ra_jit::__private::__ra_jit_cache_code(&__ra_jit_body);",
    ));
    let mut key_args = code("&__ra_jit_code, &");
    key_args.extend([group(Delimiter::Parenthesis, borrowed)]);
    wrapper.extend(code("let __ra_jit_key = ::ra_jit::__private::key"));
    wrapper.extend([group(Delimiter::Parenthesis, key_args)]);
    wrapper.extend(code(";"));
    let mut call_args = code("__ra_jit_key, move || __ra_jit_body");
    call_args.extend([group(Delimiter::Parenthesis, passed.clone())]);
    let mut arms = code("::core::option::Option::Some(__ra_jit_key) => ::ra_jit::__private::call");
    arms.extend([group(Delimiter::Parenthesis, call_args)]);
    arms.extend(code(", ::core::option::Option::None => __ra_jit_body"));
    arms.extend([group(Delimiter::Parenthesis, passed)]);
    arms.extend(code(","));
    wrapper.extend(code("match __ra_jit_key"));
    wrapper.extend([group(Delimiter::Brace, arms)]);

    // The function's own parameters are moved into the closure, which binds them as written:
    // a `mut` is the closure's.
    let mut own = TokenStream::new();
    for param in &params {
        let written = match param.written.first() {
            Some(first) if is_word(first, "mut") => &param.written[1..],
            _ => &param.written[..],
        };
        own.extend(written.iter().cloned());
        own.extend(code(","));
    }
    let mut own = Group::new(Delimiter::Parenthesis, own);
    own.set_span(params_group.span());
    let mut out: TokenStream = head.into_iter().collect();
    out.extend([TokenTree::Group(own)]);
    if !returned.is_empty() {
        out.extend(code("->"));
        out.extend(returned);
    }
    out.extend(bounds);
    let mut braces = Group::new(Delimiter::Brace, wrapper);
    braces.set_span(body.span());
    out.extend([TokenTree::Group(braces)]);
    Ok(out)
}

fn parse_params(stream: TokenStream) -> Result<Vec<Param>, Failed> {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut chunks: Vec<Vec<TokenTree>> = vec![Vec::new()];
    let mut depth = 0i32;
    for (at, token) in tokens.iter().enumerate() {
        match angle(&tokens, at) {
            Some(true) => depth += 1,
            Some(false) => depth -= 1,
            None => {}
        }
        if depth == 0 && is_punct(Some(token), ',') {
            chunks.push(Vec::new());
        } else {
            chunks.last_mut().expect("one chunk").push(token.clone());
        }
    }
    let mut params = Vec::new();
    for chunk in chunks.into_iter().filter(|it| !it.is_empty()) {
        let span = chunk[0].span();
        if is_punct(chunk.first(), '#') {
            return Err((span, "#[ra_jit::cache] does not take attributes on parameters".into()));
        }
        let colon = chunk.iter().position(|it| is_punct(Some(it), ':'));
        let receiver = chunk[..colon.unwrap_or(chunk.len())].iter().any(|it| is_word(it, "self"));
        if receiver {
            return Err((
                span,
                "#[ra_jit::cache] does not take `self` yet: take `this: &Self` instead".into(),
            ));
        }
        let pattern = &chunk[..colon.unwrap_or(0)];
        let name = match pattern {
            [TokenTree::Ident(name)] => name.clone(),
            [TokenTree::Ident(word), TokenTree::Ident(name)] if word.to_string() == "mut" => {
                name.clone()
            }
            _ => {
                return Err((
                    span,
                    "#[ra_jit::cache] takes parameters written `name: Type`".into(),
                ));
            }
        };
        let colon = colon.expect("a pattern before a colon");
        // `a::b` is a path, not a name and a type.
        if let TokenTree::Punct(punct) = &chunk[colon]
            && punct.spacing() == Spacing::Joint
            && is_punct(chunk.get(colon + 1), ':')
        {
            return Err((span, "#[ra_jit::cache] takes parameters written `name: Type`".into()));
        }
        let ty = chunk[colon + 1..].to_vec();
        if ty.is_empty() {
            return Err((span, "a parameter without a type".into()));
        }
        if ty.iter().any(mentions_impl) {
            return Err((span, "#[ra_jit::cache] cannot take `impl Trait` parameters".into()));
        }
        params.push(Param { written: chunk, name, ty });
    }
    if params.len() > 16 {
        return Err((Span::call_site(), "#[ra_jit::cache] takes at most 16 parameters".into()));
    }
    Ok(params)
}

/// Whether the token at `at` opens (`Some(true)`) or closes (`Some(false)`) angle brackets: a
/// `>` of `->` does neither.
fn angle(tokens: &[TokenTree], at: usize) -> Option<bool> {
    let TokenTree::Punct(punct) = &tokens[at] else { return None };
    match punct.as_char() {
        '<' => Some(true),
        '>' => {
            let arrow = at > 0
                && matches!(&tokens[at - 1], TokenTree::Punct(it) if it.as_char() == '-' && it.spacing() == Spacing::Joint);
            (!arrow).then_some(false)
        }
        _ => None,
    }
}

fn mentions_impl(token: &TokenTree) -> bool {
    match token {
        TokenTree::Ident(word) => word.to_string() == "impl",
        TokenTree::Group(group) => group.stream().into_iter().any(|it| mentions_impl(&it)),
        _ => false,
    }
}

fn is_punct(token: Option<&TokenTree>, ch: char) -> bool {
    matches!(token, Some(TokenTree::Punct(punct)) if punct.as_char() == ch)
}

fn is_word(token: &TokenTree, word: &str) -> bool {
    matches!(token, TokenTree::Ident(it) if it.to_string() == word)
}

fn is_group(token: &TokenTree, delimiter: Delimiter) -> bool {
    matches!(token, TokenTree::Group(group) if group.delimiter() == delimiter)
}

fn group(delimiter: Delimiter, stream: TokenStream) -> TokenTree {
    TokenTree::Group(Group::new(delimiter, stream))
}

fn code(text: &str) -> TokenStream {
    text.parse().expect("the macro's own code parses")
}

/// `compile_error!` at `span`, and the function as it was, so that its uses still resolve.
fn error(span: Span, message: &str, item: TokenStream) -> TokenStream {
    let mut out: TokenStream = [
        TokenTree::Punct(Punct::new(':', Spacing::Joint)),
        TokenTree::Punct(Punct::new(':', Spacing::Alone)),
        TokenTree::Ident(Ident::new("core", span)),
        TokenTree::Punct(Punct::new(':', Spacing::Joint)),
        TokenTree::Punct(Punct::new(':', Spacing::Alone)),
        TokenTree::Ident(Ident::new("compile_error", span)),
        TokenTree::Punct(Punct::new('!', Spacing::Alone)),
        group(
            Delimiter::Parenthesis,
            [TokenTree::Literal(proc_macro::Literal::string(message))].into_iter().collect(),
        ),
        TokenTree::Punct(Punct::new(';', Spacing::Alone)),
    ]
    .into_iter()
    .map(|mut token| {
        token.set_span(span);
        token
    })
    .collect();
    out.extend(item);
    out
}
