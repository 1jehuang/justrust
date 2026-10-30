fn main() {
    let answer = std::env::var("DEP_NATIVEISH_ANSWER").unwrap();
    println!("cargo:rustc-env=ANSWER={answer}");
}
