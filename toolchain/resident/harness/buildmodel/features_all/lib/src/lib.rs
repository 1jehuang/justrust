pub fn enabled() -> Vec<&'static str> {
    let mut on = Vec::new();
    if cfg!(feature = "default") { on.push("default"); }
    if cfg!(feature = "std") { on.push("std"); }
    if cfg!(feature = "fancy") { on.push("fancy"); }
    if cfg!(feature = "extra") { on.push("extra"); }
    if cfg!(feature = "opt") { on.push("opt"); }
    on
}

#[cfg(feature = "fancy")]
pub fn fancy() -> String {
    "fancy code".to_owned()
}

#[cfg(not(feature = "fancy"))]
pub fn fancy() -> String {
    "plain code".to_owned()
}
