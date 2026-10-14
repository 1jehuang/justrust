//! Real CLI against local HTTP, with no production requests.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output};

fn cli(base: &str, home: &std::path::Path, message: &str, optout: Option<&str>) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_justrust"));
    for (k, _) in std::env::vars_os() {
        let key = k.to_string_lossy();
        if key.starts_with("JUSTRUST_") || key == "DO_NOT_TRACK" || key == "JCODE_API_KEY" {
            c.env_remove(k);
        }
    }
    c.env("JUSTRUST_HOME", home)
        .env("JUSTRUST_BUILD_BASE", base)
        .env("JUSTRUST_TELEMETRY_FLUSH_SECS", "9999999999")
        .args(["feedback", message]);
    if let Some(key) = optout {
        c.env(key, "1");
    }
    c.output().unwrap()
}

#[test]
fn feedback_http_acceptance_and_rejection() {
    for code in [201, 200, 302, 400, 429, 503] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert_eq!(request, "POST /v1/feedback HTTP/1.1\r\n");
            let mut len = 0;
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h == "\r\n" {
                    break;
                }
                if let Some(n) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = n.trim().parse().unwrap();
                }
                assert!(!h.to_ascii_lowercase().starts_with("authorization:"));
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
                serde_json::json!({"message":"Synthetic é\nfeedback"})
            );
            write!(
                stream,
                "HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        });
        let home = std::env::temp_dir().join(format!("jr-feedback-{}-{code}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("settings.json"), r#"{"auto_update":false}"#).unwrap();
        let out = cli(&base, &home, "Synthetic é\nfeedback", None);
        assert_eq!(
            out.status.success(),
            code == 201,
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        if code != 201 {
            assert!(String::from_utf8_lossy(&out.stderr).contains(&format!("HTTP {code}")));
        }
        worker.join().unwrap();
        std::fs::remove_dir_all(home).unwrap();
    }
}

#[test]
fn feedback_optout_and_invalid_messages_send_nothing() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let home = std::env::temp_dir().join(format!("jr-feedback-off-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("settings.json"), r#"{"auto_update":false}"#).unwrap();
    for key in ["JUSTRUST_NO_TELEMETRY", "DO_NOT_TRACK"] {
        let out = cli(&base, &home, "test", Some(key));
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("telemetry is off"));
    }
    for message in [" ", &"é".repeat(2049)] {
        assert!(!cli(&base, &home, message, None).status.success());
    }
    std::fs::write(
        home.join("settings.json"),
        r#"{"telemetry":false,"auto_update":false}"#,
    )
    .unwrap();
    assert!(!cli(&base, &home, "test", None).status.success());
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    std::fs::remove_dir_all(home).unwrap();
}
