struct State { value: i32, wide: usize }

fn choose<'a>(left: &'a State, right: &'a State, pick_left: bool) -> &'a State {
    if pick_left { left } else { right }
}

fn relay<'a>(state: &'a State, count: i32) -> &'a State {
    if count == 0 { state } else { relay(state, count - 1) }
}

fn main() {
    let mut left = State { value: 7, wide: 18446744073709551615usize };
    let right = State { value: 11, wide: 4294967296usize };
    let selected = relay(choose(&left, &right, true), 3);
    let okay = (*selected).value == 7 && (*selected).wide == 18446744073709551615usize;
    left.value = 9;
    if okay && left.value == 9 && (*relay(&right, 1)).wide == 4294967296usize {
        println!("shared reference returns compiled correctly");
    }
}
