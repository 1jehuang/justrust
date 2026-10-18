struct C { base: i32 }
impl C { fn add(&self, mut extra: i32) -> i32 { extra = extra + 1; self.base + extra } }
fn main() { let c = C { base: 10 }; if c.add(4) == 15 { println!("method ok"); } }
