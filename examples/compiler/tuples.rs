fn step(pair: (i32, i32)) -> (i32, i32) {
    (pair.1, pair.0 + pair.1)
}

fn fibonacci(n: i32) -> (i32, (i32, bool)) {
    let mut pair = (0, 1);
    let mut i = 0;
    while i < n {
        pair = step(pair);
        i = i + 1;
    }
    (pair.0, (pair.1, true))
}

fn first_before_mutation(first: (i32,), second: (i32,)) -> i32 {
    first.0 * 10 + second.0
}

fn main() {
    let result = fibonacci(10);
    if result.0 == 55 && result.1.0 == 89 && result.1.1 {
        println!("nested tuple return compiled correctly");
    }
    let mut value = (3,);
    let snapshot = first_before_mutation(value, { value.0 = 7; value });
    if snapshot == 37 && value.0 == 7 {
        println!("tuple arguments preserve value snapshots");
    }
    let selected = loop { break (true, ((), 42)); };
    if selected.0 && selected.1.1 == 42 {
        println!("aggregate loop value compiled correctly");
    }
}
