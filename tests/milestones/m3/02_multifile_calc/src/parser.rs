use crate::lexer::Tok;
pub struct Parser { toks: Vec<Tok>, pos: usize }
impl Parser {
    pub fn new(toks: Vec<Tok>) -> Self { Parser { toks, pos: 0 } }
    pub fn parse(mut self) -> Result<f64, String> { let v = self.expr(0)?; if self.pos != self.toks.len() { return Err("trailing input".into()); } Ok(v) }
    fn prec(op: char) -> (u8, bool) { match op { '+' | '-' => (1, false), '*' | '/' => (2, false), _ => (3, true) } }
    fn atom(&mut self) -> Result<f64, String> {
        match self.toks.get(self.pos).cloned() {
            Some(Tok::Num(n)) => { self.pos += 1; Ok(n) }
            Some(Tok::Op('-')) => { self.pos += 1; Ok(-self.atom()?) }
            Some(Tok::LParen) => { self.pos += 1; let v = self.expr(0)?; if self.toks.get(self.pos) != Some(&Tok::RParen) { return Err("expected )".into()); } self.pos += 1; Ok(v) }
            other => Err(format!("unexpected {other:?}")),
        }
    }
    fn expr(&mut self, min: u8) -> Result<f64, String> {
        let mut lhs = self.atom()?;
        while let Some(Tok::Op(op)) = self.toks.get(self.pos).cloned() {
            let (p, right) = Self::prec(op); if p < min { break; }
            self.pos += 1; let rhs = self.expr(if right { p } else { p + 1 })?;
            lhs = match op { '+' => lhs + rhs, '-' => lhs - rhs, '*' => lhs * rhs, '/' if rhs == 0.0 => return Err("division by zero".into()), '/' => lhs / rhs, _ => lhs.powf(rhs) };
        }
        Ok(lhs)
    }
}
