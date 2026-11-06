struct Parser<'a> { input: &'a str, pos: usize }
impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self { Parser { input, pos: 0 } }
    fn next_word(&mut self) -> Option<&'a str> {
        let rest = &self.input[self.pos..];
        let rest_trim = rest.trim_start();
        if rest_trim.is_empty() { return None; }
        let start = self.pos + (rest.len() - rest_trim.len());
        let end = rest_trim.find(' ').map(|i| start + i).unwrap_or(self.input.len());
        self.pos = end;
        Some(&self.input[start..end])
    }
}
fn main() {
    let mut p = Parser::new("  alpha beta   gamma ");
    while let Some(w) = p.next_word() { println!("[{}]", w); }
}
