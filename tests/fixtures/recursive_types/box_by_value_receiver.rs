struct N { v: i32 }
impl N { fn take(self) -> i32 { self.v } }
fn main() { let b = Box::new(N { v: 4 }); let n = b.take(); println!("{n}"); }
