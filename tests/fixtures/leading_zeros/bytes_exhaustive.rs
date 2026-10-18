fn verify()->bool{let mut x=0u16;while x<256u16{let mut t=x;let mut expected=8u32;while t!=0u16{t=t>>1u8;expected=expected-1u32;}let unsigned=x as u8;let signed=x as i8;if unsigned.leading_zeros()!=expected || signed.leading_zeros()!=expected{return false;}x=x+1u16;}true}
fn main(){if verify(){println!("ok");}}
