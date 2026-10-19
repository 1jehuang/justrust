pub struct Rect {
    pub w: i64,
    pub h: i64,
}

impl Rect {
    pub fn area(&self) -> i64 {
        self.w * self.h
    }
    pub fn scaled(&self, k: i64) -> Rect {
        Rect { w: crate::double(self.w) * k / 2, h: self.h * k }
    }
}
