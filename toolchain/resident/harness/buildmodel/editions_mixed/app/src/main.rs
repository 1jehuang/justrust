fn main() {
    let n = std::env::args().count() as u32;
    println!("{} {} {} {}", e2015::run(n), e2018::run(n), e2021::run(n), e2024::run(n));
}
