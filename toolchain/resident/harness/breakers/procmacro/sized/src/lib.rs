//! A type whose layout a feature changes.

pub enum Record {
    Short(u32),
    Long(Body),
}

pub struct Body {
    pub words: Vec<u32>,
    #[cfg(feature = "wide")]
    pub wide: [u64; 3],
}

pub fn records() -> Vec<Record> {
    let words = vec![2, 3];
    #[cfg(feature = "wide")]
    let body = Body { words, wide: [0; 3] };
    #[cfg(not(feature = "wide"))]
    let body = Body { words };
    vec![Record::Short(1), Record::Long(body)]
}
