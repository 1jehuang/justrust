struct Leaf {
    n: i32,
}

struct Pair {
    left: Leaf,
    right: Leaf,
}

fn make() -> Pair {
    Pair {
        left: Leaf { n: 3 },
        right: Leaf { n: 8 },
    }
}

fn run() -> i32 {
    let mut pair = make();
    let Pair { right, .. } = pair;
    let _ = pair.right;
    pair.right = Leaf { n: 5 };
    let Pair {
        left: Leaf { n: mut value },
        right: Leaf { n: other },
    } = pair;
    value = value + other;
    let (old, (flag, _)): (i32, (bool, ())) = (value, (true, ()));
    if flag { old + right.n } else { 0 }
}

fn main() {
    if run() == 16 {
        println!("let patterns and selective moves compiled correctly");
    }
}
