pub fn shared(who: &str) -> String {
    format!("shared by {who}, test build: {}", cfg!(test))
}

#[cfg(test)]
mod tests {
    #[test]
    fn shares() {
        assert_eq!(super::shared("a test"), "shared by a test, test build: true");
    }

    #[test]
    #[should_panic(expected = "on purpose")]
    fn panics() {
        panic!("on purpose");
    }
}
