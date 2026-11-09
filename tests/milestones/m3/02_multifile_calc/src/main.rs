mod lexer;
mod parser;
fn main() {
    for src in ["1 + 2 * 3", "(1 + 2) * 3", "2 ^ 3 ^ 2", "-4 + 10 / (3 - 1)", "1 +", "8 / (2 - 2)"] {
        let r = lexer::lex(src).and_then(|t| parser::Parser::new(t).parse());
        match r { Ok(v) => println!("{src} = {v}"), Err(e) => println!("{src} -> error: {e}") }
    }
}
