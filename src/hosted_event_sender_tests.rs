//! Loopback-only regression tests for the detached hosted event sender.
use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(3);
const SECRET: &str = "event-fixture-secret-never-in-argv";
const PATH: &str = "build/runs/1731300000000-42/event";

fn default_credentials() -> Creds {
    let mut creds = Creds::new(DEFAULT_API_BASE.into(), SECRET.into()).unwrap();
    creds.set_build_base(DEFAULT_BUILD_BASE.into()).unwrap();
    creds
}

fn envelope(key: &str, path: &str, body: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "key": key, "path": path, "body": body })).unwrap()
}

#[test]
fn event_paths_accept_only_bounded_run_event_routes() {
    for id in ["0", "123-456", &"9".repeat(64)] {
        assert!(valid_event_path(&format!("build/runs/{id}/event")), "{id}");
    }
    for path in [
        "",
        "build/runs//event",
        "build/runs/abc/event",
        "build/runs/-/event",
        "build/runs/1/event/",
        "/build/runs/1/event",
        "build/runs/1/events",
        "build/runs/1/event?x=1",
        "build/runs/1/event#fragment",
        "build/runs/../event",
        "build/runs/%31/event",
        "build/runs/1/../event",
        "build/runs/１/event",
        "build/runs/1\nevent",
        "https://example.invalid/build/runs/1/event",
        "build/runs/1/event\0",
    ] {
        assert!(!valid_event_path(path), "accepted {path:?}");
    }
    assert!(!valid_event_path(&format!(
        "build/runs/{}/event",
        "9".repeat(65)
    )));
}

#[test]
fn envelope_roundtrip_preserves_unicode_and_json_exactly() {
    let body = serde_json::json!({
        "unicode": "雪🦀", "escaped": "\\\"\n\t", "nested": [null, true, 17, {"x": "é"}]
    });
    let parsed = parse_event_envelope(envelope(SECRET, PATH, body.clone()).as_slice()).unwrap();
    assert_eq!(parsed.key, SECRET);
    assert_eq!(parsed.path, PATH);
    assert_eq!(parsed.body, body);
    let encoded = serde_json::to_vec(&parsed).unwrap();
    let again = parse_event_envelope(encoded.as_slice()).unwrap();
    assert_eq!(again.body, body);
    assert_eq!(again.key, SECRET);
    assert_eq!(again.path, PATH);
}

#[test]
fn envelope_rejects_oversize_invalid_json_keys_and_paths() {
    for key in [
        "",
        "bad key",
        "bad\r\nAuthorization: injected",
        "bad\tkey",
        "bad\0key",
    ] {
        assert!(
            parse_event_envelope(envelope(key, PATH, serde_json::json!({})).as_slice()).is_err()
        );
    }
    for path in [
        "build/runs//event",
        "build/runs/../event",
        "build/runs/1/event?redirect=1",
    ] {
        assert!(
            parse_event_envelope(envelope(SECRET, path, serde_json::json!({})).as_slice()).is_err()
        );
    }
    for raw in [b"not json".as_slice(), b"{}", b"null", b"{\"key\":123}"] {
        assert!(parse_event_envelope(raw).is_err());
    }
    let oversized = envelope(
        SECRET,
        PATH,
        serde_json::json!({"data": "x".repeat(1024 * 1024)}),
    );
    assert!(parse_event_envelope(oversized.as_slice()).is_err());
    let mut trailing = envelope(SECRET, PATH, serde_json::json!({}));
    trailing.extend_from_slice(b" {}");
    assert!(parse_event_envelope(trailing.as_slice()).is_err());
    let mut unknown = serde_json::json!({"key": SECRET, "path": PATH, "body": {}});
    unknown["url"] = serde_json::json!("https://example.invalid");
    assert!(parse_event_envelope(unknown.to_string().as_bytes()).is_err());
}

#[test]
fn parent_command_keeps_credentials_and_payload_out_of_argv() {
    let creds = default_credentials();
    let body = serde_json::json!({"message": "private payload 雪"});
    let command = event_command(&creds, PATH, &body);
    if crate::telemetry::curl_customized() {
        assert!(command.is_none());
        return;
    }
    let (command, stdin) = command.expect("default hosted endpoints use detached sender");
    assert_eq!(
        command.get_program(),
        std::env::current_exe().unwrap().as_os_str()
    );
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(args, vec![std::ffi::OsStr::new(EVENT_SENDER_ARG)]);
    for arg in args {
        let arg = arg.to_string_lossy();
        assert!(!arg.contains(SECRET));
        assert!(!arg.contains("private payload"));
    }
    let parsed = parse_event_envelope(stdin.as_bytes()).unwrap();
    assert_eq!(parsed.key, SECRET);
    assert_eq!(parsed.path, PATH);
    assert_eq!(parsed.body, body);
}

#[test]
fn parent_command_rejects_custom_endpoints_and_invalid_paths() {
    let mut creds = default_credentials();
    let body = serde_json::json!({});
    assert!(event_command(&creds, "build/runs//event", &body).is_none());
    assert!(event_command(&creds, "build/local/runs", &body).is_none());
    assert!(
        event_command(
            &Creds::anonymous(DEFAULT_API_BASE.into()).unwrap(),
            PATH,
            &body
        )
        .is_none()
    );
    creds.base = "https://example.invalid/v1".into();
    assert!(event_command(&creds, PATH, &body).is_none());
    creds = default_credentials();
    creds.build_base = "https://example.invalid".into();
    assert!(event_command(&creds, PATH, &body).is_none());
}

#[test]
fn failed_child_spawn_is_distinct_from_ambiguous_delivery() {
    // A file cannot contain a child executable. No fixture directory needed.
    let missing = std::env::current_exe()
        .unwrap()
        .join("missing-event-sender");
    let command = Command::new(missing);
    assert!(!spawn_detached((command, "no-secret-fixture".into())));
}

fn test_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .proxy(None)
        .https_only(false)
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .timeout_global(Some(TIMEOUT))
        .build()
        .into()
}

#[test]
fn production_event_agent_requires_https_and_bounds_delivery() {
    let agent = event_agent();
    let config = agent.config();
    assert!(config.https_only());
    assert!(!config.http_status_as_error());
    assert_eq!(config.max_redirects(), 0);
    assert!(!config.max_redirects_will_error());
    assert_eq!(config.timeouts().global, Some(Duration::from_secs(30)));
    assert_eq!(config.timeouts().resolve, Some(Duration::from_secs(5)));
    assert_eq!(config.timeouts().connect, Some(Duration::from_secs(5)));
}

#[test]
fn native_sender_bounds_and_validates_response_bodies() {
    for (declared, actual, success) in [
        (MAX_EVENT_BYTES, MAX_EVENT_BYTES, true),
        (MAX_EVENT_BYTES + 1, MAX_EVENT_BYTES + 1, false),
        (100, 4, false),
    ] {
        let server = listener();
        let creds = Creds::new(
            format!("http://{}", server.local_addr().unwrap()),
            SECRET.into(),
        )
        .unwrap();
        let fixture = thread::spawn(move || {
            let mut stream = accept(&server);
            request(&mut stream);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let _ = stream.write_all(&vec![b'x'; actual as usize]);
        });
        let result = send_event_native(&test_agent(), &creds, PATH, &serde_json::json!({}));
        fixture.join().unwrap();
        assert_eq!(
            result.is_ok(),
            success,
            "declared={declared}, actual={actual}"
        );
    }
}

#[test]
fn native_sender_times_out_while_draining_response() {
    let server = listener();
    let creds = Creds::new(
        format!("http://{}", server.local_addr().unwrap()),
        SECRET.into(),
    )
    .unwrap();
    let fixture = thread::spawn(move || {
        let mut stream = accept(&server);
        request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
            .unwrap();
        thread::sleep(Duration::from_millis(350));
    });
    let agent = ureq::Agent::config_builder()
        .proxy(None)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_millis(100)))
        .build()
        .new_agent();
    let start = Instant::now();
    assert!(send_event_native(&agent, &creds, PATH, &serde_json::json!({})).is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
    fixture.join().unwrap();
}

fn listener() -> TcpListener {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}

fn accept(listener: &TcpListener) -> TcpStream {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // BSD and Windows accepted sockets inherit the listener's
                // nonblocking mode. Linux ones do not.
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(TIMEOUT)).unwrap();
                stream.set_write_timeout(Some(TIMEOUT)).unwrap();
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "fixture accept timed out");
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("fixture accept failed: {error}"),
        }
    }
}

fn request(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        assert!(head.len() < 16 * 1024);
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    let length: usize = header(&head, "content-length").unwrap().parse().unwrap();
    assert!(length <= 1024 * 1024);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).unwrap();
    (head, body)
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

#[test]
fn native_sender_preserves_body_auth_and_http_status() {
    for status in [204, 401, 429, 503] {
        let server = listener();
        let creds = Creds::new(
            format!("http://{}/v1", server.local_addr().unwrap()),
            SECRET.into(),
        )
        .unwrap();
        let body = serde_json::json!({"event": "finished", "detail": "雪🦀\\\"\n", "code": 0});
        let expected = serde_json::to_vec(&body).unwrap();
        let fixture = thread::spawn(move || {
            let mut stream = accept(&server);
            let (head, received) = request(&mut stream);
            assert!(head.starts_with(&format!("POST /v1/{PATH} HTTP/1.1\r\n")));
            assert_eq!(
                header(&head, "authorization"),
                Some(format!("Bearer {SECRET}").as_str())
            );
            assert_eq!(header(&head, "content-type"), Some("application/json"));
            assert_eq!(received, expected);
            write!(
                stream,
                "HTTP/1.1 {status} Fixture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        });
        let result = send_event_native(&test_agent(), &creds, PATH, &body);
        fixture.join().unwrap();
        assert_eq!(result.unwrap(), status);
    }
}

#[test]
fn native_sender_does_not_follow_redirects() {
    let server = listener();
    let target = listener();
    let creds = Creds::new(
        format!("http://{}", server.local_addr().unwrap()),
        SECRET.into(),
    )
    .unwrap();
    let location = format!("http://{}/leaked", target.local_addr().unwrap());
    let fixture = thread::spawn(move || {
        let mut stream = accept(&server);
        request(&mut stream);
        write!(stream, "HTTP/1.1 307 Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
    });
    let result = send_event_native(&test_agent(), &creds, PATH, &serde_json::json!({}));
    fixture.join().unwrap();
    assert_eq!(result.unwrap(), 307);
    assert_eq!(
        target.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn native_sender_rejects_oversized_body_before_connecting() {
    let server = listener();
    let creds = Creds::new(
        format!("http://{}", server.local_addr().unwrap()),
        SECRET.into(),
    )
    .unwrap();
    let body = serde_json::json!({"data": "x".repeat(1024 * 1024)});
    assert!(send_event_native(&test_agent(), &creds, PATH, &body).is_err());
    assert_eq!(
        server.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
