fn c0(x:i64)->bool{let n:u32=x.leading_zeros();n==64u32}
fn c1(x:i64)->bool{let n:u32=x.leading_zeros();n==63u32}
fn c2(x:i64)->bool{let n:u32=x.leading_zeros();n==62u32}
fn c3(x:i64)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c4(x:i64)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c5(x:i64)->bool{let n:u32=x.leading_zeros();n==0u32}
fn c6(x:i64)->bool{let n:u32=x.leading_zeros();n==0u32}
fn main(){if c0(0i64) && c1(1i64) && c2(2i64) && c3(9223372036854775807i64) && c4(4611686018427387904i64) && c5(-1i64) && c6(-9223372036854775808i64){println!("ok");}}
