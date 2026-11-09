use std::collections::BTreeMap;
use std::fmt;
#[derive(Debug, Clone, PartialEq)]
enum Json { Null, Bool(bool), Num(f64), Str(String), Arr(Vec<Json>), Obj(BTreeMap<String, Json>) }
struct P<'a> { s: &'a [u8], i: usize }
impl<'a> P<'a> {
    fn ws(&mut self) { while self.i < self.s.len() && (self.s[self.i] as char).is_ascii_whitespace() { self.i += 1; } }
    fn eat(&mut self, c: u8) -> Result<(), String> { self.ws(); if self.s.get(self.i) == Some(&c) { self.i += 1; Ok(()) } else { Err(format!("expected '{}' at {}", c as char, self.i)) } }
    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.s.get(self.i).copied() {
            Some(b'n') => self.lit("null", Json::Null), Some(b't') => self.lit("true", Json::Bool(true)), Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b'[') => { self.i += 1; let mut v = vec![]; self.ws(); if self.s.get(self.i) == Some(&b']') { self.i += 1; return Ok(Json::Arr(v)); } loop { v.push(self.value()?); self.ws(); match self.s.get(self.i) { Some(b',') => self.i += 1, Some(b']') => { self.i += 1; return Ok(Json::Arr(v)); } _ => return Err(format!("bad array at {}", self.i)) } } }
            Some(b'{') => { self.i += 1; let mut m = BTreeMap::new(); self.ws(); if self.s.get(self.i) == Some(&b'}') { self.i += 1; return Ok(Json::Obj(m)); } loop { self.ws(); let k = self.string()?; self.eat(b':')?; let v = self.value()?; m.insert(k, v); self.ws(); match self.s.get(self.i) { Some(b',') => self.i += 1, Some(b'}') => { self.i += 1; return Ok(Json::Obj(m)); } _ => return Err(format!("bad object at {}", self.i)) } } }
            Some(c) if c == b'-' || c.is_ascii_digit() => { let st = self.i; self.i += 1; while self.i < self.s.len() && matches!(self.s[self.i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') { self.i += 1; } std::str::from_utf8(&self.s[st..self.i]).unwrap().parse().map(Json::Num).map_err(|e| format!("{e}")) }
            _ => Err(format!("unexpected at {}", self.i)),
        }
    }
    fn lit(&mut self, w: &str, v: Json) -> Result<Json, String> { if self.s[self.i..].starts_with(w.as_bytes()) { self.i += w.len(); Ok(v) } else { Err("bad literal".into()) } }
    fn string(&mut self) -> Result<String, String> {
        self.eat(b'"')?; let mut out = String::new();
        loop { match self.s.get(self.i).copied() { None => return Err("unterminated".into()), Some(b'"') => { self.i += 1; return Ok(out); } Some(b'\\') => { self.i += 1; let c = self.s[self.i]; self.i += 1; out.push(match c { b'n' => '\n', b't' => '\t', b'u' => { let h = std::str::from_utf8(&self.s[self.i..self.i + 4]).unwrap(); self.i += 4; char::from_u32(u32::from_str_radix(h, 16).unwrap()).unwrap_or('?') } c => c as char }); } Some(_) => { let rest = std::str::from_utf8(&self.s[self.i..]).unwrap(); let ch = rest.chars().next().unwrap(); out.push(ch); self.i += ch.len_utf8(); } } }
    }
}
impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self { Json::Null => write!(f, "null"), Json::Bool(b) => write!(f, "{b}"), Json::Num(n) => write!(f, "{n}"), Json::Str(s) => write!(f, "{s:?}"),
            Json::Arr(v) => { write!(f, "[")?; for (i, x) in v.iter().enumerate() { if i > 0 { write!(f, ",")?; } write!(f, "{x}")?; } write!(f, "]") }
            Json::Obj(m) => { write!(f, "{{")?; for (i, (k, v)) in m.iter().enumerate() { if i > 0 { write!(f, ",")?; } write!(f, "{k:?}:{v}")?; } write!(f, "}}") } }
    }
}
fn depth(j: &Json) -> usize { match j { Json::Arr(v) => 1 + v.iter().map(depth).max().unwrap_or(0), Json::Obj(m) => 1 + m.values().map(depth).max().unwrap_or(0), _ => 0 } }
fn main() {
    let docs = [r#"{"name": "jr", "tags": ["a", "b\n", "\u00e9"], "v": 1.5e2, "ok": true, "n": null, "nested": {"x": [1, [2, [3]]]}}"#, "[1, 2,", r#"{"a" 1}"#, "  -0.25 "];
    for d in docs { match (P { s: d.as_bytes(), i: 0 }).value() { Ok(j) => println!("{j} depth={}", depth(&j)), Err(e) => println!("error: {e}") } }
}
