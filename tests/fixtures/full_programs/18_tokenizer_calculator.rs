#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
}

fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' => {
                i += 1;
            }
            '+' => { tokens.push(Token::Plus); i += 1; }
            '-' => { tokens.push(Token::Minus); i += 1; }
            '*' => { tokens.push(Token::Star); i += 1; }
            '/' => { tokens.push(Token::Slash); i += 1; }
            '(' => { tokens.push(Token::LParen); i += 1; }
            ')' => { tokens.push(Token::RParen); i += 1; }
            '0'..='9' | '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                let n: f64 = s.parse().map_err(|_| format!("bad number {}", s))?;
                tokens.push(Token::Num(n));
            }
            other => return Err(format!("unexpected character '{}' at {}", other, i)),
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }
    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        while let Some(t) = self.peek() {
            match t {
                Token::Plus => { self.next(); v += self.term()?; }
                Token::Minus => { self.next(); v -= self.term()?; }
                _ => break,
            }
        }
        Ok(v)
    }
    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.factor()?;
        loop {
            match self.peek() {
                Some(Token::Star) => { self.next(); v *= self.factor()?; }
                Some(Token::Slash) => {
                    self.next();
                    let d = self.factor()?;
                    if d == 0.0 {
                        return Err("division by zero".to_string());
                    }
                    v /= d;
                }
                _ => return Ok(v),
            }
        }
    }
    fn factor(&mut self) -> Result<f64, String> {
        match self.next() {
            Some(Token::Num(n)) => Ok(n),
            Some(Token::Minus) => Ok(-self.factor()?),
            Some(Token::LParen) => {
                let v = self.expr()?;
                match self.next() {
                    Some(Token::RParen) => Ok(v),
                    _ => Err("expected )".to_string()),
                }
            }
            Some(t) => Err(format!("unexpected token {:?}", t)),
            None => Err("unexpected end".to_string()),
        }
    }
}

fn eval(src: &str) -> Result<f64, String> {
    let tokens = tokenize(src)?;
    let mut p = Parser { tokens, pos: 0 };
    let v = p.expr()?;
    if p.pos != p.tokens.len() {
        return Err(format!("trailing input at token {}", p.pos));
    }
    Ok(v)
}

fn main() {
    let cases = ["1 + 2 * 3", "(1 + 2) * 3", "10 / 4", "-(2.5 * 4) + 1", "2 * (3 + 4) * 5 - 6 / 3", "1 / 0", "2 +", "3 $ 4", "(1 + 2"];
    for c in cases {
        match eval(c) {
            Ok(v) => println!("{} = {}", c, v),
            Err(e) => println!("{} -> error: {}", c, e),
        }
    }
}
