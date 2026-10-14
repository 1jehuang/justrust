//! Explicit feedback is synchronous, never queued, and sends only the message.
use anyhow::{Context, bail};
use std::io::Write;
use std::process::{Command, Stdio};

const MAX_BYTES: usize = 4096;

fn validate(message: &str) -> anyhow::Result<()> {
    if message.trim().is_empty() {
        bail!("feedback message must not be empty");
    }
    if message.len() > MAX_BYTES {
        bail!("feedback message exceeds {MAX_BYTES} UTF-8 bytes");
    }
    if message
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        bail!("feedback message contains unsupported control characters");
    }
    Ok(())
}

pub fn command(message: &str) -> anyhow::Result<()> {
    validate(message)?;
    if let Some(why) = crate::telemetry::disabled_reason() {
        bail!("feedback not sent: telemetry is off ({why})");
    }
    let url = format!("{}/feedback", crate::remote_hosted::build_base());
    send(&url, message)?;
    println!("Feedback delivered to the maintainers. Thank you.");
    Ok(())
}

fn send(url: &str, message: &str) -> anyhow::Result<()> {
    let loopback = url.strip_prefix("http://").is_some_and(|rest| {
        let authority = rest.split('/').next().unwrap_or("");
        ["127.0.0.1:", "localhost:", "[::1]:"].iter().any(|prefix| {
            authority
                .strip_prefix(prefix)
                .is_some_and(|port| port.parse::<u16>().is_ok())
        })
    });
    if !url.starts_with("https://") && !loopback {
        bail!("feedback not sent: endpoint must use HTTPS (HTTP allowed only for loopback tests)");
    }
    let body = serde_json::to_vec(&serde_json::json!({ "message": message }))?;
    let mut child = Command::new("curl")
        .args([
            "-q", // First: ignore curlrc (which could attach credentials or change delivery).
            "-sS",
            "--proto",
            if loopback { "=https,http" } else { "=https" },
            "--max-redirs",
            "0",
            "--connect-timeout",
            "5",
            "--max-time",
            "20",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
            "-o",
            crate::sys::DEV_NULL,
            "-w",
            "%{http_code}",
        ])
        .arg(url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("feedback not sent: could not start curl")?;
    let write = child
        .stdin
        .take()
        .context("feedback stdin unavailable")?
        .write_all(&body);
    let out = child
        .wait_with_output()
        .context("feedback delivery failed")?;
    write.context("feedback delivery failed while writing message")?;
    if !out.status.success() {
        bail!(
            "feedback delivery failed (curl exit {}): receipt is unconfirmed; not queued",
            out.status
        );
    }
    let status: u16 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    // This endpoint only acknowledges after the durable insert completes.
    if status != 201 {
        bail!("feedback not accepted (HTTP {status}); not queued");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_nonlocal_plaintext_before_curl() {
        for url in [
            "http://example.com/feedback",
            "http://127.0.0.1:80@example.com/feedback",
            "http://localhost:bad/feedback",
        ] {
            assert!(send(url, "test").unwrap_err().to_string().contains("HTTPS"));
        }
    }
    #[test]
    fn bounded_message() {
        assert!(validate(" \n\t").is_err());
        assert!(validate(&"a".repeat(MAX_BYTES)).is_ok());
        assert!(validate(&"é".repeat(MAX_BYTES / 2 + 1)).is_err());
        assert!(validate("hi\0").is_err());
        assert!(validate("hello\nworld\t!").is_ok());
    }
}
