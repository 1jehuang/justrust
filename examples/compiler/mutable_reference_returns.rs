struct State {
    value: i32,
    wide: usize,
}

fn choose<'a>(left: &'a mut State, right: &'a mut State, first: bool) -> &'a mut State {
    if first { left } else { right }
}

fn relay<'a>(state: &'a mut State, remaining: i32) -> &'a mut State {
    if remaining == 0 {
        state
    } else {
        relay(state, remaining - 1)
    }
}

fn main() {
    let mut left = State {
        value: 1,
        wide: 4294967296usize,
    };
    let mut right = State {
        value: 2,
        wide: 18446744073709551615usize,
    };
    let selected = choose(&mut left, &mut right, false);
    let result = relay(selected, 3);
    (*result).value = 7;
    let observed = (*result).value == 7 && (*result).wide == 18446744073709551615usize;
    left.value = 8;
    right.value = 9;
    if observed && left.value == 8 && right.value == 9 {
        println!("mutable reference returns compiled correctly");
    }
}
