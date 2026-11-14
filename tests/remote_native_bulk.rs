//! Real rsync sender/receiver processes connected by the native TLS exec stream.
//! The loopback supervisor uses only a cfg(test) current-user fixture, never the
//! production root supervisor or the account's hosted credentials.
#![cfg(target_os = "linux")]

#[allow(dead_code)]
#[path = "../src/remote_native.rs"]
mod remote_native;

use std::fs;
use std::path::PathBuf;
use std::process::{Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::process::Command;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn quote(s: &str) -> String {
    // rsync -e parses its own words, not POSIX shell escapes.
    format!("'{}'", s.replace('\'', "''"))
}

struct Fixture {
    root: PathBuf,
    endpoint: remote_native::Endpoint,
    _runtime: tokio::runtime::Runtime,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "jr-native-bulk-{}-{} with 'quotes'",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let identity = ssh_key::PrivateKey::random(
            &mut ssh_key::rand_core::OsRng,
            ssh_key::Algorithm::Ed25519,
        )
        .unwrap();
        let key_path = root.join("identity with 'quotes'");
        identity
            .write_openssh_file(&key_path, ssh_key::LineEnding::LF)
            .unwrap();
        let certified = rcgen::generate_simple_self_signed(vec!["justrust-native".into()]).unwrap();
        let cert = certified.cert.der().to_vec();
        let key = certified.signing_key.serialize_der();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let address = listener.local_addr().unwrap();
        let endpoint = remote_native::Endpoint {
            address: address.ip().to_string(),
            port: address.port(),
            server_name: "justrust-native".into(),
            certificate_sha256: remote_native::certificate_pin(&cert),
            key_path,
            resume_path: root.join("unused resume"),
        };
        let home = root.clone();
        let keys = vec![identity.public_key().to_openssh().unwrap()];
        runtime.spawn(async move {
            remote_native::serve_test(listener, cert, key, keys, home)
                .await
                .unwrap();
        });
        Self {
            root,
            endpoint,
            _runtime: runtime,
        }
    }

    fn shell(&self) -> String {
        format!(
            "{} __native rsync {}",
            quote(env!("CARGO_BIN_EXE_justrust")),
            quote(&serde_json::to_string(&self.endpoint).unwrap())
        )
    }

    fn rsync(&self, source: &std::path::Path, destination: &std::path::Path) -> Command {
        let mut command = Command::new("rsync");
        command
            .args(["-az", "--secluded-args", "-e", &self.shell()])
            .arg(source)
            .arg(format!("justrust-native:{}", destination.display()))
            .stdin(Stdio::null())
            .kill_on_drop(true);
        command
    }

    fn output(&self, command: &mut Command) -> Output {
        self._runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
                .await
                .expect("native rsync transfer stalled")
                .unwrap()
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "status {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn native_bulk_shell_preserves_stdout_stderr_and_exit_status() {
    let fixture = Fixture::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_justrust"));
    command
        .args([
            "__native",
            "client",
            &serde_json::to_string(&fixture.endpoint).unwrap(),
            "printf '\\000\\377native output'; printf 'native error' >&2; exit 37",
        ])
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let output = fixture.output(&mut command);
    assert_eq!(output.status.code(), Some(37));
    assert_eq!(output.stdout, b"\x00\xffnative output");
    assert_eq!(output.stderr, b"native error");
}

#[test]
fn native_bulk_early_failure_drains_stderr_with_unconsumed_input() {
    let fixture = Fixture::new();
    fixture._runtime.block_on(async {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        // More than the pipe and TLS buffers, while the child explicitly closes
        // stdin before writing its diagnostic and returning a nonzero status.
        let input = vec![0xa5; 4 * 1024 * 1024];
        let code = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            remote_native::exec_endpoint_io(
                &fixture.endpoint,
                "exec 0<&-; printf 'early failure\\n' >&2; exit 37",
                &mut input.as_slice(),
                &mut stdout,
                &mut stderr,
            ),
        )
        .await
        .expect("early native failure stalled")
        .unwrap();
        assert_eq!(code, 37);
        assert!(stdout.is_empty());
        assert_eq!(stderr, b"early failure\n");
    });
}

#[test]
fn native_bulk_transfer_preserves_quoted_names_deletes_stale_and_protects_outputs() {
    let fixture = Fixture::new();
    let source = fixture.root.join("source ' tree");
    let destination = fixture.root.join("receiver ' tree");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::create_dir(destination.join("target")).unwrap();
    fs::create_dir(destination.join(".justrust")).unwrap();
    fs::write(destination.join("target/warm rlib"), b"keep target").unwrap();
    fs::write(destination.join(".justrust/checkpoint"), b"keep state").unwrap();
    fs::write(destination.join("stale.rs"), b"delete me").unwrap();
    let names = [
        "normal.rs",
        "single'quote.rs",
        "double\"quote.rs",
        "space and $dollar;$(touch injected).rs",
        "back\\slash\nnewline.rs",
        "-leading-dash.rs",
    ];
    for name in names {
        fs::write(source.join(name), format!("contents: {name}\0\u{ff}")).unwrap();
    }
    let mut command = fixture.rsync(&source.join(""), &destination.join(""));
    command.args([
        "--delete",
        "--checksum",
        "--no-times",
        "--out-format=%i %n",
        "--filter=P target/",
        "--filter=P .justrust/",
        "--exclude=target/",
        "--exclude=.justrust/",
    ]);
    let first = fixture.output(&mut command);
    success(&first);
    assert!(String::from_utf8_lossy(&first.stdout).contains("deleting"));
    for name in names {
        assert_eq!(
            fs::read(source.join(name)).unwrap(),
            fs::read(destination.join(name)).unwrap()
        );
    }
    assert!(!destination.join("stale.rs").exists());
    assert!(!fixture.root.join("injected").exists());
    assert_eq!(
        fs::read(destination.join("target/warm rlib")).unwrap(),
        b"keep target"
    );
    assert_eq!(
        fs::read(destination.join(".justrust/checkpoint")).unwrap(),
        b"keep state"
    );
    fs::remove_file(source.join("single'quote.rs")).unwrap();
    fs::write(source.join("normal.rs"), b"changed").unwrap();
    success(&fixture.output(&mut command));
    assert!(!destination.join("single'quote.rs").exists());
    assert_eq!(fs::read(destination.join("normal.rs")).unwrap(), b"changed");
}

#[test]
fn native_bulk_binary_candidate_upload_is_byte_exact_and_failure_is_not_success() {
    let fixture = Fixture::new();
    let source = fixture.root.join("candidate ' executable");
    let destination = fixture.root.join("private ' upload");
    let mut state = 0x1234_5678_u32;
    let bytes: Vec<u8> = (0..(512 * 1024))
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    fs::write(&source, &bytes).unwrap();
    success(&fixture.output(&mut fixture.rsync(&source, &destination)));
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    let missing_parent = fixture.root.join("absent parent/candidate");
    // Exercise both pipe-read scheduling orders repeatedly. A fatal rsync
    // stdout message can make the sender kill its transport before stderr is
    // forwarded, so a nonzero status alone is not sufficient coverage.
    for attempt in 0..32 {
        let failed = fixture.output(&mut fixture.rsync(&source, &missing_parent));
        assert!(!failed.status.success());
        assert!(
            !failed.stderr.is_empty(),
            "failed upload lost diagnostics on attempt {attempt}: status {}\nstdout: {:?}\nstderr: {:?}",
            failed.status,
            String::from_utf8_lossy(&failed.stdout),
            String::from_utf8_lossy(&failed.stderr)
        );
        assert!(
            String::from_utf8_lossy(&failed.stderr).contains("absent parent"),
            "failed upload did not preserve receiver diagnostic on attempt {attempt}: {:?}",
            String::from_utf8_lossy(&failed.stderr)
        );
        assert!(!missing_parent.exists());
    }
}
