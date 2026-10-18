fn c0(x:u128)->bool{let n:u32=x.leading_zeros();n==128u32}
fn c1(x:u128)->bool{let n:u32=x.leading_zeros();n==127u32}
fn c2(x:u128)->bool{let n:u32=x.leading_zeros();n==126u32}
fn c3(x:u128)->bool{let n:u32=x.leading_zeros();n==0u32}
fn c4(x:u128)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c5(x:u128)->bool{let n:u32=x.leading_zeros();n==0u32}
fn c6(x:u128)->bool{let n:u32=x.leading_zeros();n==64u32}
fn c7(x:u128)->bool{let n:u32=x.leading_zeros();n==63u32}
fn c8(x:u128)->bool{let n:u32=x.leading_zeros();n==62u32}
fn c9(x:u128)->bool{let n:u32=x.leading_zeros();n==31u32}
fn main(){if c0(0u128) && c1(1u128) && c2(2u128) && c3(340282366920938463463374607431768211455u128) && c4(85070591730234615865843651857942052864u128) && c5(170141183460469231731687303715884105728u128) && c6(9223372036854775808u128) && c7(18446744073709551616u128) && c8(36893488147419103232u128) && c9(79228162514264337593543950337u128){println!("ok");}}
