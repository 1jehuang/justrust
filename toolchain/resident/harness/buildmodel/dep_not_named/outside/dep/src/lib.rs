pub fn scale(n: u32) -> u32 {
    n * 3
}

pub fn kept<T: Into<u64>>(of: T) -> u32 {
    of.into() as u32 + 100
}

pub fn origin() -> &'static str {
    "a path outside the workspace"
}
