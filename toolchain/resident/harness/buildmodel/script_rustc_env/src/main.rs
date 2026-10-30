fn main() {
    println!("{} {:?}", env!("FROM_SCRIPT"), option_env!("FROM_SCRIPT"));
}
