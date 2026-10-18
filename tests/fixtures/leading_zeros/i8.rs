fn c0(x:i8)->bool{let n:u32=x.leading_zeros();n==8u32}
fn c1(x:i8)->bool{let n:u32=x.leading_zeros();n==7u32}
fn c2(x:i8)->bool{let n:u32=x.leading_zeros();n==6u32}
fn c3(x:i8)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c4(x:i8)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c5(x:i8)->bool{let n:u32=x.leading_zeros();n==0u32}
fn c6(x:i8)->bool{let n:u32=x.leading_zeros();n==0u32}
fn main(){if c0(0i8) && c1(1i8) && c2(2i8) && c3(127i8) && c4(64i8) && c5(-1i8) && c6(-128i8){println!("ok");}}
