fn c0(x:u32)->bool{let n:u32=x.leading_zeros();n==32u32}
fn c1(x:u32)->bool{let n:u32=x.leading_zeros();n==31u32}
fn c2(x:u32)->bool{let n:u32=x.leading_zeros();n==30u32}
fn c3(x:u32)->bool{let n:u32=x.leading_zeros();n==0u32}
fn c4(x:u32)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c5(x:u32)->bool{let n:u32=x.leading_zeros();n==0u32}
fn main(){if c0(0u32) && c1(1u32) && c2(2u32) && c3(4294967295u32) && c4(1073741824u32) && c5(2147483648u32){println!("ok");}}
