struct Number {
    value: i32,
}

struct Wrapped<T> {
    value: T,
}

trait Value {
    fn value(self) -> i32;
}

trait Make {
    fn make(value: i32) -> Self;
}

trait Offset {
    fn offset(self, amount: i32) -> i32;
}

impl Make for Number {
    fn make(value: i32) -> Self {
        Number { value }
    }
}

impl Value for Number {
    fn value(self) -> i32 {
        self.value
    }
}

impl Offset for Number {
    fn offset(mut self, amount: i32) -> i32 {
        self.value = self.value + amount;
        self.value
    }
}

impl<T: Value> Value for Wrapped<T> {
    fn value(self) -> i32 {
        self.value.value()
    }
}

fn extract<T: Value>(value: T) -> i32 {
    <T as Value>::value(value)
}

fn forward<T: Value>(value: T) -> i32 {
    extract::<T>(value)
}

fn use_two<T: Value + Offset>(first: T, second: T) -> i32 {
    first.value() + second.offset(1)
}

fn receiver(trace: &mut i32) -> Number {
    *trace = *trace * 10 + 1;
    Number { value: 3 }
}

fn argument(trace: &mut i32) -> i32 {
    *trace = *trace * 10 + 2;
    4
}

fn main() {
    let nested = Wrapped::<Wrapped<Number>> {
        value: Wrapped::<Number> {
            value: <Number as Make>::make(7),
        },
    };
    let value = forward::<Wrapped<Wrapped<Number>>>(nested);
    let combined = use_two::<Number>(Number { value: 3 }, Number { value: 4 });
    let mut trace = 0;
    let ordered = receiver(&mut trace).offset(argument(&mut trace));
    if value == 7 && combined == 8 && ordered == 7 && trace == 12 {
        println!("trait obligations compiled correctly");
    }
}
