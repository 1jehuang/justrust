struct State {
    value: i32,
    wide: usize,
}

fn advance(state: &mut State, count: i32) {
    if count > 0 {
        (*state).value = (*state).value + 1;
        advance(state, count - 1);
    }
}

fn inspect(state: &State) -> i32 {
    if (*state).wide == 18446744073709551615usize {
        (*state).value
    } else {
        0
    }
}

fn main() {
    let mut state = State {
        value: 4,
        wide: 18446744073709551615usize,
    };
    let handle = &mut state;
    advance(handle, 3);
    advance(handle, 2);
    if inspect(handle) == 9 {
        println!("reference calls compiled correctly");
    }
}
