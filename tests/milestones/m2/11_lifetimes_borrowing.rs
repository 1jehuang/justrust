struct Parser<'a> { src: &'a str, pos: usize }
impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self { Parser { src, pos: 0 } }
    fn skip_ws(&mut self) { while self.src[self.pos..].starts_with(' ') { self.pos += 1; } }
    fn word(&mut self) -> Option<&'a str> { self.skip_ws(); let rest = &self.src[self.pos..]; if rest.is_empty() { return None; } let end = rest.find(' ').unwrap_or(rest.len()); self.pos += end; Some(&rest[..end]) }
}
fn longest<'a>(a: &'a str, b: &'a str) -> &'a str { if a.len() >= b.len() { a } else { b } }
fn split_first_mut(v: &mut [i32]) -> Option<(&mut i32, &mut [i32])> { v.split_first_mut() }
struct Highest<'a, T> { best: Option<&'a T> }
impl<'a, T: PartialOrd> Highest<'a, T> { fn feed(&mut self, x: &'a T) { if self.best.map_or(true, |b| x > b) { self.best = Some(x); } } }
fn main() {
    let text = String::from("  the   borrow checker  rules ");
    let mut p = Parser::new(&text);
    let mut words = vec![];
    while let Some(w) = p.word() { words.push(w); }
    println!("{:?}", words);
    let l = words.iter().fold("", |acc, w| longest(acc, w)); println!("{l}");
    let mut nums = vec![5, 1, 4];
    if let Some((h, t)) = split_first_mut(&mut nums) { *h += t.iter().sum::<i32>(); t[0] = 9; }
    println!("{:?}", nums);
    let mut hi = Highest { best: None }; for n in &nums { hi.feed(n); } println!("{:?}", hi.best);
    let mut grid = vec![vec![0; 3]; 2];
    for (r, row) in grid.iter_mut().enumerate() { for (c, cell) in row.iter_mut().enumerate() { *cell = r * 3 + c; } }
    let (top, bottom) = grid.split_at_mut(1); top[0][0] = bottom[0][2] * 10;
    println!("{:?}", grid);
    let mut s = String::from("abc"); let r = &mut s; r.push('d'); let len = r.len(); s.push_str(&len.to_string()); println!("{s}");
    let mut v = vec![1, 2, 3]; let last = *v.last().unwrap(); v.push(last + 1); println!("{v:?}");
    let mut opt = Some(String::from("x")); if let Some(s) = opt.as_mut() { s.push('y'); } let taken = opt.take(); println!("{taken:?} {opt:?}");
    let mut arr = [3, 1, 2]; arr.swap(0, 2); std::mem::swap(&mut arr[0], &mut 7.clone()); let old = std::mem::replace(&mut arr[1], 0); println!("{:?} {}", arr, old);
}
