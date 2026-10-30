//! Statics of one name from macros that rustc expands out of the order they are written in:
//! one it finds by an import that a macro expanded to waits for the others.

macro_rules! site {
    ($name:expr) => {{
        static SITE: (&str, u32) = ($name, line!());
        &SITE
    }};
}
pub(crate) use site;

macro_rules! items {
    ($($item:item)*) => { $($item)* };
}

items! {
    mod expanded {
        use super::site as imported;
        use super::site;

        pub fn names() -> Vec<&'static str> {
            let first = imported!("first");
            let second = site!("second");
            let third = imported!("third");
            let fourth = site!("fourth");
            let fifth = imported!("fifth");
            let counts = [1, 2, 3].map(|it| it + imported!("in a closure").0.len());
            let all = [first, second, third, fourth, fifth];
            assert!(counts[0] < counts[2] && all[0].1 < all[4].1);
            all.iter().map(|it| it.0).collect()
        }
    }
}

pub fn run() -> Vec<&'static str> {
    expanded::names()
}
