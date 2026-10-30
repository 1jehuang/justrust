//! tracing-opentelemetry's tests import `tracing::span` in the test itself and use it twice.
//! `#[test]` is a macro, so the import is one a macro expanded to, and rustc expands the
//! uses of it last first: the statics of one name they declare are numbered in that order.

mod sites {
    macro_rules! site {
        ($name:expr) => {{
            static SITE: (&str, u32) = ($name, line!());
            &SITE
        }};
    }
    pub(crate) use site;
}

use sites::site as beside;

#[test]
fn imported_beside_the_test() {
    let first = beside!("first");
    let second = beside!("second");
    assert_eq!((first.0, second.0), ("first", "second"));
}

#[test]
fn imported_in_the_test() {
    use sites::site;
    let first = site!("first");
    let second = site!("second");
    assert_eq!((first.0, second.0), ("first", "second"));
    assert_eq!((first.1, second.1), (line!() - 3, line!() - 2));
}

#[test]
fn imported_in_a_block() {
    let first = {
        use sites::site as inner;
        inner!("first")
    };
    use sites::site;
    let second = site!("second");
    let in_closure = || (site!("third"), site!("fourth"));
    let (third, fourth) = in_closure();
    assert_eq!((first.0, second.0, third.0, fourth.0), ("first", "second", "third", "fourth"));
}
