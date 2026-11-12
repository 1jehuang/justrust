enum L<'a> { Nil, C(&'a str, Box<L<'a>>) }
fn main() {
    let l = L::C("x", Box::new(L::Nil));
    let n: Box<Option<i32>> = Box::new(Some(3));
    if let L::C(w, _) = &l { println!("{} {:?}", w, n); }
}
