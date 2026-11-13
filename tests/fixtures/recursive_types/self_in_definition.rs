// `Self` inside struct and enum definitions names the nominal itself.
struct List {
    value: i32,
    next: Option<Box<Self>>,
}

enum Expr {
    Num(i64),
    Add(Box<Self>, Box<Expr>),
    Neg(Box<Self>),
}

struct Graph
where
    Self: Sized,
{
    edges: Vec<(usize, usize)>,
}

fn sum(list: &List) -> i32 {
    match &list.next {
        Some(next) => list.value + sum(next),
        None => list.value,
    }
}

fn eval(expr: &Expr) -> i64 {
    match expr {
        Expr::Num(value) => *value,
        Expr::Add(left, right) => eval(left) + eval(right),
        Expr::Neg(inner) => -eval(inner),
    }
}

fn main() {
    let list = List {
        value: 1,
        next: Some(Box::new(List {
            value: 2,
            next: Some(Box::new(List { value: 3, next: None })),
        })),
    };
    let expr = Expr::Add(
        Box::new(Expr::Num(4)),
        Box::new(Expr::Neg(Box::new(Expr::Num(10)))),
    );
    let graph = Graph { edges: vec![(0, 1), (1, 2)] };
    println!("{} {} {}", sum(&list), eval(&expr), graph.edges.len());
}
