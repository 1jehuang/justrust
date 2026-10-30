fn main() {
    // A future that panics is caught; the message would only differ in where it was printed.
    std::panic::set_hook(Box::new(|_| {}));
    futs::run();
}
