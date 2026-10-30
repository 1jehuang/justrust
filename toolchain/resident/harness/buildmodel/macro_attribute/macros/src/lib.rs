use proc_macro::{Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};

fn name_of(input: &TokenStream) -> String {
    let mut tokens = input.clone().into_iter();
    while let Some(token) = tokens.next() {
        if matches!(&token, TokenTree::Ident(it) if ["struct", "enum", "fn"].contains(&it.to_string().as_str())) {
            return tokens.next().unwrap().to_string();
        }
    }
    panic!("nothing named");
}

/// `impl Named for T { fn name() -> &'static str }`.
#[proc_macro_derive(Named)]
pub fn named(input: TokenStream) -> TokenStream {
    let name = name_of(&input);
    let label = helper::label(&name);
    format!("impl Named for {name} {{ fn name() -> &'static str {{ {label:?} }} }}").parse().unwrap()
}

/// Has the function print its name before it runs.
#[proc_macro_attribute]
pub fn traced(_args: TokenStream, item: TokenStream) -> TokenStream {
    let name = name_of(&item);
    let mut tokens: Vec<TokenTree> = item.into_iter().collect();
    let Some(TokenTree::Group(body)) = tokens.pop() else { panic!("no body") };
    let mut inner: TokenStream = format!("println!(\"enter {{}}\", {:?});", helper::label(&name)).parse().unwrap();
    inner.extend(body.stream());
    tokens.push(TokenTree::Group(Group::new(Delimiter::Brace, inner)));
    tokens.into_iter().collect()
}

/// The sum of the integer literals given, as a literal.
#[proc_macro]
pub fn sum(input: TokenStream) -> TokenStream {
    let total: u64 = input.into_iter().filter_map(|it| it.to_string().parse::<u64>().ok()).sum();
    let _ = (Ident::new("unused", Span::call_site()), Punct::new('+', Spacing::Alone));
    TokenTree::Literal(Literal::u64_suffixed(helper::scaled(total))).into()
}
