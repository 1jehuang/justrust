struct Loud(&'static str);

impl Drop for Loud {
    fn drop(&mut self) {
        println!("dropped {}", self.0);
    }
}

fn fails(n: usize) -> usize {
    let _loud = Loud("in the panicking frame");
    if n > 0 {
        panic!("asked to");
    }
    n
}

fn main() {
    std::panic::set_hook(Box::new(|_| println!("hook ran")));
    println!("cfg(panic = \"abort\"): {}", cfg!(panic = "abort"));
    println!("cfg(panic = \"unwind\"): {}", cfg!(panic = "unwind"));
    let caught = std::panic::catch_unwind(|| fails(std::env::args().count()));
    println!("caught: {}", caught.is_err());
}
