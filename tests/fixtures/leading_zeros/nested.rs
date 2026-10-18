fn nested(x:u128)->u32{x.leading_zeros().leading_zeros()}
fn main(){if nested(1u128)==25u32 && 0u8.leading_zeros().leading_zeros()==28u32{println!("ok");}}
