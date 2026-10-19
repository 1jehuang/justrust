fn main(){let mut a=[1i32,2,3,4];let s:&mut[i32]=&mut a;println!("before");let r=&mut s[3..1];println!("{}",r.len());}
