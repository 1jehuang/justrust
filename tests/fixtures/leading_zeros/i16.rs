fn c0(x:i16)->bool{let n:u32=x.leading_zeros();n==16u32}
fn c1(x:i16)->bool{let n:u32=x.leading_zeros();n==15u32}
fn c2(x:i16)->bool{let n:u32=x.leading_zeros();n==14u32}
fn c3(x:i16)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c4(x:i16)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c5(x:i16)->bool{let n:u32=x.leading_zeros();n==0u32}
fn c6(x:i16)->bool{let n:u32=x.leading_zeros();n==0u32}
fn main(){if c0(0i16) && c1(1i16) && c2(2i16) && c3(32767i16) && c4(16384i16) && c5(-1i16) && c6(-32768i16){println!("ok");}}
