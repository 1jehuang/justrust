struct S{v:i32}impl Drop for S{fn drop(&mut self){self.v=0;}}fn run()->i32{let s=S{v:23};let result=s.v;result}
fn main(){if run()==23{println!("canonical copy oracle ok");}}
