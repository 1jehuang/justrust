use geom::shapes::Rect;

#[test]
fn scaled_area() {
    let r = Rect { w: 2, h: 5 }.scaled(3);
    assert_eq!(r.area(), 90);
}

#[test]
fn has_default_feature() {
    assert!(geom::has_area());
}
