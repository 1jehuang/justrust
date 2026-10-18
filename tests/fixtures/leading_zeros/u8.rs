fn c0(x:u8)->bool{let n:u32=x.leading_zeros();n==8u32}
fn c1(x:u8)->bool{let n:u32=x.leading_zeros();n==7u32}
fn c2(x:u8)->bool{let n:u32=x.leading_zeros();n==6u32}
fn c3(x:u8)->bool{let n:u32=x.leading_zeros();n==0u32}
fn c4(x:u8)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c5(x:u8)->bool{let n:u32=x.leading_zeros();n==0u32}
fn main(){if c0(0u8) && c1(1u8) && c2(2u8) && c3(255u8) && c4(64u8) && c5(128u8){println!("ok");}}
