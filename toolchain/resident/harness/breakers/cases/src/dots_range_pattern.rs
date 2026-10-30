//! autocfg is of the 2015 edition and writes inclusive range patterns with three dots. The
//! pattern was not lowered, so the closure that holds it had no body.

pub fn run() -> (String, [u8; 4]) {
    breakers2015::dots_range_pattern::run()
}
