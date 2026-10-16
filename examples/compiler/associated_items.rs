struct Wrap<T> {
    value: T,
}

impl<T> Wrap<T> {
    fn make(value: T) -> Self {
        Self { value }
    }

    fn pair<U>(self, other: U) -> (T, U) {
        (self.value, other)
    }

    fn recur<U>(self, other: U, remaining: i32) -> (T, U) {
        if remaining == 0 {
            (self.value, other)
        } else {
            self.recur::<U>(other, remaining - 1)
        }
    }
}

struct Counter {
    value: i32,
}

impl Counter {
    fn add(mut self, increment: i32) -> Self {
        self.value += increment;
        self
    }

    fn combine(self, left: i32, right: i32) -> i32 {
        self.value + left + right
    }
}

fn receiver(effects: &mut i32) -> Counter {
    *effects = *effects * 10 + 1;
    Counter { value: 1 }
}

fn argument(effects: &mut i32, digit: i32) -> i32 {
    *effects = *effects * 10 + digit;
    digit
}

fn relay<T, U>(value: T, other: U) -> (T, U) {
    Wrap::<T>::make(value).pair::<U>(other)
}

fn main() {
    let pair = relay::<u64, u8>(18446744073709551615u64, 7u8);
    let recursive = Wrap::<i32>::make(3).recur::<u16>(4u16, 3);
    let updated = Counter { value: 2 }.add(5);
    let mut effects = 0;
    let ordered =
        receiver(&mut effects).combine(argument(&mut effects, 2), argument(&mut effects, 3));
    if pair.0 == 18446744073709551615u64
        && pair.1 == 7u8
        && recursive.0 == 3
        && recursive.1 == 4u16
        && updated.value == 7
        && ordered == 6
        && effects == 123
    {
        println!("associated items compiled correctly");
    }
}
