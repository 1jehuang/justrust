fn main() {
 std::fs::write("script.pid", std::process::id().to_string()).unwrap();
 std::thread::sleep(std::time::Duration::from_secs(60));
}
