//! In-scope array methods precede array-to-slice fallback.
trait Other {
    fn len(&self) -> usize;
}
impl Other for [i32; 2] {
    fn len(&self) -> usize {
        99
    }
}
fn bounded<T: Other>(value: &T) -> usize {
    value.len()
}
mod hidden {
    pub trait Other {
        fn len(&self) -> usize;
    }
    impl Other for [i32; 3] {
        fn len(&self) -> usize {
            77
        }
    }
}
mod imported {
    use super::hidden::Other;
    pub fn run() {
        let value = [1, 2, 3];
        let n = value.len();
        println!("{}", n);
    }
}
mod mutable {
    trait Other {
        fn len(&mut self) -> usize;
    }
    impl Other for [i32; 4] {
        fn len(&mut self) -> usize {
            (*self)[0] = 9;
            88
        }
    }
    pub fn run() {
        let mut value = [1, 2, 3, 4];
        let n = value.len();
        println!("{}", n);
        println!("{}", value[0]);
    }
}
mod generic {
    trait Other {
        fn len<T>(&self) -> usize;
    }
    impl Other for [i32; 5] {
        fn len<T>(&self) -> usize {
            66
        }
    }
    pub fn run() {
        let value = [1, 2, 3, 4, 5];
        let n = value.len::<i32>();
        println!("{}", n);
    }
}
fn main() {
    let mut value = [1, 2];
    let n = value.len();
    println!("{}", n);
    let n = (&value).len();
    println!("{}", n);
    let n = (&mut value).len();
    println!("{}", n);
    value[0] = 6;
    println!("{}", value[0]);
    let n = [3, 4].len();
    println!("{}", n);
    let n = bounded(&value);
    println!("{}", n);
    let value = [1, 2, 3];
    let n = value.len();
    println!("{}", n);
    imported::run();
    mutable::run();
    generic::run();
}
