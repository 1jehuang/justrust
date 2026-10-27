// Nine failures force whole-group confirmation. Eight binaries demonstrate
// whether confirmations serialize, without depending on compiler behavior.
macro_rules! failure {
    ($name:ident) => {
        #[test]
        fn $name() {
            std::thread::sleep(std::time::Duration::from_millis(400));
            panic!("intentional retry-contention failure");
        }
    };
}
failure!(failure_0);
failure!(failure_1);
failure!(failure_2);
failure!(failure_3);
failure!(failure_4);
failure!(failure_5);
failure!(failure_6);
failure!(failure_7);
failure!(failure_8);

#[test]
fn passing_control() {}
