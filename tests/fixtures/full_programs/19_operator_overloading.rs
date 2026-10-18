use std::ops::{Add, Mul, Neg, Sub, Index};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Vec2 {
    x: i32,
    y: i32,
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2 { x: self.x + o.x, y: self.y + o.y }
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2 { x: self.x - o.x, y: self.y - o.y }
    }
}

impl Mul<i32> for Vec2 {
    type Output = Vec2;
    fn mul(self, k: i32) -> Vec2 {
        Vec2 { x: self.x * k, y: self.y * k }
    }
}

impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2 { x: -self.x, y: -self.y }
    }
}

impl fmt::Display for Vec2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}, {}>", self.x, self.y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Fraction {
    num: i64,
    den: i64,
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a.abs() } else { gcd(b, a % b) }
}

impl Fraction {
    fn new(num: i64, den: i64) -> Fraction {
        let g = gcd(num, den);
        let s = if den < 0 { -1 } else { 1 };
        Fraction { num: s * num / g, den: s * den / g }
    }
}

impl Add for Fraction {
    type Output = Fraction;
    fn add(self, o: Fraction) -> Fraction {
        Fraction::new(self.num * o.den + o.num * self.den, self.den * o.den)
    }
}

impl fmt::Display for Fraction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 { write!(f, "{}", self.num) } else { write!(f, "{}/{}", self.num, self.den) }
    }
}

struct Polyline {
    points: Vec<Vec2>,
}

impl Index<usize> for Polyline {
    type Output = Vec2;
    fn index(&self, i: usize) -> &Vec2 {
        &self.points[i]
    }
}

fn main() {
    let a = Vec2 { x: 1, y: 2 };
    let b = Vec2 { x: 3, y: -4 };
    println!("a + b = {}", a + b);
    println!("a - b = {}", a - b);
    println!("a * 3 = {}", a * 3);
    println!("-b = {}", -b);
    println!("eq: {}", a + b == Vec2 { x: 4, y: -2 });
    let mut h = Fraction::new(0, 1);
    for k in 1..=6 {
        h = h + Fraction::new(1, k);
    }
    println!("H(6) = {}", h);
    println!("2/4 = {}", Fraction::new(2, 4));
    println!("6/-3 = {}", Fraction::new(6, -3));
    println!("cmp: {:?}", Fraction::new(1, 2).cmp(&Fraction::new(1, 3)));
    let line = Polyline { points: vec![a, b, a + b] };
    println!("line[2] = {}", line[2]);
    let sum = line.points.iter().fold(Vec2 { x: 0, y: 0 }, |acc, &p| acc + p);
    println!("sum = {:?}", sum);
}
