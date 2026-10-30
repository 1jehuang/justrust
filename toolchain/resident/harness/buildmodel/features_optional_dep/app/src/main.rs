fn main() {
    println!("lib: {:?} {}", lib::enabled(), lib::fancy());
    #[cfg(feature = "with-opt")]
    println!("opt: {:?}", opt::enabled());
    #[cfg(not(feature = "with-opt"))]
    println!("opt: absent");
    println!("app: with-opt {} loud {} weak {}", cfg!(feature = "with-opt"), cfg!(feature = "loud"), cfg!(feature = "weak"));
}
