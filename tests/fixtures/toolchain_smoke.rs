pub fn quotient_and_remainder(value: u128, divisor: u128) -> (u128, u128) {
    (value / divisor, value % divisor)
}

#[cfg(test)]
mod tests {
    use super::quotient_and_remainder;

    #[test]
    fn optimized_runtime_integer_arithmetic() {
        let value = (1_u128 << 119) + 0x123456789abcdef;
        let divisor = (1_u128 << 67) + 139;
        let (quotient, remainder) = quotient_and_remainder(value, divisor);
        assert_eq!(quotient * divisor + remainder, value);
        assert!(remainder < divisor);
    }

    #[test]
    fn std_allocation_strings_and_iterators() {
        let numbers: Vec<u64> = "2,3,5,7,11"
            .split(',')
            .map(|part| part.parse().unwrap())
            .collect();
        assert_eq!(numbers.iter().sum::<u64>(), 28);
        assert_eq!(format!("owned {}", numbers.len()), "owned 5");
    }

    #[test]
    fn std_threads_and_synchronization() {
        let value = std::sync::Arc::new(std::sync::Mutex::new(40));
        let child_value = value.clone();
        std::thread::spawn(move || *child_value.lock().unwrap() += 2)
            .join()
            .unwrap();
        assert_eq!(*value.lock().unwrap(), 42);
    }
}
