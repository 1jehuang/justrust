use std::fmt::Display;
struct Bag<T: Ord> { data: Vec<T> }
impl<T: Ord + Display> Bag<T> {
fn render(&self)->String {let parts:Vec<String>=self.data.iter().map(|x|format!("{}",x)).collect();parts.join(" < ")}
}
fn main(){}
