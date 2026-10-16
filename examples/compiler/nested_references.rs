struct Pair {
    left: i32,
    right: i32,
}

fn identity<'outer, 'inner, T>(value: &'outer &'inner T) -> &'outer &'inner T {
    value
}

fn load<'outer, 'inner, T>(value: &'outer &'inner T) -> &'inner T {
    *value
}

fn observe_once<'outer, 'inner>(
    value: &'outer &'inner Pair,
    calls: &mut i32,
) -> &'outer &'inner Pair {
    *calls += 1;
    value
}

fn run() -> i32 {
    let original = Pair { left: 6, right: 7 };
    let mut calls = 0;
    let result;
    {
        let handle = &original;
        let outer = &handle;
        result = load::<Pair>(identity::<Pair>(observe_once(outer, &mut calls)));
    }
    result.left + result.right + calls
}

fn main() {
    if run() == 14 {
        println!("nested references compiled correctly");
    }
}
