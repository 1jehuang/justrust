fn apply<F: Fn(i32) -> i32>(f: F, x: i32) -> i32 { f(x) }
fn compose<A, B, C>(f: impl Fn(A) -> B, g: impl Fn(B) -> C) -> impl Fn(A) -> C { move |x| g(f(x)) }
fn make_counter() -> impl FnMut() -> u32 { let mut c = 0; move || { c += 1; c } }
fn make_adders() -> Vec<Box<dyn Fn(i32) -> i32>> { (1..=3).map(|n| Box::new(move |x| x + n) as Box<dyn Fn(i32) -> i32>).collect() }
fn call_once<F: FnOnce() -> String>(f: F) -> String { f() }
struct Button { handlers: Vec<Box<dyn FnMut(&str) -> String>> }
fn main() {
    let k = 10;
    println!("{}", apply(|x| x * k, 4));
    let h = compose(|x: i32| x + 1, |y: i32| format!("<{y}>"));
    println!("{}", h(41));
    let mut c = make_counter(); c(); c();
    println!("{}", c());
    println!("{:?}", make_adders().iter().map(|f| f(10)).collect::<Vec<_>>());
    let s = String::from("owned");
    println!("{}", call_once(move || s + "!"));
    let mut log = Vec::new();
    { let mut push = |m: &str| log.push(m.to_uppercase()); push("a"); push("b"); }
    println!("{:?}", log);
    let mut count = 0;
    let mut b = Button { handlers: vec![] };
    b.handlers.push(Box::new(|e| format!("first:{e}")));
    b.handlers.push(Box::new(move |e| { count += 1; format!("second:{e}#{count}") }));
    for _ in 0..2 { for hd in b.handlers.iter_mut() { println!("{}", hd("click")); } }
    let mut v = vec![(3, "c"), (1, "a"), (2, "b")];
    v.sort_by_key(|&(n, _)| std::cmp::Reverse(n));
    println!("{:?}", v);
    let fns: [fn(f64) -> f64; 2] = [f64::sqrt, f64::abs];
    println!("{:?}", fns.iter().map(|f| f(16.0)).collect::<Vec<_>>());
}
