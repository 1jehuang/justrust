struct Pair {
    value: i32,
    wide: usize,
}

fn maximum() -> usize {
    18446744073709551615
}

fn main() {
    let mut pair = Pair {
        value: 7,
        wide: maximum(),
    };
    let parent = &mut pair.value;
    {
        let child = &mut *parent;
        *child = *child + 2;
    }
    let seen = *parent;
    pair.value = 10;
    let shared = &pair.wide;
    let alias = shared;
    if seen == 9 && pair.value == 10 && *alias == maximum() {
        println!("local borrowing compiled correctly");
    }
}
