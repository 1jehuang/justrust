fn c0(x:i128)->bool{let n:u32=x.leading_zeros();n==128u32}
fn c1(x:i128)->bool{let n:u32=x.leading_zeros();n==127u32}
fn c2(x:i128)->bool{let n:u32=x.leading_zeros();n==126u32}
fn c3(x:i128)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c4(x:i128)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c5(x:i128)->bool{let n:u32=x.leading_zeros();n==0u32}
fn c6(x:i128)->bool{let n:u32=x.leading_zeros();n==0u32}
fn c7(x:i128)->bool{let n:u32=x.leading_zeros();n==64u32}
fn c8(x:i128)->bool{let n:u32=x.leading_zeros();n==63u32}
fn c9(x:i128)->bool{let n:u32=x.leading_zeros();n==62u32}
fn c10(x:i128)->bool{let n:u32=x.leading_zeros();n==31u32}
fn main(){if c0(0i128) && c1(1i128) && c2(2i128) && c3(170141183460469231731687303715884105727i128) && c4(85070591730234615865843651857942052864i128) && c5(-1i128) && c6(-170141183460469231731687303715884105728i128) && c7(9223372036854775808i128) && c8(18446744073709551616i128) && c9(36893488147419103232i128) && c10(79228162514264337593543950337i128){println!("ok");}}
