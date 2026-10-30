//! Each lint `cargo blaze clippy` has, firing, and its level set by attributes.

pub enum Never {}

#[allow(clippy::empty_enums)]
pub enum Allowed {}

#[expect(clippy::empty_enums)]
pub enum Expected {}

#[expect(clippy::empty_enums, reason = "a struct is no enum")]
pub struct NotAnEnum;

#[warn(clippy::nursery)]
pub enum Nursery {}

pub const GREETING: &'static str = "hi";
pub static NAMES: &'static [&'static str] = &["a"];

#[macro_export]
macro_rules! my_macro {
    ($($t:tt)*) => {
        0
    };
}

pub fn braces() -> Vec<i32> {
    let v = vec! {1, 2};
    let _ = my_macro!(1);
    v
}

pub fn maps(x: Option<i32>, r: Result<i32, ()>) -> i32 {
    let a = x.map(|it| it + 1).unwrap_or(0);
    let b = r.map(|it| it * 2).unwrap_or_default();
    let c = x.map(|it| it > 1).unwrap_or(false);
    let d = x.map(Some).unwrap_or(None);
    println!("{}", x.map(|it| it + 2).unwrap_or(1));
    if c { a + b } else { d.unwrap_or(0) }
}

pub fn formats(s: &str, n: i32, m: &mut i32) -> String {
    println!("{} {:?}", &s, &&n);
    println!("{0:1$}", &n, &5usize);
    println!("{}", &mut *m);
    println!("{:p}", &n);
    println!("{n} {n}", n = &(n));
    format!("{}", &(n + 1))
}

pub fn conditions(x: Option<i32>, r: Result<i32, ()>, a: bool, b: bool) -> Result<bool, std::io::Error> {
    let _error = std::io::Error::new(std::io::ErrorKind::Other, "x");
    let _ = x.map_or(false, |it| it > 1);
    let _ = x.map_or(true, |it| it > 1);
    let _ = r.map_or(false, |it| it > 1);
    if a {
        if b || a {
            println!("both");
        }
    }
    let _ = if a { true } else { false };
    if b {
        return Ok(true);
    }
    Ok(false)
}

fn sum(values: &[u32]) -> u32 {
    values.iter().sum()
}

pub fn vectors(n: usize) -> u32 {
    let mut taken = vec![false; 16];
    taken[n] = true;
    let listed = vec![1, 2, 3];
    let mut total = sum(&vec![4, 5]) + listed.len() as u32;
    for it in vec![6, 7] {
        total += it;
    }
    let kept = vec![8];
    total += kept.iter().sum::<u32>();
    let grown = vec![9];
    let mut grown = grown;
    grown.push(10);
    total + sum(&grown) + u32::from(taken[0]) + sum(&mut vec![11, /* twelve */ 12])
}

#[derive(Clone, Copy)]
pub struct Point(pub u8);

pub struct Builder {
    name: String,
}

impl Builder {
    pub fn new() -> Builder {
        Builder { name: String::new() }
    }

    pub fn build(&mut self, point: &Point, items: &[u8]) -> Option<u8> {
        self.name.push_str("a");
        self.name.insert_str(0, "'");
        let _ = format!("{}", self.name);
        let _ = format!("plain");
        let copied = point.clone();
        let _ = items.get(0);
        let _ = &[b'a', b'b'];
        return Some(copied.0);
    }
}

enum Encoding {
    UTF8,
    Latin,
}

pub fn encoding(utf8: bool) -> u8 {
    let pick = |it: Encoding| match it {
        Encoding::UTF8 => return 1,
        Encoding::Latin => 2,
    };
    pick(if utf8 { Encoding::UTF8 } else { Encoding::Latin })
}

#[macro_export]
macro_rules! here {
    () => {
        crate::encoding(true)
    };
}

pub fn is_word(b: u8, kind: Option<Option<u8>>) -> bool {
    let latin = if let Some(None) = kind { true } else { false };
    latin
        || match b {
            b'a'..=b'z' | b'_' => true,
            _ => false,
        }
}

pub fn copies(names: &[u32], words: &[String], first: Option<&String>) -> (Vec<u32>, Vec<u32>, Vec<String>, Option<String>) {
    let a = names.iter().map(|&n| n).collect();
    let b = names.iter().map(|n| *n).collect();
    let c = words.iter().map(|w| w.clone()).collect();
    (a, b, c, first.map(|w| w.clone()))
}

pub fn refs(words: &[&str]) -> usize {
    match words.first() {
        Some(ref word) => word.len(),
        None => 0,
    }
}

#[deny(clippy::bool_assert_comparison)]
pub fn denied() {
    assert_eq!(braces().is_empty(), false);
}

#[cfg(test)]
mod tests {
    #[test]
    fn asserts() {
        assert_eq!(true, 1 > 0);
        assert_ne!(super::maps(None, Ok(1)) == 2, false);
        debug_assert_eq!(1 == 1, true);
    }
}
