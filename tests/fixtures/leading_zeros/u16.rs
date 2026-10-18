fn c0(x:u16)->bool{let n:u32=x.leading_zeros();n==16u32}
fn c1(x:u16)->bool{let n:u32=x.leading_zeros();n==15u32}
fn c2(x:u16)->bool{let n:u32=x.leading_zeros();n==14u32}
fn c3(x:u16)->bool{let n:u32=x.leading_zeros();n==0u32}
fn c4(x:u16)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c5(x:u16)->bool{let n:u32=x.leading_zeros();n==0u32}
fn main(){if c0(0u16) && c1(1u16) && c2(2u16) && c3(65535u16) && c4(16384u16) && c5(32768u16){println!("ok");}}
