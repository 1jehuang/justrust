#[derive(Debug, Clone, PartialEq)]
enum Expr { Num(f64), Var(String), Bin(Box<Expr>, Op, Box<Expr>), Neg(Box<Expr>), Let { name: String, val: Box<Expr>, body: Box<Expr> } }
#[derive(Debug, Clone, Copy, PartialEq)] enum Op { Add, Sub, Mul, Div }
fn eval(e: &Expr, env: &mut Vec<(String, f64)>) -> Result<f64, String> {
    Ok(match e {
        Expr::Num(n) => *n,
        Expr::Var(v) => env.iter().rev().find(|(n, _)| n == v).map(|(_, x)| *x).ok_or_else(|| format!("unbound {v}"))?,
        Expr::Neg(inner) => -eval(inner, env)?,
        Expr::Bin(l, op, r) => { let (a, b) = (eval(l, env)?, eval(r, env)?); match op { Op::Add => a + b, Op::Sub => a - b, Op::Mul => a * b, Op::Div if b == 0.0 => return Err("div by zero".into()), Op::Div => a / b } }
        Expr::Let { name, val, body } => { let v = eval(val, env)?; env.push((name.clone(), v)); let r = eval(body, env); env.pop(); r? }
    })
}
fn num(n: f64) -> Box<Expr> { Box::new(Expr::Num(n)) }
fn classify(x: i32) -> &'static str { match x { i32::MIN..=-1 => "neg", 0 => "zero", 1 | 2 | 3 => "small", n if n % 2 == 0 => "even", _ => "odd" } }
fn main() {
    let e = Expr::Let { name: "x".into(), val: Box::new(Expr::Bin(num(3.0), Op::Mul, num(4.0))), body: Box::new(Expr::Bin(Box::new(Expr::Var("x".into())), Op::Sub, Box::new(Expr::Neg(num(0.5))))) };
    let mut env = vec![];
    println!("{:?}", eval(&e, &mut env));
    println!("{:?}", eval(&Expr::Bin(num(1.0), Op::Add, num(2.0)), &mut env));
    println!("{:?}", eval(&Expr::Bin(num(1.0), Op::Div, num(0.0)), &mut env));
    println!("{:?}", eval(&Expr::Var("y".into()), &mut env));
    println!("{:?}", [-5, 0, 2, 8, 9].map(classify));
    let pts = [(0, 0), (3, 0), (0, -2), (1, 1)];
    for p in pts { let d = match p { (0, 0) => "origin".to_string(), (x, 0) | (0, x) => format!("axis {x}"), (x, y) if x == y => "diag".into(), _ => "other".into() }; print!("{d}; "); }
    println!();
    let v = vec![1, 2, 3, 4, 5];
    match v.as_slice() { [first, .., last] => println!("{first}..{last}"), _ => {} }
    if let [a, b, rest @ ..] = &v[..] { println!("{a} {b} {rest:?}"); }
    let msg = Some(Expr::Num(2.0));
    if let Some(Expr::Num(n @ 1.0..=5.0)) = msg { println!("in range {n}"); }
    let opt: Option<Option<i32>> = Some(None);
    println!("{}", matches!(opt, Some(None)));
    let Some(x) = "42".parse::<i32>().ok() else { panic!() }; println!("{x}");
}
