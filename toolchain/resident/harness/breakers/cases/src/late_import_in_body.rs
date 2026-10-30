//! As `late_import_macro`, with the import in the function the macro is used in: a macro
//! expanded to the function, so rustc resolves the import after it first tries the uses.

macro_rules! site {
    ($name:expr) => {{
        static SITE: (&str, u32) = ($name, line!());
        &SITE
    }};
}
pub(crate) use site;

macro_rules! items {
    ($($item:item)*) => { $($item)* };
}

items! {
    fn names() -> Vec<&'static str> {
        use self::site as imported;
        let first = imported!("first");
        let second = site!("second");
        let third = imported!("third");
        let in_block = {
            use self::site as inner;
            [inner!("fourth"), imported!("fifth"), inner!("sixth")]
        };
        let in_closure = || [imported!("seventh"), imported!("eighth")];
        let mut all = vec![first, second, third];
        all.extend(in_block);
        all.extend(in_closure());
        all.iter().map(|it| it.0).collect()
    }
}

fn written() -> [&'static str; 2] {
    use self::site as imported;
    [imported!("one").0, imported!("two").0]
}

pub fn run() -> (Vec<&'static str>, [&'static str; 2]) {
    (names(), written())
}
