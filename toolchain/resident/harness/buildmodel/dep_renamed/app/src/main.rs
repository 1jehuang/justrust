fn main() {
    println!("{} from {}", other_name::scale(std::env::args().count() as u32), other_name::origin());
}
