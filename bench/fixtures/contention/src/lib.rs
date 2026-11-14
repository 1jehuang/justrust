pub fn answer() -> u64 {
    42
}

#[cfg(test)]
mod tests {
    #[test]
    fn probe_answer() {
        assert_eq!(super::answer(), 42);
    }

    #[test]
    fn probe_unrelated_unit() {
        assert_eq!([2_u64, 3, 5, 7].iter().sum::<u64>(), 17);
    }
}
