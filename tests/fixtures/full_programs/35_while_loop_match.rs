#[derive(Debug)]
enum Token { Num(i64), Plus, Minus, Star, LParen, RParen }
fn lex(s: &str) -> Vec<Token> {
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0; let mut out = Vec::new();
    while i < cs.len() {
        let c = cs[i];
        match c {
            ' ' => { i += 1; }
            '+' => { out.push(Token::Plus); i += 1; }
            '-' => { out.push(Token::Minus); i += 1; }
            '*' => { out.push(Token::Star); i += 1; }
            '(' => { out.push(Token::LParen); i += 1; }
            ')' => { out.push(Token::RParen); i += 1; }
            d if d.is_ascii_digit() => {
                let mut n = 0i64;
                while i < cs.len() && cs[i].is_ascii_digit() { n = n * 10 + cs[i].to_digit(10).unwrap() as i64; i += 1; }
                out.push(Token::Num(n));
            }
            _ => panic!("bad"),
        }
    }
    out
}
fn main() { println!("{:?}", lex("12 + (3 * 4) - 5")); }
