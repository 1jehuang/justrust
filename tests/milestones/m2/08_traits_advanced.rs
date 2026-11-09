use std::collections::HashMap;
use std::fmt::Debug;
trait Animal { fn name(&self) -> &str; fn sound(&self) -> String; fn speak(&self) -> String { format!("{} says {}", self.name(), self.sound()) } }
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)] struct Dog { name: String }
impl Animal for Dog { fn name(&self) -> &str { &self.name } fn sound(&self) -> String { "woof".into() } }
trait Container { type Item; fn get(&self, i: usize) -> Option<&Self::Item>; fn first(&self) -> Option<&Self::Item> { self.get(0) } fn len(&self) -> usize; }
struct Stack<T> { items: Vec<T> }
impl<T> Container for Stack<T> { type Item = T; fn get(&self, i: usize) -> Option<&T> { self.items.get(i) } fn len(&self) -> usize { self.items.len() } }
fn show_all<C: Container>(c: &C) -> Vec<String> where C::Item: Debug { (0..c.len()).filter_map(|i| c.get(i)).map(|x| format!("{x:?}")).collect() }
struct Countdown(u32);
impl Iterator for Countdown { type Item = u32; fn next(&mut self) -> Option<u32> { if self.0 == 0 { None } else { self.0 -= 1; Some(self.0 + 1) } } }
trait Zero { const ZERO: Self; } impl Zero for i32 { const ZERO: i32 = 0; }
fn count_nonzero<T: Zero + PartialEq>(xs: &[T]) -> usize { xs.iter().filter(|x| **x != T::ZERO).count() }
impl From<&str> for Dog { fn from(s: &str) -> Dog { Dog { name: s.to_string() } } }
fn main() {
    let d: Dog = "rex".into();
    println!("{}", d.speak());
    let s = Stack { items: vec![d.clone(), Dog::from("ace")] };
    println!("{:?} {:?}", show_all(&s), s.first().map(|d| d.name()));
    let mut dogs = s.items.clone(); dogs.sort(); println!("{:?}", dogs.iter().map(|d| &d.name).collect::<Vec<_>>());
    let mut hm = HashMap::new(); hm.insert(d.clone(), 3); println!("{:?}", hm.get(&Dog::from("rex")));
    println!("{:?} {}", Countdown(4).collect::<Vec<_>>(), Countdown(10).filter(|x| x % 3 == 0).map(|x| x * x).sum::<u32>());
    println!("{}", count_nonzero(&[0, 3, 0, 5]));
    let animals: Vec<&dyn Animal> = vec![&d, &dogs[0]];
    for a in animals { println!("{}", a.speak()); }
    println!("{}", d == dogs[1] && d > dogs[0]);
}
