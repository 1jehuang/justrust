use blanket::AsAny;

struct Local(u32);

#[test]
fn downcasts() {
    let local = Local(3);
    assert_eq!(local.as_any().downcast_ref::<Local>().map(|it| it.0), Some(3));
    assert!(!7u8.as_any().is::<u32>());
}
