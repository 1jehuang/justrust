fn sum_to(limit: i32) -> i32 {
    let mut total: i32 = 0;
    let mut n = 1;
    while n <= limit {
        total = total + n;
        n = n + 1;
    }
    total
}

fn main() {
    let result = sum_to(10);
    if result == 55 && -7 / 3 == -2 && -7 % 3 == -1 {
        println!("arithmetic ok");
    } else {
        println!("arithmetic failed");
    }
}
