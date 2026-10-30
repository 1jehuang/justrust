fn main() {
    println!("dep {} {} {}", dep::version(), feat::voice(), tool::name());
    println!("{} {:?}", cfg!(scripted), option_env!("BM_FROM_SCRIPT"));
}
