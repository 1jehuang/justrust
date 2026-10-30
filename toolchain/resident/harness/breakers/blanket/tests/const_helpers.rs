//! pin-project-lite puts a helper in a `const _` beside each struct, generic as the struct is.
//! Several in one function have one path, and the generic ones were taken for the one rustc
//! has a symbol of: compiled with nothing for `T`, and defined twice.

struct Plain<'a>(&'a u8);
struct Held<T>(T);
struct Kept<T: 'static>(T);

#[test]
fn helpers() {
    const _: () = {
        fn field<'a>(this: &Plain<'a>) -> &'a u8 {
            this.0
        }
        assert!(std::mem::size_of::<fn(&Plain<'static>) -> &'static u8>() != 0);
        let _ = field;
    };
    const _: () = {
        fn field<T>(this: &Held<T>) {
            let _ = &this.0;
        }
    };
    const _: () = {
        fn field<T: 'static>(this: &Kept<T>) {
            let _ = &this.0;
        }
    };
    assert_eq!(Held(3u8).0 + *Plain(&4).0 + Kept(1u8).0, 8);
}
