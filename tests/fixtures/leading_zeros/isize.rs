fn c0(x:isize)->bool{let n:u32=x.leading_zeros();n==64u32}
fn c1(x:isize)->bool{let n:u32=x.leading_zeros();n==63u32}
fn c2(x:isize)->bool{let n:u32=x.leading_zeros();n==62u32}
fn c3(x:isize)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c4(x:isize)->bool{let n:u32=x.leading_zeros();n==1u32}
fn c5(x:isize)->bool{let n:u32=x.leading_zeros();n==0u32}
fn c6(x:isize)->bool{let n:u32=x.leading_zeros();n==0u32}
fn main(){if c0(0isize) && c1(1isize) && c2(2isize) && c3(9223372036854775807isize) && c4(4611686018427387904isize) && c5(-1isize) && c6(-9223372036854775808isize){println!("ok");}}
