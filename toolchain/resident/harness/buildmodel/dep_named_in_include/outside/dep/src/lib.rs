pub struct Meters(pub u32);

impl From<Meters> for u64 {
    fn from(m: Meters) -> u64 {
        u64::from(m.0) + 100
    }
}
