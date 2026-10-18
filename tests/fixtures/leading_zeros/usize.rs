fn c0(x:usize)->bool{let n:u32=x.leading_zeros();n==64u32}
fn c1(x:usize)->bool{let n:u32=x.leading_zeros();n==63u32}
fn c2(x:usize)->bool{let n:u32=x.leading_zeros();n==62u32}
fn c3(x:usize)->bool{let n:u32=x.leading_zeros();n==0u32}
fn c4(x:usize)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c5(x:usize)->bool{let n:u32=x.leading_zeros();n==0u32}
fn main(){if c0(0usize) && c1(1usize) && c2(2usize) && c3(18446744073709551615usize) && c4(4611686018427387904usize) && c5(9223372036854775808usize){println!("ok");}}
