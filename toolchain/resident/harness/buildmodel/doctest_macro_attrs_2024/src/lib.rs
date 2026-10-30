macro_rules! wrap {
    ($(#[$a:meta])* $v:vis fn $n:ident() {}) => {
        $(#[$a])*
        $v fn $n() {}
    };
}
macro_rules! whole {
    ($i:item) => { $i };
}

wrap! {
    /// One.
    ///
    /// ```
    /// app::one();
    /// ```
    pub fn one() {}
}

whole! {
    /// Two.
    ///
    /// ```
    /// app::two();
    /// ```
    pub fn two() {}
}

pub struct S;
wrap! {
    /// Three.
    #[doc = "x"]
    /// ```
    /// app::S;
    /// ```
    pub fn three() {}
}
