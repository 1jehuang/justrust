struct S{n:i32}
impl S{fn leading_zeros(self)->i32{self.n+2}}
fn main(){let s=S{n:5};if s.leading_zeros()==7{println!("ok");}}
