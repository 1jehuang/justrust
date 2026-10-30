//! Macros that give tokens only a proc macro can make.

use proc_macro::{Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};

/// Three negative numbers, each one literal token: `(-1, -20i16, -1.5,)`.
#[proc_macro]
pub fn negative_numbers(_: TokenStream) -> TokenStream {
    let parsed: Literal = "-20i16".parse().expect("a negative number is a literal");
    let numbers = [Literal::i8_unsuffixed(-1), parsed, Literal::f32_unsuffixed(-1.5)];
    let mut tuple = TokenStream::new();
    for number in numbers {
        tuple.extend([TokenTree::from(number), TokenTree::from(Punct::new(',', Spacing::Alone))]);
    }
    TokenTree::from(Group::new(Delimiter::Parenthesis, tuple)).into()
}

/// `[stringify!(<input>), spelled_type!(<input>)]`, the input's tokens made again one by one,
/// as salsa's `#[salsa::tracked]` hands an impl's type, parsed by syn, to `stringify!` to name
/// its queries. Handed on as it came, the input would keep the spacing of its source.
#[proc_macro]
pub fn spelled(input: TokenStream) -> TokenStream {
    let input = remade(input);
    let call = |path: &[&str], input: TokenStream| {
        let mut call = TokenStream::new();
        for (at, segment) in path.iter().enumerate() {
            if at > 0 {
                call.extend([
                    TokenTree::from(Punct::new(':', Spacing::Joint)),
                    TokenTree::from(Punct::new(':', Spacing::Alone)),
                ]);
            }
            call.extend([TokenTree::from(Ident::new(segment, Span::call_site()))]);
        }
        call.extend([
            TokenTree::from(Punct::new('!', Spacing::Alone)),
            TokenTree::from(Group::new(Delimiter::Parenthesis, input)),
        ]);
        call
    };
    let mut array = call(&["stringify"], input.clone());
    array.extend([TokenTree::from(Punct::new(',', Spacing::Alone))]);
    array.extend(call(&["spelled_type"], input));
    TokenTree::from(Group::new(Delimiter::Bracket, array)).into()
}

fn remade(tokens: TokenStream) -> TokenStream {
    tokens
        .into_iter()
        .map(|token| match token {
            TokenTree::Group(group) => {
                let mut made = Group::new(group.delimiter(), remade(group.stream()));
                made.set_span(group.span());
                TokenTree::from(made)
            }
            TokenTree::Punct(punct) => {
                let mut made = Punct::new(punct.as_char(), punct.spacing());
                made.set_span(punct.span());
                TokenTree::from(made)
            }
            leaf => leaf,
        })
        .collect()
}
