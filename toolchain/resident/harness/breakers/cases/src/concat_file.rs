//! tracing-subscriber compares a message with `concat!("...", file!(), ":")`. `file!()` is
//! expanded to a placeholder that is given its value where the literal is used, and the
//! literal `concat!` made of it had the placeholder in it.

macro_rules! place {
    () => {
        concat!(file!(), " at ", line!())
    };
}

pub fn run() -> [&'static str; 4] {
    let direct = concat!(
        "name: event ",
        file!(),
        ":"
    );
    let numbers = concat!("line ", line!(), ", column ", column!(), ' ', true, 7);
    let through = place!();
    let plain = concat!("no", " place");
    [direct, numbers, through, plain]
}
