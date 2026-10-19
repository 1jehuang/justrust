pub fn half(x: i32) -> i32 {
    x / 2
}

#[cfg(test)]
mod tests {
    use super::half;

    #[test]
    fn halves() {
        assert_eq!(half(8), 4);
    }

    #[test]
    fn wrong() {
        println!("captured output");
        assert_eq!(half(7), 4);
    }

    #[test]
    #[should_panic(expected = "divide by zero")]
    fn zero() {
        half(1);
        panic!("attempt to divide by zero");
    }

    #[test]
    #[ignore]
    fn skipped() {}
}
