struct Wrap<T> {
    value: T,
}

enum Maybe<T> {
    None,
    Some(T),
}

fn id<T>(value: T) -> T {
    value
}

fn relay<T>(value: T, remaining: i32) -> T {
    if remaining == 0 {
        id(value)
    } else {
        relay(value, remaining - 1)
    }
}

fn take<T>(value: Maybe<T>, fallback: T) -> T {
    match value {
        Maybe::None => fallback,
        Maybe::Some(value) => value,
    }
}

fn main() {
    let small = Wrap { value: 7u8 };
    let large: Wrap<u64> = Wrap {
        value: 18446744073709551615u64,
    };
    let absent: Maybe<u16> = Maybe::None;
    let present = Maybe::Some(9u16);
    let pair = id((3u8, 4u16));
    let mut effects = 0;
    let once: u8 = id({
        effects = effects + 1;
        7
    });
    let relayed = relay(small, 3);
    let explicit = id::<Wrap<u64>>(large);
    let truncated = id(256) as u8;
    let left_peer = id(1) + 2u8;
    let right_peer = 1 + id(2u8);
    if relayed.value == 7u8
        && explicit.value == 18446744073709551615u64
        && take(absent, 5u16) == 5u16
        && take(present, 0u16) == 9u16
        && pair.0 == 3u8
        && pair.1 == 4u16
        && once == 7u8
        && effects == 1
        && truncated == 0u8
        && left_peer == 3u8
        && right_peer == 3u8
    {
        println!("generic inference compiled correctly");
    }
}
