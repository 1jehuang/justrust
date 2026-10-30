fn main() {
    println!("second: {} {}", app::shared("second"), env!("CARGO_BIN_NAME"));
}
