fn main(){let mut a=[1i32,2,3,4];let s:&mut[i32]=&mut a;println!("before");let max=0usize.wrapping_sub(1);let r=&mut s[..=max];println!("{}",r.len());}
