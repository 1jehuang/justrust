//! What only the tests of `devdep` depend on.

pub struct Range {
    pub from: u32,
    pub to: u32,
}

impl Range {
    pub fn sum(&self, of: impl Fn(u32) -> u32) -> u32 {
        (self.from..self.to).map(of).sum()
    }
}

#[macro_export]
macro_rules! range {
    ($from:expr, $to:expr) => {
        $crate::Range { from: $from, to: $to }
    };
}
