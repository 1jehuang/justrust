//! Loopback-only regression tests for the native telemetry transport.
use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

const FIXTURE_TIMEOUT: Duration = Duration::from_secs(3);

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .proxy(None)
        .https_only(false)
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .timeout_global(Some(timeout))
        .build()
        .into()
}

fn listener() -> TcpListener {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}

fn accept(listener: &TcpListener) -> TcpStream {
    let deadline = Instant::now() + FIXTURE_TIMEOUT;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_read_timeout(Some(FIXTURE_TIMEOUT)).unwrap();
                stream.set_write_timeout(Some(FIXTURE_TIMEOUT)).unwrap();
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

#[derive(Debug)]
struct Request {
    head: String,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }
}

fn request(stream: &mut TcpStream) -> Request {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        assert!(head.len() < 16 * 1024, "oversized request headers");
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    let mut result = Request {
        head: String::from_utf8(head).unwrap(),
        body: Vec::new(),
    };
    let length: usize = result.header("content-length").unwrap().parse().unwrap();
    assert!(length < 1024 * 1024);
    result.body.resize(length, 0);
    stream.read_exact(&mut result.body).unwrap();
    result
}

fn response(stream: &mut TcpStream, status: u16, body: &[u8], extra: &str) {
    write!(
        stream,
        "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\n{extra}\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
}

fn assert_outcome(outcome: PostOutcome, expected: Delivery, status: Option<u16>) {
    assert_eq!(outcome.delivery, expected);
    assert_eq!(outcome.http_status, status);
    assert_eq!(outcome.curl_exit, None, "native requests never invoke curl");
}

#[test]
fn native_post_preserves_json_unicode_and_auth_without_sticky_headers() {
    let server = listener();
    let url = format!(
        "http://{}/telemetry?fixture=1",
        server.local_addr().unwrap()
    );
    let body = r#"{"escaped":"\\\"\n\u263a","unicode":"雪🦀","nested":{"value":1}}"#;
    let expected = body.as_bytes().to_vec();
    let fixture = thread::spawn(move || {
        // Both requests must use this one connection. An unread response body
        // or an accidental new Agent for every POST makes the second read fail.
        let mut stream = accept(&server);
        for authenticated in [true, false] {
            let received = request(&mut stream);
            assert!(
                received
                    .head
                    .starts_with("POST /telemetry?fixture=1 HTTP/1.1\r\n")
            );
            assert_eq!(received.body, expected);
            assert_eq!(received.header("content-type"), Some("application/json"));
            assert_eq!(
                received.header("authorization"),
                authenticated.then_some("Bearer fixture-secret-not-on-command-line")
            );
            response(&mut stream, 200, b"response must be drained for reuse", "");
        }
    });
    let client = agent(FIXTURE_TIMEOUT);
    let credentials = crate::remote_hosted::Creds::new(
        "https://example.invalid".into(),
        "fixture-secret-not-on-command-line".into(),
    )
    .unwrap();
    let first = post_native(&client, &url, body, Some(&credentials));
    let second = post_native(&client, &url, body, None);
    fixture.join().unwrap();
    assert_outcome(first, Delivery::Accepted, Some(200));
    assert_outcome(second, Delivery::Accepted, Some(200));
}

#[test]
fn native_post_maps_http_statuses_without_status_errors() {
    for (status, expected) in [
        (200, Delivery::Accepted),
        (201, Delivery::Accepted),
        (204, Delivery::Accepted),
        (299, Delivery::Accepted),
        (401, Delivery::Auth),
        (403, Delivery::Auth),
        (400, Delivery::Rejected),
        (422, Delivery::Rejected),
        (301, Delivery::Retry),
        (302, Delivery::Retry),
        (307, Delivery::Retry),
        (308, Delivery::Retry),
        (404, Delivery::Retry),
        (408, Delivery::Retry),
        (429, Delivery::Retry),
        (500, Delivery::Retry),
        (503, Delivery::Retry),
        (600, Delivery::Retry),
        (999, Delivery::Retry),
    ] {
        let server = listener();
        let url = format!("http://{}/", server.local_addr().unwrap());
        let fixture = thread::spawn(move || {
            let mut stream = accept(&server);
            request(&mut stream);
            response(&mut stream, status, b"", "Connection: close\r\n");
        });
        let outcome = post_native(&agent(FIXTURE_TIMEOUT), &url, "{}", None);
        fixture.join().unwrap();
        assert_outcome(
            outcome,
            expected,
            (100..=599).contains(&status).then_some(status),
        );
    }
}

#[test]
fn native_post_does_not_follow_redirect_or_leak_auth() {
    for status in [301, 302, 303, 307, 308] {
        let destination = listener();
        let server = listener();
        let url = format!("http://{}/", server.local_addr().unwrap());
        let location = format!("http://{}/stolen", destination.local_addr().unwrap());
        let fixture = thread::spawn(move || {
            let mut stream = accept(&server);
            assert_eq!(
                request(&mut stream).header("authorization"),
                Some("Bearer secret")
            );
            response(
                &mut stream,
                status,
                b"redirect body",
                &format!("Location: {location}\r\nConnection: close\r\n"),
            );
        });
        let credentials =
            crate::remote_hosted::Creds::new("https://example.invalid".into(), "secret".into())
                .unwrap();
        let outcome = post_native(&agent(FIXTURE_TIMEOUT), &url, "{}", Some(&credentials));
        fixture.join().unwrap();
        assert_outcome(outcome, Delivery::Retry, Some(status));
        assert!(
            matches!(destination.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "redirect destination received a connection, potentially leaking credentials"
        );
    }
}

#[test]
fn native_post_response_limit_accepts_boundary_and_retries_overflow() {
    for (length, expected) in [
        (MAX_RESPONSE_BYTES as usize, Delivery::Accepted),
        (MAX_RESPONSE_BYTES as usize + 1, Delivery::Retry),
    ] {
        let server = listener();
        let url = format!("http://{}/", server.local_addr().unwrap());
        let fixture = thread::spawn(move || {
            let mut stream = accept(&server);
            request(&mut stream);
            // Write failures are expected when the client rejects the oversized body.
            let header =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&vec![b'x'; length]);
        });
        let outcome = post_native(&agent(FIXTURE_TIMEOUT), &url, "{}", None);
        fixture.join().unwrap();
        assert_outcome(outcome, expected, Some(200));
    }
}

#[test]
fn native_post_retries_incomplete_response_body() {
    let server = listener();
    let url = format!("http://{}/", server.local_addr().unwrap());
    let fixture = thread::spawn(move || {
        let mut stream = accept(&server);
        request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort")
            .unwrap();
    });
    let outcome = post_native(&agent(FIXTURE_TIMEOUT), &url, "{}", None);
    fixture.join().unwrap();
    assert_outcome(outcome, Delivery::Retry, Some(200));
}

#[test]
fn native_post_retries_transport_errors_without_curl_fallback() {
    let server = listener();
    let url = format!("http://{}/", server.local_addr().unwrap());
    let fixture = thread::spawn(move || {
        let mut stream = accept(&server);
        request(&mut stream);
        // Close without any HTTP response, a deterministic transport failure.
    });
    let outcome = post_native(&agent(FIXTURE_TIMEOUT), &url, "{}", None);
    fixture.join().unwrap();
    assert_outcome(outcome, Delivery::Retry, None);
}

#[test]
fn native_post_global_timeout_bounds_stalled_response() {
    let server = listener();
    let url = format!("http://{}/", server.local_addr().unwrap());
    let fixture = thread::spawn(move || {
        let mut stream = accept(&server);
        request(&mut stream);
        thread::sleep(Duration::from_millis(400));
    });
    let started = Instant::now();
    let outcome = post_native(&agent(Duration::from_millis(100)), &url, "{}", None);
    let elapsed = started.elapsed();
    fixture.join().unwrap();
    assert_outcome(outcome, Delivery::Retry, None);
    assert!(
        elapsed < Duration::from_secs(2),
        "request timeout took {elapsed:?}"
    );
}

#[test]
fn native_post_bounds_chunked_response_without_content_length() {
    let server = listener();
    let url = format!("http://{}/", server.local_addr().unwrap());
    let fixture = thread::spawn(move || {
        let mut stream = accept(&server);
        request(&mut stream);
        let length = MAX_RESPONSE_BYTES as usize + 1;
        let header = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{length:x}\r\n"
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&vec![b'x'; length]);
        let _ = stream.write_all(b"\r\n0\r\n\r\n");
    });
    let outcome = post_native(&agent(FIXTURE_TIMEOUT), &url, "{}", None);
    fixture.join().unwrap();
    assert_outcome(outcome, Delivery::Retry, Some(200));
}

#[test]
fn native_post_timeout_also_bounds_response_body_drain() {
    let server = listener();
    let url = format!("http://{}/", server.local_addr().unwrap());
    let fixture = thread::spawn(move || {
        let mut stream = accept(&server);
        request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
            .unwrap();
        thread::sleep(Duration::from_millis(400));
    });
    let started = Instant::now();
    let outcome = post_native(&agent(Duration::from_millis(100)), &url, "{}", None);
    let elapsed = started.elapsed();
    fixture.join().unwrap();
    assert_outcome(outcome, Delivery::Retry, Some(200));
    assert!(
        elapsed < Duration::from_secs(2),
        "body drain took {elapsed:?}"
    );
}

#[test]
fn production_agent_enforces_https_platform_roots_and_bounded_timeouts() {
    let client = telemetry_agent();
    let config = client.config();
    assert!(config.https_only());
    assert!(!config.http_status_as_error());
    assert_eq!(config.max_redirects(), 0);
    assert!(!config.max_redirects_will_error());
    assert_eq!(config.timeouts().global, Some(Duration::from_secs(20)));
    assert_eq!(config.timeouts().connect, Some(Duration::from_secs(5)));
    assert_eq!(config.timeouts().resolve, Some(Duration::from_secs(5)));
    assert!(matches!(
        config.tls_config().root_certs(),
        ureq::tls::RootCerts::PlatformVerifier
    ));
    assert_eq!(
        config.tls_config().provider(),
        ureq::tls::TlsProvider::Rustls
    );
    // HTTPS-only rejection must occur before connecting to this local listener.
    let server = listener();
    let url = format!("http://{}/", server.local_addr().unwrap());
    let outcome = post_native(&client, &url, "{}", None);
    assert_outcome(outcome, Delivery::Retry, None);
    assert!(
        matches!(server.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
}

// TEST ONLY: generated locally with openssl, never a production identity or trust root.
const SELF_SIGNED_TEST_CERT: &str = r#"-----BEGIN CERTIFICATE-----
MIIDJTCCAg2gAwIBAgIUXGeotmdMjlhn00esS8RbxXgtlw8wDQYJKoZIhvcNAQEL
BQAwFDESMBAGA1UEAwwJbG9jYWxob3N0MB4XDTI2MTExMTA0NTcyNloXDTM2MTEw
ODA0NTcyNlowFDESMBAGA1UEAwwJbG9jYWxob3N0MIIBIjANBgkqhkiG9w0BAQEF
AAOCAQ8AMIIBCgKCAQEA41QSe0KsaSiuEEuTg69O05Zz1HWztdtQR8VoPoZXTTDT
ZmShXPZBQfHwxRylHC5wb/TKwkMgLmGb3K/EjSIjxp4ToQvOaXgV2i5XmyBFcIeP
XPAY9KrmuzZgz3NQTBN2MzanDcEAP/gLS0uDNXNOWkwlf66nnwa9xsmUni9yuyQ/
eEN0WXy8ROyIskfEo9hKHjz1mrvmNq2aaR+QLRAhuglOOls0dCRi4ECvgApDgmmU
oLQSTJ53JhwKcXYoB27wNbdTJuQOr3fWAsJ3T2/wv2B+lLfefKO1S2LeVzkwFHlM
7mNTbRiOlGx6eR5Io9Cjs8m5ZMvWnSGBqK75ovu8eQIDAQABo28wbTAdBgNVHQ4E
FgQUhmM/3LO3CqcaZVm0ZxUfuQOpa8swHwYDVR0jBBgwFoAUhmM/3LO3CqcaZVm0
ZxUfuQOpa8swDwYDVR0TAQH/BAUwAwEB/zAaBgNVHREEEzARgglsb2NhbGhvc3SH
BH8AAAEwDQYJKoZIhvcNAQELBQADggEBAKhM0Kxp6dNqXdE2j76iL6YCvoeT6xww
v4QVAqRSylwtKKDDCo6vs/YpbrLDw7sCwVQ2Jvi8PMCBlUKXYlCn4FqimR+glWZC
DtYiWpN/oSey0DKgkJ/wM4KEOYDt85vWhX0ZnK3zvteVZLVnfgA2c8R5wctEPQuO
AmnCnf4SvUBzTM/DU9+mPWcbpcqrd3WoVHjXE+ePi4SuyTt6tYOLNYpIm3EktUFM
zHWxlJ6Ig8JcULkZOHIUPF6gAcXHt9G2zzf7zPlCZ88y7xW/sPY0+AZGQdx4/0ra
+V9UlTb4jfm530CfmzgUF9qlERDBgnhehfn5OW/NscnZW/HpijPneKU=
-----END CERTIFICATE-----
"#;
// TEST ONLY: public fixture private key, intentionally committed alongside its certificate.
const SELF_SIGNED_TEST_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQDjVBJ7QqxpKK4Q
S5ODr07TlnPUdbO121BHxWg+hldNMNNmZKFc9kFB8fDFHKUcLnBv9MrCQyAuYZvc
r8SNIiPGnhOhC85peBXaLlebIEVwh49c8Bj0qua7NmDPc1BME3YzNqcNwQA/+AtL
S4M1c05aTCV/rqefBr3GyZSeL3K7JD94Q3RZfLxE7IiyR8Sj2EoePPWau+Y2rZpp
H5AtECG6CU46WzR0JGLgQK+ACkOCaZSgtBJMnncmHApxdigHbvA1t1Mm5A6vd9YC
wndPb/C/YH6Ut958o7VLYt5XOTAUeUzuY1NtGI6UbHp5Hkij0KOzyblky9adIYGo
rvmi+7x5AgMBAAECgf80Hyl5EFUAtSnqm7KW3taBthQrAnY/nBcHgkApFdH3nEgA
tq1bDFIXyBOMQsmq3sQWnjLxTmmXFEERb03Kss3BHGH/M/pbGN/FNGQPev/ekcia
1DstuV0ABo6iuNJZenhImA0HsSHrP697z32l/yDqLqbF0gE4MHylAJwyuwCX3l/k
RVMBkB5D0rhvAlEbN9ITqSNrALjWfS2vbtodRrmmnpKKDjweL7/PQpT8Ej8srL8Z
rSnTdwigCPvVOJ6IB/3PeqZCuFk+eJAcy2j0V0wgghPNw51C5Xierh4VYha1T6qe
FB3ZY+X+6pKhewqGNLHozByu8IO2ZnxqzSdZyDECgYEA+43CRaIGWiupnn/ZqLdJ
NwONRcUp922e9oaKuU86FuACmP6B+e66z1Ano3CQ8Yehl333jaJtKu3Ouja9To0V
e6s5rgGRrX5iAAm5AtFQrZl2b0JfKu0tf7StVnEl3sK4sW3/B+J9s6Y5/B1rz3y0
w1hLL8T9E8t22v8uiQEpp7ECgYEA51iyjYnFIBqbh/Ul6PP2HtMdpzxN9Cc7Rg4t
P6VHcC7lGfHYz9gTyLIXfrn9zTVNM3dXceJovSVjWL/JVexWFatoZvZh3oFj7+7p
HRzXriSBCiUqQFbLaU9U7wiTEp1YespS6zRH/6SgodKz042dm3lbZaYf/H11IvrN
x5YvW0kCgYEAgGxkzAvl1k8IL2TyCBJajH2BuCbqP+l1CEKzu9tYuqjuapE0Zhac
xI4VTnl/Ze9qbd7fbS2JMitqKOMdTmb0mshVkCI47yvgeEh+bZPS7m20aySlVVL0
sYfaHpFxVgJvLBYN50aYCglI/Q9AJRNzbW88I00cLujm6vLwHP28+GECgYBE8dcM
fVFk9ukBBJsOqJ3ho9MDguJMb+CnSRRNEmmO0jrul3egwWlr9z8vQiiUHl1qiBdk
VjAWpmzMByPuWmdxP8r2XFjgYbkxyQZp6a2RNUnLhzXnRLaMJNtP/UMvWfsDZQZ/
F+6ZDivc7hkvY9XjfrAxQ+fmbFEbYpku8OAeGQKBgQDqQoCr+J4ATQhpOQhljheS
GrhEXKHvO8ceqr7SNv5Zyfzlxfj/uyX25iSSnCSO3jwepu/N6X8nHwqPN8NWbJQN
IX5J3Cjy/Uhlo1LzgVUFh7Sxkphtx8rXnPjK2OwLTA7piCxavQGZOBneb7BKBbxC
nHtkVR3MA/yyBlqI+ANeGQ==
-----END PRIVATE KEY-----
"#;

#[test]
fn production_agent_rejects_self_signed_tls_before_sending_auth() {
    use rustls::pki_types::pem::PemObject;
    use std::sync::Arc;

    let certificate =
        rustls::pki_types::CertificateDer::from_pem_slice(SELF_SIGNED_TEST_CERT.as_bytes())
            .unwrap();
    let key =
        rustls::pki_types::PrivateKeyDer::from_pem_slice(SELF_SIGNED_TEST_KEY.as_bytes()).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![certificate], key)
    .unwrap();
    let server = listener();
    let url = format!(
        "https://{}/must-not-receive-auth",
        server.local_addr().unwrap()
    );
    let fixture = thread::spawn(move || {
        let stream = accept(&server);
        let connection = rustls::ServerConnection::new(Arc::new(config)).unwrap();
        let mut tls = rustls::StreamOwned::new(connection, stream);
        let mut application_data = [0u8; 4096];
        // A certificate rejection produces an alert, I/O error, or EOF, never HTTP.
        match tls.read(&mut application_data) {
            Ok(0) | Err(_) => {}
            Ok(count) => panic!("untrusted TLS server received {count} bytes of application data"),
        }
    });
    let credentials = crate::remote_hosted::Creds::new(
        "https://example.invalid".into(),
        "must-not-leak-to-untrusted-tls".into(),
    )
    .unwrap();
    let outcome = post_native(
        &telemetry_agent(),
        &url,
        "{\"private\":true}",
        Some(&credentials),
    );
    fixture.join().unwrap();
    assert_outcome(outcome, Delivery::Retry, None);
}
