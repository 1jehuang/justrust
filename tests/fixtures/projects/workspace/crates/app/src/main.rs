use geom::shapes::Rect;

fn total(a: &Rect, b: &Rect) -> i64 {
    a.area() + b.area()
}

fn main() {
    let a = Rect { w: 1, h: 2 };
    let b = Rect { w: 3, h: 4 };
    println!("total area {}", total(&a, &b));
    println!("fancy {}", geom::fancy());
    println!("double {}", app_helper());
}

fn app_helper() -> i64 {
    geom::double(5)
}

#[test]
fn helper_works() {
    assert_eq!(app_helper(), 10);
}
