//! A dev-dependency no test names, with an impl that would leave `1.5 == x` open.

pub struct Value;

impl PartialEq<Value> for f64 {
    fn eq(&self, _: &Value) -> bool {
        false
    }
}
