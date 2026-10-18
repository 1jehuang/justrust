use std::fmt::Display;

#[derive(Debug, Clone, PartialEq)]
struct Pair<T> {
    a: T,
    b: T,
}

impl<T: PartialOrd + Copy> Pair<T> {
    fn largest(&self) -> T {
        if self.a > self.b { self.a } else { self.b }
    }
    fn swap(self) -> Pair<T> {
        Pair { a: self.b, b: self.a }
    }
}

struct SortedVec<T: Ord> {
    data: Vec<T>,
}

impl<T: Ord + Display> SortedVec<T> {
    fn new() -> Self {
        SortedVec { data: Vec::new() }
    }
    fn insert(&mut self, item: T) {
        let pos = match self.data.binary_search(&item) {
            Ok(p) => p,
            Err(p) => p,
        };
        self.data.insert(pos, item);
    }
    fn contains(&self, item: &T) -> bool {
        self.data.binary_search(item).is_ok()
    }
    fn render(&self) -> String {
        let parts: Vec<String> = self.data.iter().map(|x| format!("{}", x)).collect();
        parts.join(" < ")
    }
}

fn largest<T: PartialOrd + Copy>(items: &[T]) -> T {
    let mut m = items[0];
    for &it in items {
        if it > m {
            m = it;
        }
    }
    m
}

trait Summary {
    fn summarize(&self) -> String;
}

impl<T: Display> Summary for Pair<T> {
    fn summarize(&self) -> String {
        format!("({}, {})", self.a, self.b)
    }
}

fn print_summary(item: &impl Summary) {
    println!("summary: {}", item.summarize());
}

fn main() {
    let p = Pair { a: 3, b: 9 };
    println!("largest of {:?} is {}", p, p.largest());
    let q = p.clone().swap();
    println!("swapped: {:?}, equal: {}", q, p == q);
    print_summary(&Pair { a: "x", b: "y" });
    println!("largest int: {}", largest(&[4, 8, 15, 16, 23, 42]));
    println!("largest char: {}", largest(&['q', 'z', 'a']));
    let mut sv = SortedVec::new();
    for x in [5, 1, 4, 2, 3, 9, 0] {
        sv.insert(x);
    }
    println!("{}", sv.render());
    println!("contains 4: {}, contains 7: {}", sv.contains(&4), sv.contains(&7));
    let mut words = SortedVec::new();
    for w in ["pear", "apple", "fig"] {
        words.insert(w.to_string());
    }
    println!("{}", words.render());
}
