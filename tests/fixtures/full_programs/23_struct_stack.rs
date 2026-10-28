struct Stack{items:Vec<i32>}
impl Stack{fn new()->Self{Stack{items:Vec::new()}}fn push(&mut self,x:i32){self.items.push(x);}fn pop(&mut self)->Option<i32>{self.items.pop()}}
fn main(){let mut s=Stack::new();for i in 1..=5{s.push(i*i);}while let Some(x)=s.pop(){print!("{} ",x);}println!();}
