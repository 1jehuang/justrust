use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn actual_proxy_scenarios() {
    let capture_modes = if cfg!(feature = "candidate") {
        vec!["0", "4096"]
    } else {
        vec!["baseline"]
    };
    for capture in capture_modes {
        for scenario in ["one", "multi", "immediate", "surplus"] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_allowance-tickets-baseline"));
            command
                .args([scenario, "2000"])
                .env_remove("CARGO_MAKEFLAGS")
                .env_remove("MAKEFLAGS")
                .env_remove("MFLAGS")
                .env_remove("ALLOWANCE_CAPTURE_CAPACITY")
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
            if capture != "baseline" {
                command.env("ALLOWANCE_CAPTURE_CAPACITY", capture);
            }
            let mut child = command.spawn().unwrap();
            let deadline = Instant::now() + Duration::from_secs(25);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success(), "{scenario} capture={capture}: {status}");
                    break;
                }
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("{scenario} capture={capture} parent watchdog expired");
                }
                // Watchdog polling only, never a scenario synchronization oracle.
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
