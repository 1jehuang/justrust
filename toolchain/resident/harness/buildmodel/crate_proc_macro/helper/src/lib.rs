pub fn label(name: &str) -> String {
    format!("<{}>", name.to_lowercase())
}

pub fn scaled(n: u64) -> u64 {
    n * 2
}
