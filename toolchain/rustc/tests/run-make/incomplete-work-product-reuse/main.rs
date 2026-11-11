mod independent {
    #[inline(never)]
    pub fn value() -> u32 {
        2
    }
}

mod edited {
    #[inline(never)]
    pub fn value() -> u32 {
        40
    }
}

fn main() {
    println!("{}", independent::value() + edited::value());
}
