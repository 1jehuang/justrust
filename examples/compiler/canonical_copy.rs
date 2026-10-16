struct Opaque {
    v: i32,
}
fn run() -> i32 {
    let owner = Opaque { v: 11 };
    let result;
    {
        let r = &owner;
        result = <&Opaque as core::clone::Clone>::clone(&r);
    }
    result.v
}
fn main() {
    if run() == 11 {
        println!("canonical copy oracle ok");
    }
}
