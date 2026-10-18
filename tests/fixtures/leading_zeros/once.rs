fn next(p:&mut i32)->u64{*p=*p+1;1u64}
fn main(){let mut n=0;let z=next(&mut n).leading_zeros();if n==1 && z==63u32{println!("ok");}}
