//! Found in a production validator: a guarded match nested in another arm's guard cleared
//! the enclosing arm's binding references. Later reads in the outer guard then used the
//! by-value locals before they were initialized, producing wrong results or a crash.

use std::hint::black_box;

enum Expr {
    Bin { left: Box<Expr>, minus: bool, right: Box<Expr> },
    Ident(String),
    Number(String),
}

fn day_offset(expr: Expr) -> bool {
    matches!(expr,
        Expr::Bin { left, minus: true, right }
            if matches!(*left, Expr::Ident(ref s) if s == "day")
                && matches!(*right, Expr::Number(ref n) if n.parse::<u32>().is_ok()))
}

fn unrelated_inner_guard(pair: (String, String), n: u8) -> bool {
    match pair {
        (_, b) if matches!(n, m if m == 1) && b == "b" => true,
        _ => false,
    }
}

fn in_a_let(pair: (String, String)) -> String {
    match pair {
        (a, b)
            if {
                let inner = matches!(a.len(), l if l > 0);
                inner && b.starts_with('b')
            } =>
        {
            format!("{a}{b}")
        }
        (a, _) => format!("other {a}"),
    }
}

fn inner_or(pair: (String, String), n: u8) -> bool {
    match pair {
        (_, b) if matches!((0, n), (m, _) | (_, m) if m == 1) && b == "b" => true,
        _ => false,
    }
}

fn outer_or(pair: (String, String), n: u8) -> String {
    match pair {
        (a, _) | (_, a) if matches!(n, m if m == 1) && a == "b" => a,
        _ => "other".into(),
    }
}

fn unguarded_inner(pair: (String, String), n: u8) -> bool {
    match pair {
        (a, b)
            if (match n {
                1 => a == "a",
                _ => false,
            }) && b == "b" =>
        {
            true
        }
        _ => false,
    }
}

fn inner_body(pair: (String, String), inner: Option<String>) -> bool {
    match pair {
        (a, b)
            if (match inner {
                Some(s) if s == "a" => {
                    let owned = s;
                    owned == a
                }
                _ => false,
            }) && b == "b" =>
        {
            true
        }
        _ => false,
    }
}

fn expr(left: &str, minus: bool, right: &str) -> Expr {
    Expr::Bin {
        left: Box::new(Expr::Ident(left.to_owned())),
        minus,
        right: Box::new(Expr::Number(right.to_owned())),
    }
}

pub fn run() -> Vec<String> {
    let mut out = Vec::new();
    for (left, minus, right) in [
        ("day", true, "42"),
        ("day", true, "0"),
        ("day", false, "42"),
        ("month", true, "1"),
        ("day", true, "1.5"),
    ] {
        out.push(format!(
            "{left} {minus} {right}: {}",
            day_offset(black_box(expr(left, minus, right)))
        ));
    }
    for n in [0, 1] {
        out.push(format!(
            "unrelated {n}: {}",
            unrelated_inner_guard(black_box(("a".into(), "b".into())), black_box(n))
        ));
        out.push(format!(
            "inner or {n}: {}",
            inner_or(black_box(("a".into(), "b".into())), black_box(n))
        ));
        out.push(format!(
            "outer or {n}: {}",
            outer_or(black_box(("a".into(), "b".into())), black_box(n))
        ));
        out.push(format!(
            "unguarded {n}: {}",
            unguarded_inner(black_box(("a".into(), "b".into())), black_box(n))
        ));
    }
    out.push(format!("in a let: {}", in_a_let(black_box(("a".into(), "bee".into())))));
    out.push(format!("in a let, fails: {}", in_a_let(black_box((String::new(), "bee".into())))));
    for inner in [Some("a".into()), Some("other".into()), None] {
        out.push(format!(
            "inner body: {}",
            inner_body(black_box(("a".into(), "b".into())), black_box(inner))
        ));
    }
    out
}
