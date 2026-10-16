enum State { Ready, Busy }
struct Pair { left: i32, right: i32 }

fn choose(flag: bool) -> Pair {
    let pair: Pair;
    if flag {
        pair = Pair { left: 3, right: 7 };
    } else {
        pair = Pair { left: 11, right: 13 };
    }
    pair
}

fn main() {
    let ignored: State;
    match ignored { _ => {} }
    let pair: Pair;
    pair = choose(true);
    let state: State;
    state = State::Ready;
    let result: i32;
    match state {
        State::Ready => { result = pair.left + pair.right; },
        State::Busy => { result = 0; },
    }
    if result == 10 { println!("deferred initialization compiled correctly"); }
}
