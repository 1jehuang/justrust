fn main() {
    println!("target sees {}, the build script saw {}", shared::enabled(), env!("HOST_SEES"));
}
