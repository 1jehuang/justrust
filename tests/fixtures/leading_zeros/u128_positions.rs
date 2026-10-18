fn verify()->bool{let mut x=1u128;let mut i=0u32;while i<128u32{if x.leading_zeros()!=127u32-i{return false;}if i<127u32{x=x<<1u8;}i=i+1u32;}0u128.leading_zeros()==128u32}
fn main(){if verify(){println!("ok");}}
