//! Opt-in persistent hosted admission transport. Local socket only.
//! Every request still reaches authoritative billing. No entitlement cache.
use crate::remote_hosted::{Creds, Resp};
use crate::remote_proto::{Msg, read_frame, write_frame};
use anyhow::{Result, bail};
use serde_json::Value;
use std::io::{BufReader, BufWriter};
use std::sync::OnceLock;
use std::time::Duration;

const LIMIT: u64 = 1 << 20;
const CUSTOM_ENV: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "CURL_CA_BUNDLE",
    "CURL_SSL_BACKEND",
    "SSLKEYLOGFILE",
];

pub fn enabled() -> bool {
    std::env::var("JUSTRUST_HOSTED_HTTP_POOL")
        .is_ok_and(|v| matches!(v.as_str(), "1" | "on" | "true"))
}

pub fn custom_transport() -> bool {
    custom_transport_with(|name| std::env::var_os(name).is_some())
}

fn custom_transport_with(mut configured: impl FnMut(&str) -> bool) -> bool {
    CUSTOM_ENV.iter().any(|name| configured(name))
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let config = ureq::Agent::config_builder()
            .tls_config(tls)
            .proxy(None)
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(30)))
            .max_idle_connections(8)
            .max_idle_connections_per_host(3)
            .max_idle_age(Duration::from_secs(60))
            .build();
        ureq::Agent::new_with_config(config)
    })
}

pub fn request(c: &Creds, body: &Value) -> Result<Resp> {
    // A daemon may have inherited a different proxy/certificate environment.
    // Fail back to the caller's curl rather than silently change its policy.
    if custom_transport() {
        bail!("custom HTTP transport requires curl");
    }
    request_native(c, body)
}

fn request_native(c: &Creds, body: &Value) -> Result<Resp> {
    let valid = Creds::from_local_auth(c.local_auth())?;
    let id = body["run_id"].as_str().unwrap_or("");
    let data = body.to_string();
    if id.is_empty() || id.len() > 128 || data.len() > 4096 {
        bail!("invalid hosted admission request");
    }
    let mut response = agent()
        .post(valid.url("build/runs"))
        .header("Authorization", valid.bearer())
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .send(data.as_bytes())
        .map_err(|_| anyhow::anyhow!("pooled hosted admission transport failed"))?;
    let code = response.status().as_u16();
    let bytes = response
        .body_mut()
        .with_config()
        .limit(LIMIT)
        .read_to_vec()
        .map_err(|_| anyhow::anyhow!("pooled hosted admission body invalid or too large"))?;
    Ok(Resp {
        code,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    })
}

pub fn via_daemon(c: &Creds, body: &Value) -> Result<Resp> {
    let socket = crate::remote_daemon::connect()?;
    socket.set_read_timeout(Some(Duration::from_secs(35)))?;
    socket.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut output = BufWriter::new(socket.try_clone()?);
    let mut input = BufReader::new(socket);
    write_frame(
        &mut output,
        &Msg::HostedAdmission {
            auth: c.local_auth(),
            body: body.clone(),
        },
        &[],
    )?;
    match read_frame(&mut input)? {
        Some((Msg::HostedAdmissionResponse { code, body }, _)) => Ok(Resp { code, body }),
        _ => bail!("local hosted admission transport unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Write};
    use std::net::TcpListener;

    #[test]
    fn custom_proxy_and_certificate_settings_always_choose_curl() {
        for name in CUSTOM_ENV {
            assert!(custom_transport_with(|configured| configured == *name));
        }
        assert!(!custom_transport_with(|name| name == "JCODE_API_KEY"));
        assert!(!custom_transport_with(|_| false));
        if custom_transport() {
            let names: Vec<_> = CUSTOM_ENV
                .iter()
                .filter(|name| std::env::var_os(name).is_some())
                .collect();
            eprintln!("configured transport variable names, no values: {names:?}");
            let c = Creds::new("http://127.0.0.1:1/v1".into(), "canary".into()).unwrap();
            assert!(
                request(&c, &serde_json::json!({"run_id":"policy"}))
                    .unwrap_err()
                    .to_string()
                    .contains("requires curl")
            );
        }
    }

    fn server(
        responses: Vec<(u16, Vec<u8>, bool)>,
    ) -> (Creds, std::thread::JoinHandle<Vec<(String, Value)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let c = Creds::new(base, "key-one".into()).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut requests = Vec::new();
            for (code, body, redirect) in responses {
                let mut headers = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                    headers.push_str(&line);
                }
                let mut input = vec![0u8; length];
                reader.read_exact(&mut input).unwrap();
                requests.push((headers, serde_json::from_slice(&input).unwrap()));
                let crlf = String::from_utf8(vec![13, 10]).unwrap();
                let location = if redirect {
                    format!("Location: http://127.0.0.1:1/leak{crlf}")
                } else {
                    String::new()
                };
                let header = format!(
                    "HTTP/1.1 {code} Response{crlf}Content-Length: {}{crlf}Content-Type: application/json{crlf}{location}{crlf}",
                    body.len()
                );
                if stream.write_all(header.as_bytes()).is_err() || stream.write_all(&body).is_err()
                {
                    break;
                }
                stream.flush().unwrap();
            }
            requests
        });
        (c, handle)
    }

    #[test]
    fn reuses_connection_but_sends_rotated_credentials_and_reads_every_charge() {
        let body1 = serde_json::json!({"builds":{"used":1}})
            .to_string()
            .into_bytes();
        let body2 = serde_json::json!({"builds":{"used":2}})
            .to_string()
            .into_bytes();
        let (c, handle) = server(vec![(200, body1, false), (200, body2, false)]);
        assert_eq!(
            request_native(&c, &serde_json::json!({"run_id":"r1"}))
                .unwrap()
                .body["builds"]["used"],
            1
        );
        let mut auth = c.local_auth();
        auth.key = "key-two".into();
        let c2 = Creds::from_local_auth(auth).unwrap();
        assert_eq!(
            request_native(&c2, &serde_json::json!({"run_id":"r2"}))
                .unwrap()
                .body["builds"]["used"],
            2
        );
        let requests = handle.join().unwrap();
        assert_eq!(
            requests.len(),
            2,
            "both requests arrive on the single accepted connection"
        );
        assert!(
            requests[0]
                .0
                .to_ascii_lowercase()
                .contains("authorization: bearer key-one")
        );
        assert!(
            requests[1]
                .0
                .to_ascii_lowercase()
                .contains("authorization: bearer key-two")
        );
        assert_eq!(requests[0].1["run_id"], "r1");
        assert_eq!(requests[1].1["run_id"], "r2");
    }

    #[test]
    fn does_not_follow_redirects_or_hide_quota_denials() {
        for code in [302, 401, 402, 429, 503] {
            let response = serde_json::json!({"error":{"code":"usage_cap_exceeded"}})
                .to_string()
                .into_bytes();
            let (c, handle) = server(vec![(code, response, code == 302)]);
            let r = request_native(&c, &serde_json::json!({"run_id":"denied"})).unwrap();
            assert_eq!(r.code, code);
            assert_eq!(r.body["error"]["code"], "usage_cap_exceeded");
            assert_eq!(handle.join().unwrap().len(), 1);
        }
    }

    #[test]
    fn bounds_response_and_rejects_invalid_auth_before_network() {
        let (c, handle) = server(vec![(200, vec![b'x'; LIMIT as usize + 1], false)]);
        let error = request_native(&c, &serde_json::json!({"run_id":"oversized"}))
            .unwrap_err()
            .to_string();
        assert!(!error.contains("key-one"));
        assert!(error.contains("too large"));
        handle.join().unwrap();
        for base in [
            "http://public.example/v1",
            "https://user:pass@example.com/v1",
            "https://example.com/v1?key=secret",
        ] {
            let auth = crate::remote_proto::HostedAuth {
                base: base.into(),
                build_base: base.into(),
                key: "key-one".into(),
            };
            assert!(Creds::from_local_auth(auth).is_err());
        }
    }

    #[test]
    fn rejects_unbounded_admission_payloads_without_network() {
        let c = Creds::new("http://127.0.0.1:1/v1".into(), "key-one".into()).unwrap();
        for body in [
            serde_json::json!({}),
            serde_json::json!({"run_id":""}),
            serde_json::json!({"run_id":"x".repeat(129)}),
            serde_json::json!({"run_id":"ok", "cmd":"x".repeat(4096)}),
        ] {
            assert!(
                request_native(&c, &body)
                    .unwrap_err()
                    .to_string()
                    .contains("invalid hosted admission")
            );
        }
    }
}
