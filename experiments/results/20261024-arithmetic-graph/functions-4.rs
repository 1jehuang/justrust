fn f0(x:i32)->i32 { if x < 0 { -x } else { x + 0 } }
fn f1(x:i32)->i32 { if x < 0 { -x } else { x + 1 } }
fn f2(x:i32)->i32 { if x < 0 { -x } else { x + 2 } }
fn f3(x:i32)->i32 { if x < 0 { -x } else { x + 3 } }
fn fail(x:i32) { let y=1/x; }
fn main(){if f0(2) != 2 { fail(0); }
if f1(2) != 3 { fail(0); }
if f2(2) != 4 { fail(0); }
if f3(2) != 5 { fail(0); }println!("compiled correctly");}