//! A panic in what a macro wrote is reported from where the macro is called. ra-jit took that
//! to start at the comments and attributes on top of a call in item position, lines too early:
//! simd-json's `fail!(fail02);` under a comment panicked a line above where rustc said.

macro_rules! located {
    ($name:ident) => {
        fn $name() -> (u32, u32, u32) {
            let unwrapped = std::panic::catch_unwind(|| None::<u32>.unwrap()).unwrap_err();
            drop(unwrapped);
            let here = std::panic::Location::caller();
            (line!(), here.line(), column!())
        }
    };
}

located!(plain);

// A comment on top of the call.
located!(commented);

// A comment and attributes.
#[cfg(not(feature = "none_of_this_name"))]
#[allow(dead_code)]
located!(attributed);

pub fn run() -> Vec<(u32, u32, u32)> {
    let hook = std::panic::take_hook();
    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = lines.clone();
    std::panic::set_hook(Box::new(move |info| {
        let at = info.location().unwrap();
        seen.lock().unwrap().push((at.line(), at.column(), 0));
    }));
    let mut all = vec![plain(), commented(), attributed()];
    std::panic::set_hook(hook);
    all.extend(lines.lock().unwrap().iter());
    all
}
