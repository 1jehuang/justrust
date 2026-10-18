fn c0(x:i32)->bool{let n:u32=x.leading_zeros();n==32u32}
fn c1(x:i32)->bool{let n:u32=x.leading_zeros();n==31u32}
fn c2(x:i32)->bool{let n:u32=x.leading_zeros();n==30u32}
fn c3(x:i32)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c4(x:i32)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c5(x:i32)->bool{let n:u32=x.leading_zeros();n==0u32}
fn c6(x:i32)->bool{let n:u32=x.leading_zeros();n==0u32}
fn main(){if c0(0i32) && c1(1i32) && c2(2i32) && c3(2147483647i32) && c4(1073741824i32) && c5(-1i32) && c6(-2147483648i32){println!("ok");}}
