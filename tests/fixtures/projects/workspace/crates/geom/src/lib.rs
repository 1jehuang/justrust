pub mod shapes;

pub fn double(x: i64) -> i64 {
    x * 2
}

#[cfg(feature = "area")]
pub fn has_area() -> bool {
    true
}

#[cfg(feature = "fancy")]
pub fn fancy() -> i64 {
    42
}

#[cfg(test)]
mod tests {
    use super::double;
    use crate::shapes::Rect;

    #[test]
    fn doubles() {
        assert_eq!(double(21), 42);
    }

    #[test]
    fn rect_area() {
        let r = Rect { w: 3, h: 4 };
        assert_eq!(r.area(), 12);
    }
}
