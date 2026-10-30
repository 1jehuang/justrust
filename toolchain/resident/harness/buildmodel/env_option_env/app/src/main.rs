fn main() {
    println!("{:?} {:?}", option_env!("BM_OPTIONAL"), option_env!("BM_NEVER_SET"));
    println!("{}", dep::read());
}
