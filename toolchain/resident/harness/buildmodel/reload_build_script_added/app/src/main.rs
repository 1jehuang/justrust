fn main() {
    println!("{} {:?}", cfg!(scripted), option_env!("BM_FROM_SCRIPT"));
}
