mod inner {
    pub fn async(dyn: u32) -> u32 {
        dyn + 1
    }
}

pub fn run(n: u32) -> String {
    let r#try = ::inner::async(n);
    format!("2015:{}", r#try)
}
