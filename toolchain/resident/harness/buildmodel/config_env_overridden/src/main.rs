fn main() {
    println!("{:?} {:?}", option_env!("FROM_CONFIG"), option_env!("FORCED"));
}
