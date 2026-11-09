#[derive(Debug, Clone, PartialEq)]
pub enum Tok { Num(f64), Op(char), LParen, RParen }
pub fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new(); let cs: Vec<char> = s.chars().collect(); let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() { i += 1; continue; }
        if c.is_ascii_digit() { let st = i; while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.') { i += 1; } out.push(Tok::Num(cs[st..i].iter().collect::<String>().parse().map_err(|_| "bad number")?)); continue; }
        out.push(match c { '+' | '-' | '*' | '/' | '^' => Tok::Op(c), '(' => Tok::LParen, ')' => Tok::RParen, _ => return Err(format!("unexpected {c:?}")) });
        i += 1;
    }
    Ok(out)
}
