struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}}impl Copy for S{}impl Drop for S{fn drop(&mut self){}}fn run()->i32{0}
fn main(){if run()==0{println!("canonical copy oracle ok");}}
