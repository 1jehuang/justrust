pub fn first_position(needle: u8, bytes: &[u8]) -> Option<usize> {
    memchr::memchr(needle, bytes)
}

#[cfg(test)]
mod tests {
    use super::first_position;

    #[test]
    fn finds_expected_position() {
        assert_eq!(first_position(b'r', b"justrust"), Some(4));
    }

    #[test]
    fn reports_absent_byte() {
        assert_eq!(first_position(b'x', b"justrust"), None);
    }
}
