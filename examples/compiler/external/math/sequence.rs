pub fn sum(n: i32) -> i32 {
    if n == 0 { 0 } else { super::add(n, self::sum(n - 1)) }
}
