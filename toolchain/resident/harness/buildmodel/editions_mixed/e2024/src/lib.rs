use std::cell::RefCell;

unsafe extern "C" {
    pub safe fn abs(n: i32) -> i32;
}

fn tail(cell: &RefCell<u32>) -> u32 {
    *cell.borrow()
}

pub fn run(n: u32) -> String {
    let r#gen = RefCell::new(n);
    let value = if let Some(it) = Some(tail(&r#gen)) { it } else { 0 };
    format!("2024:{}", value + abs(-2) as u32)
}
