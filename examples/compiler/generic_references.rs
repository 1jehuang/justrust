trait Field {
    fn field(&self) -> &i32;
    fn field_mut(&mut self) -> &mut i32;
}

struct Cell {
    value: i32,
}

impl Field for Cell {
    fn field(&self) -> &i32 {
        &self.value
    }
    fn field_mut(&mut self) -> &mut i32 {
        &mut self.value
    }
}

impl Cell {
    fn get(&self) -> i32 {
        self.value
    }
    fn set(&mut self, value: i32) -> &mut i32 {
        self.value = value;
        &mut self.value
    }
}

fn shared<T: Field>(value: &T) -> &i32 {
    value.field()
}
fn mutable<T: Field>(value: &mut T) -> &mut i32 {
    value.field_mut()
}
fn identity<T>(value: &T) -> &T {
    value
}

fn run() -> i32 {
    let mut cell = Cell { value: 3 };
    *mutable::<Cell>(&mut cell) = 4;
    let mut iteration = 0;
    while iteration < 3 {
        let result = cell.set(cell.get() + 1);
        let _ = *result;
        iteration += 1;
    }
    let observed = *shared::<Cell>(&cell);
    let expected = 7;
    if observed == *identity::<i32>(&expected) {
        7
    } else {
        0
    }
}

fn main() {
    if run() == 7 {
        println!("generic references compiled correctly");
    }
}
