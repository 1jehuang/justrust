//! Native hosted transport. TLS 1.3 authenticates the pinned guest certificate;
//! an SSH Ed25519 signature authenticates each command against the TLS exporter.
//! The root supervisor never lends its slot credential to the build user.
use anyhow::{Context, Result, bail, ensure};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

const LABEL: &[u8] = b"EXPORTER-justrust-native-v1";
const NAMESPACE: &str = "justrust-native-v1";
const MAX_FRAME: usize = 65536;
const MAX_COMMAND: usize = 32768;
const AUTH_TIMEOUT: Duration = Duration::from_secs(15);
const IO_TIMEOUT: Duration = Duration::from_secs(120);
const EXEC_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);
const CHALLENGE: u8 = 1;
const AUTH: u8 = 2;
const STDIN: u8 = 3;
const EOF: u8 = 4;
const STDOUT: u8 = 5;
const STDERR: u8 = 6;
const EXIT: u8 = 7;
const ERROR: u8 = 8;
const READY: u8 = 9;
const AUTH_REJECTED: u8 = 10;
const ACCEPT_AUTH: u8 = 11;
const ACCEPT_RECEIPT: u8 = 12;
const AUTH_REJECTION_BODY: &[u8] = b"native identity rejected before execution";

#[derive(Debug)]
struct NativeAuthenticationRejected;
impl std::fmt::Display for NativeAuthenticationRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("native authentication explicitly rejected before execution")
    }
}
impl std::error::Error for NativeAuthenticationRejected {}

/// Native routing and identity. Only key/resume paths are client-private data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub address: String,
    pub port: u16,
    pub server_name: String,
    pub certificate_sha256: String,
    pub key_path: PathBuf,
    pub resume_path: PathBuf,
}

#[allow(dead_code)] // Available to alternate Target adapters without exposing TLS internals.
pub fn command(endpoint: &Endpoint, script: &str) -> std::process::Command {
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("justrust"));
    let mut cmd = std::process::Command::new(executable);
    cmd.arg("__native")
        .arg("client")
        .arg(serde_json::to_string(endpoint).expect("endpoint serialization"))
        .arg(script);
    cmd
}

#[derive(Debug)]
struct PinnedCertificate([u8; 32]);
impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if !intermediates.is_empty() || Sha256::digest(cert.as_ref()).as_slice() != self.0 {
            return Err(rustls::Error::General(
                "native server certificate pin mismatch".into(),
            ));
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General(
            "native transport requires TLS 1.3".into(),
        ))
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub fn certificate_pin(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn parse_pin(pin: &str) -> Result<[u8; 32]> {
    ensure!(
        pin.len() == 64 && pin.is_ascii(),
        "expected 64 hex digits for certificate DER SHA256 pin"
    );
    let mut out = [0; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte =
            u8::from_str_radix(&pin[i * 2..i * 2 + 2], 16).context("invalid certificate pin")?;
    }
    Ok(out)
}
fn client_config(pin: &str) -> Result<Arc<rustls::ClientConfig>> {
    let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(PinnedCertificate(parse_pin(pin)?)))
    .with_no_client_auth();
    config.alpn_protocols = vec![NAMESPACE.as_bytes().to_vec()];
    config.enable_early_data = false;
    config.resumption = rustls::client::Resumption::disabled();
    Ok(Arc::new(config))
}
fn server_config(cert: Vec<u8>, key: Vec<u8>) -> Result<Arc<rustls::ServerConfig>> {
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(cert)],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
    )?;
    config.alpn_protocols = vec![NAMESPACE.as_bytes().to_vec()];
    config.max_early_data_size = 0;
    config.send_tls13_tickets = 0;
    Ok(Arc::new(config))
}
async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, kind: u8, body: &[u8]) -> Result<()> {
    ensure!(body.len() <= MAX_FRAME, "native frame too large");
    tokio::time::timeout(IO_TIMEOUT, async {
        w.write_u8(kind).await?;
        w.write_u32(body.len() as u32).await?;
        w.write_all(body).await?;
        w.flush().await
    })
    .await
    .context("native write timeout")??;
    Ok(())
}
async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<(u8, Vec<u8>)> {
    let kind = r.read_u8().await?;
    let len = r.read_u32().await? as usize;
    ensure!(len <= MAX_FRAME, "native frame too large");
    let mut body = vec![0; len];
    r.read_exact(&mut body).await?;
    Ok((kind, body))
}
#[derive(Serialize, Deserialize)]
struct Proof {
    key: String,
    signature: String,
    command: String,
    #[serde(default)]
    enroll: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    acceptance: Option<AcceptanceRequest>,
}
#[derive(Clone, Serialize, Deserialize)]
struct AcceptanceRequest {
    challenge: String,
    certificate_sha256: String,
}
fn transcript(exporter: &[u8; 32], challenge: &[u8], command: &str) -> Vec<u8> {
    let mut data = NAMESPACE.as_bytes().to_vec();
    data.extend_from_slice(exporter);
    data.extend_from_slice(challenge);
    data.extend_from_slice(&(command.len() as u32).to_be_bytes());
    data.extend_from_slice(command.as_bytes());
    data
}
#[cfg(all(test, unix))]
fn prove(
    key: &ssh_key::PrivateKey,
    exporter: &[u8; 32],
    challenge: &[u8],
    command: &str,
) -> Result<Proof> {
    prove_enroll(key, exporter, challenge, command, None)
}
fn proof_data(
    exporter: &[u8; 32],
    challenge: &[u8],
    command: &str,
    enroll: Option<&str>,
    acceptance: Option<&AcceptanceRequest>,
) -> Vec<u8> {
    let mut data = transcript(exporter, challenge, command);
    let enroll = enroll.unwrap_or("").as_bytes();
    data.extend_from_slice(&(enroll.len() as u32).to_be_bytes());
    data.extend_from_slice(enroll);
    if let Some(request) = acceptance {
        // Preserve ordinary exec's signed bytes. Acceptance has its own domain
        // and length-delimited fields, so its proof cannot authorize a command.
        data.extend_from_slice(b"\0justrust-native-accept-v1");
        for value in [&request.challenge, &request.certificate_sha256] {
            data.extend_from_slice(&(value.len() as u32).to_be_bytes());
            data.extend_from_slice(value.as_bytes());
        }
    }
    data
}
#[cfg(all(test, unix))]
fn prove_enroll(
    key: &ssh_key::PrivateKey,
    exporter: &[u8; 32],
    challenge: &[u8],
    command: &str,
    enroll: Option<String>,
) -> Result<Proof> {
    prove_request(key, exporter, challenge, command, enroll, None)
}
fn prove_request(
    key: &ssh_key::PrivateKey,
    exporter: &[u8; 32],
    challenge: &[u8],
    command: &str,
    enroll: Option<String>,
    acceptance: Option<AcceptanceRequest>,
) -> Result<Proof> {
    ensure!(
        key.algorithm() == ssh_key::Algorithm::Ed25519,
        "native client requires Ed25519 SSH identity"
    );
    ensure!(command.len() <= MAX_COMMAND, "native command too large");
    let signature = key.sign(
        NAMESPACE,
        ssh_key::HashAlg::Sha256,
        &proof_data(
            exporter,
            challenge,
            command,
            enroll.as_deref(),
            acceptance.as_ref(),
        ),
    )?;
    Ok(Proof {
        key: key.public_key().to_openssh()?,
        signature: signature.to_pem(ssh_key::LineEnding::LF)?,
        command: command.into(),
        enroll,
        acceptance,
    })
}
fn verify(proof: &Proof, exporter: &[u8; 32], challenge: &[u8], allowed: &[String]) -> Result<()> {
    ensure!(
        proof.command.len() <= MAX_COMMAND && !proof.command.contains('\0'),
        "invalid native command"
    );
    if let Some(request) = &proof.acceptance {
        ensure!(
            proof.command.is_empty(),
            "acceptance cannot execute a command"
        );
        ensure!(
            request.challenge.len() == 64
                && request
                    .challenge
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid acceptance challenge"
        );
        parse_pin(&request.certificate_sha256)?;
    }
    let key = ssh_key::PublicKey::from_openssh(&proof.key)?;
    ensure!(
        key.algorithm() == ssh_key::Algorithm::Ed25519,
        "only Ed25519 identities are supported"
    );
    ensure!(
        allowed
            .iter()
            .filter_map(|s| ssh_key::PublicKey::from_openssh(s).ok())
            .any(|k| k.key_data() == key.key_data()),
        "unauthorized native identity"
    );
    key.verify(
        NAMESPACE,
        &proof_data(
            exporter,
            challenge,
            &proof.command,
            proof.enroll.as_deref(),
            proof.acceptance.as_ref(),
        ),
        &ssh_key::SshSig::from_pem(&proof.signature)?,
    )?;
    Ok(())
}

/// Configuration is held only by the privileged supervisor, never in a command's environment.
#[derive(Clone)]
pub struct ServeConfig {
    pub bind: String,
    pub cert: PathBuf,
    pub key: PathBuf,
    pub slot_token_file: PathBuf,
    pub slot_keys_url: String,
}
#[derive(Clone)]
struct ExecutionUser {
    #[cfg(unix)]
    uid: u32,
    #[cfg(unix)]
    gid: u32,
    #[cfg(unix)]
    home: String,
}
#[cfg(unix)]
fn ubuntu_user() -> Result<ExecutionUser> {
    use std::ffi::CStr;
    // Resolve once before accepting requests. Never accept a caller-selected uid.
    let entry = unsafe { libc::getpwnam(c"ubuntu".as_ptr()) };
    ensure!(!entry.is_null(), "ubuntu execution user is missing");
    let entry = unsafe { &*entry };
    ensure!(entry.pw_uid != 0, "ubuntu execution user must not be root");
    Ok(ExecutionUser {
        uid: entry.pw_uid,
        gid: entry.pw_gid,
        home: unsafe { CStr::from_ptr(entry.pw_dir) }
            .to_string_lossy()
            .into_owned(),
    })
}
#[cfg(not(unix))]
fn ubuntu_user() -> Result<ExecutionUser> {
    bail!("native guest serving requires Unix")
}

fn read_root_secret(path: &Path) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        ensure!(
            unsafe { libc::geteuid() } == 0,
            "native serve must run as root supervisor"
        );
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let meta = file.metadata()?;
        ensure!(
            meta.is_file() && meta.uid() == 0 && meta.mode() & 0o077 == 0,
            "native secret must be a root-owned 0600 file"
        );
        ensure!(meta.len() <= 65536, "native secret is oversized");
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut bytes)?;
        Ok(bytes)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        bail!("native guest serving requires Unix")
    }
}

#[derive(Clone)]
enum KeySource {
    Slot {
        url: String,
        token: String,
    },
    #[cfg(all(test, unix))]
    Fixed(Vec<String>),
}
#[derive(Deserialize)]
struct Authorization {
    keys: Vec<String>,
    account_id: String,
    lease_id: String,
}
#[derive(Default)]
struct ResumeKeys(std::collections::HashMap<String, (String, String, std::time::Instant)>);
impl ResumeKeys {
    fn authorize(
        &mut self,
        proof: &Proof,
        exporter: &[u8; 32],
        challenge: &[u8],
        auth: &Authorization,
    ) -> Result<()> {
        let now = std::time::Instant::now();
        self.0.retain(|_, (account, lease, expiry)| {
            *expiry > now && account == &auth.account_id && lease == &auth.lease_id
        });
        if verify(proof, exporter, challenge, &auth.keys).is_ok() {
            if let Some(enroll) = &proof.enroll {
                ensure!(enroll.len() <= 1024, "oversized resume identity");
                let key = ssh_key::PublicKey::from_openssh(enroll)?;
                ensure!(
                    key.algorithm() == ssh_key::Algorithm::Ed25519,
                    "resume identity must use Ed25519"
                );
                let canonical = key.to_openssh()?;
                ensure!(
                    self.0.len() < 4096 || self.0.contains_key(&canonical),
                    "too many resume identities"
                );
                self.0.insert(
                    canonical,
                    (
                        auth.account_id.clone(),
                        auth.lease_id.clone(),
                        now + EXEC_TIMEOUT,
                    ),
                );
            }
            return Ok(());
        }
        ensure!(
            proof.enroll.is_none(),
            "resume identities cannot enroll more identities"
        );
        let key = ssh_key::PublicKey::from_openssh(&proof.key)?.to_openssh()?;
        ensure!(
            self.0.contains_key(&key),
            "unauthorized native resume identity"
        );
        verify(proof, exporter, challenge, &[key])
    }
}
fn slot_api_permit() -> Result<tokio::sync::OwnedSemaphorePermit> {
    static REQUESTS: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    // Retain in blocking jobs: dropping async authentication cannot cancel DNS.
    REQUESTS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(64)))
        .clone()
        .try_acquire_owned()
        .context("slot API concurrency limit")
}
impl KeySource {
    async fn acceptance(
        &self,
        request: &AcceptanceRequest,
        authorization: &Authorization,
        pin: &str,
    ) -> Result<serde_json::Value> {
        ensure!(
            parse_pin(&request.certificate_sha256)? == parse_pin(pin)?,
            "acceptance identity mismatch"
        );
        match self {
            #[cfg(all(test, unix))]
            Self::Fixed(_) => {}
            Self::Slot { url, token } => {
                let base = url.split('?').next().context("invalid slot keys URL")?;
                let origin = base
                    .strip_suffix("/v1/build/pool/slot-keys")
                    .context("slot keys URL cannot derive acceptance API")?;
                let url = format!("{origin}/v1/build/pool/native-acceptance");
                let token = token.clone();
                let body = serde_json::to_vec(
                    &serde_json::json!({"nonce": request.challenge, "certificate_sha256": pin}),
                )?;
                let permit = slot_api_permit()?;
                tokio::task::spawn_blocking(move || -> Result<()> {
                    let _permit = permit;
                    let config = ureq::Agent::config_builder()
                        .timeout_global(Some(Duration::from_secs(10)))
                        .max_redirects(0)
                        .build();
                    let mut response = config
                        .new_agent()
                        .post(&url)
                        .header("Authorization", &format!("Bearer {token}"))
                        .header("Content-Type", "application/json")
                        .send(body.as_slice())
                        .context("submit supervisor native acceptance")?;
                    let bytes = response
                        .body_mut()
                        .with_config()
                        .limit(16384)
                        .read_to_vec()?;
                    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                    ensure!(
                        value.get("accepted").and_then(|v| v.as_bool()) == Some(true),
                        "control plane did not verify native acceptance"
                    );
                    Ok(())
                })
                .await??;
            }
        }
        // The control plane stores the actual receipt. These fields are only
        // returned after its slot-authenticated confirmation, never client
        // authority to change lease or SSH state on their own.
        Ok(
            serde_json::json!({"accepted":true,"nonce":request.challenge,"certificate_sha256":pin,"account_id":authorization.account_id,"lease_id":authorization.lease_id}),
        )
    }
    async fn fetch(&self) -> Result<Authorization> {
        match self {
            #[cfg(all(test, unix))]
            Self::Fixed(keys) => Ok(Authorization {
                keys: keys.clone(),
                account_id: "test".into(),
                lease_id: "test-lease".into(),
            }),
            Self::Slot { url, token } => {
                let permit = slot_api_permit()?;
                let url = url.clone();
                let token = token.clone();
                tokio::task::spawn_blocking(move || -> Result<Authorization> {
                    let _permit = permit;
                    let config = ureq::Agent::config_builder()
                        .timeout_global(Some(Duration::from_secs(10)))
                        .max_redirects(0)
                        .build();
                    let mut response = config
                        .new_agent()
                        .get(&url)
                        .header("Authorization", &format!("Bearer {token}"))
                        .call()
                        .context("fetch valid slot SSH keys")?;
                    let bytes = response
                        .body_mut()
                        .with_config()
                        .limit(1024 * 1024)
                        .read_to_vec()?;
                    let auth: Authorization = serde_json::from_slice(&bytes).context(
                        "native slot keys API must provide lease-scoped authorization JSON",
                    )?;
                    ensure!(
                        auth.keys.len() <= 1024
                            && auth.keys.iter().all(|key| key.len() <= 4096)
                            && !auth.account_id.is_empty()
                            && auth.account_id.len() <= 256
                            && !auth.lease_id.is_empty()
                            && auth.lease_id.len() <= 256,
                        "invalid slot authorization"
                    );
                    Ok(auth)
                })
                .await?
            }
        }
    }
}

/// Bind and serve forever. Limit accepted TLS handshakes and commands to 64 combined.
#[cfg(all(test, unix))]
#[allow(dead_code)] // Used by the separate process-level bulk-transfer fixture.
pub async fn serve_test(
    listener: TcpListener,
    cert: Vec<u8>,
    key: Vec<u8>,
    keys: Vec<String>,
    home: PathBuf,
) -> Result<()> {
    let user = ExecutionUser {
        uid: unsafe { libc::geteuid() },
        gid: unsafe { libc::getegid() },
        home: home.to_string_lossy().into_owned(),
    };
    let pin = certificate_pin(&cert);
    serve_listener(
        listener,
        server_config(cert, key)?,
        KeySource::Fixed(keys),
        user,
        pin,
    )
    .await
}

/// Bind a production guest supervisor, with root-only secrets and ubuntu children.
pub async fn serve(config: ServeConfig) -> Result<()> {
    ensure!(
        config.slot_keys_url.starts_with("https://"),
        "slot keys API must use HTTPS"
    );
    let user = ubuntu_user()?;
    let key = read_root_secret(&config.key)?;
    let token = String::from_utf8(read_root_secret(&config.slot_token_file)?)?
        .trim()
        .to_owned();
    ensure!(!token.is_empty(), "empty slot credential");
    let cert = std::fs::read(&config.cert)?;
    let pin = certificate_pin(&cert);
    let tls = server_config(cert, key)?;
    let listener = TcpListener::bind(&config.bind).await?;
    serve_listener(
        listener,
        tls,
        KeySource::Slot {
            url: config.slot_keys_url,
            token,
        },
        user,
        pin,
    )
    .await
}
async fn serve_listener(
    listener: TcpListener,
    tls: Arc<rustls::ServerConfig>,
    keys: KeySource,
    user: ExecutionUser,
    pin: String,
) -> Result<()> {
    let capacity = Arc::new(tokio::sync::Semaphore::new(64));
    let resume = Arc::new(tokio::sync::Mutex::new(ResumeKeys::default()));
    loop {
        let (socket, _) = listener.accept().await?;
        let Ok(permit) = capacity.clone().try_acquire_owned() else {
            drop(socket);
            continue;
        };
        let acceptor = TlsAcceptor::from(tls.clone());
        let keys = keys.clone();
        let user = user.clone();
        let resume = resume.clone();
        let pin = pin.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(error) = serve_connection(socket, acceptor, keys, user, resume, pin).await {
                // Do not echo credentials or untrusted command text in supervisor logs.
                eprintln!("native connection closed: {error}");
            }
        });
    }
}
async fn serve_connection(
    socket: TcpStream,
    acceptor: TlsAcceptor,
    keys: KeySource,
    user: ExecutionUser,
    resume: Arc<tokio::sync::Mutex<ResumeKeys>>,
    pin: String,
) -> Result<()> {
    socket.set_nodelay(true)?;
    let mut tls = tokio::time::timeout(AUTH_TIMEOUT, acceptor.accept(socket))
        .await
        .context("TLS handshake timeout")??;
    ensure!(
        tls.get_ref().1.alpn_protocol() == Some(NAMESPACE.as_bytes()),
        "native ALPN required"
    );
    let exporter = tls
        .get_ref()
        .1
        .export_keying_material([0u8; 32], LABEL, None)?;
    let mut challenge = [0u8; 32];
    rustls::crypto::ring::default_provider()
        .secure_random
        .fill(&mut challenge)
        .map_err(|_| anyhow::anyhow!("secure randomness unavailable"))?;
    let (proof, authorization) = tokio::time::timeout(AUTH_TIMEOUT, async {
        write_frame(&mut tls, CHALLENGE, &challenge).await?;
        let (kind, bytes) = read_frame(&mut tls).await?;
        ensure!(
            kind == AUTH || kind == ACCEPT_AUTH,
            "native authentication required"
        );
        let proof: Proof = serde_json::from_slice(&bytes)?;
        ensure!(
            (kind == ACCEPT_AUTH) == proof.acceptance.is_some(),
            "native request kind/proof mismatch"
        );
        if let Some(request) = &proof.acceptance {
            ensure!(
                parse_pin(&request.certificate_sha256)? == parse_pin(&pin)?,
                "acceptance pin does not match guest identity"
            );
        }
        let authorization = keys.fetch().await?;
        let decision = resume
            .lock()
            .await
            .authorize(&proof, &exporter, &challenge, &authorization);
        if let Err(error) = decision {
            // This is the only retryable response. It is emitted on the pinned
            // TLS channel before READY and before any command can be spawned.
            write_frame(&mut tls, AUTH_REJECTED, AUTH_REJECTION_BODY).await?;
            return Err(error);
        }
        if proof.acceptance.is_none() {
            write_frame(&mut tls, READY, &[]).await?;
        }
        Ok::<_, anyhow::Error>((proof, authorization))
    })
    .await
    .context("native authentication timeout")??;
    if let Some(request) = &proof.acceptance {
        let receipt =
            tokio::time::timeout(AUTH_TIMEOUT, keys.acceptance(request, &authorization, &pin))
                .await
                .context("native acceptance receipt timeout")??;
        write_frame(&mut tls, ACCEPT_RECEIPT, &serde_json::to_vec(&receipt)?).await?;
        tls.shutdown().await?;
        return Ok(());
    }
    let command = proof.command;
    let (mut input, mut output) = tokio::io::split(tls);
    let result = tokio::time::timeout(
        EXEC_TIMEOUT,
        execute(&command, &user, &mut input, &mut output),
    )
    .await;
    match result {
        Ok(Ok(code)) => write_frame(&mut output, EXIT, &code.to_be_bytes()).await?,
        Ok(Err(error)) => {
            let message = format!("native command I/O failed: {error:#}");
            let _ = write_frame(&mut output, ERROR, message.as_bytes()).await;
            return Err(error.context("native command I/O failed"));
        }
        Err(_) => {
            let _ = write_frame(&mut output, ERROR, b"native command deadline exceeded").await;
            bail!("native command deadline exceeded");
        }
    }
    output.shutdown().await?;
    // EXIT is already flushed and the child is reaped. Keep consuming residual
    // upload bytes until the client observes EXIT and closes, instead of dropping
    // a socket with unread input (which can reset TCP and lose the final output).
    // execute's frame reader may have been cancelled midway through a frame, so
    // this terminal cleanup deliberately drains bytes, not protocol frames.
    // Peer disconnect/errors here cannot change the command result already sent.
    let _ = tokio::time::timeout(
        IO_TIMEOUT,
        tokio::io::copy(&mut input, &mut tokio::io::sink()),
    )
    .await;
    Ok(())
}

#[cfg(unix)]
struct ProcessGroup(u32);
#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.0 as i32), libc::SIGKILL);
        }
    }
}
#[cfg(unix)]
async fn execute<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    command: &str,
    user: &ExecutionUser,
    input: &mut R,
    output: &mut W,
) -> Result<i32> {
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::Stdio;
    let mut cmd = tokio::process::Command::new("/bin/bash");
    cmd.arg("-c")
        .arg(command)
        .env_clear()
        .env("HOME", &user.home)
        .env("USER", "ubuntu")
        .env("LOGNAME", "ubuntu")
        .env(
            "PATH",
            "/home/ubuntu/.cargo/bin:/usr/local/bin:/usr/bin:/bin",
        )
        .current_dir(&user.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd.as_std_mut().process_group(0);
    let uid = user.uid;
    let gid = user.gid;
    unsafe {
        cmd.pre_exec(move || {
            if libc::geteuid() == 0 {
                if libc::setgroups(0, std::ptr::null()) != 0
                    || libc::setgid(gid) != 0
                    || libc::setuid(uid) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
            } else if libc::geteuid() != uid || libc::getegid() != gid {
                return Err(std::io::Error::from_raw_os_error(libc::EPERM));
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    let _group = ProcessGroup(child.id().context("missing child pid")?);
    let mut stdin = child.stdin.take();
    let mut stdout = child.stdout.take().context("missing stdout")?;
    let mut stderr = child.stderr.take().context("missing stderr")?;
    // Keep framing in a persistent future: cancelling read_exact midway corrupts
    // the stream. Independently drive stdin so a child emitting output before
    // consuming its input cannot deadlock the bidirectional connection.
    let send_input = async {
        let mut eof = false;
        loop {
            let (kind, bytes) = read_frame(input).await?;
            ensure!(!eof, "native stdin frame after EOF");
            match kind {
                STDIN => {
                    if let Some(writer) = &mut stdin {
                        match tokio::time::timeout(IO_TIMEOUT, writer.write_all(&bytes))
                            .await
                            .context("command stdin stalled")?
                        {
                            Ok(()) => {}
                            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                                stdin.take();
                            }
                            Err(e) => return Err(e.into()),
                        }
                    }
                }
                EOF if bytes.is_empty() => {
                    stdin.take();
                    eof = true;
                }
                _ => bail!("invalid native stdin frame"),
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };
    let wait = async {
        let status = child.wait().await?;
        Ok(status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)))
    };
    let receive_output = forward_command_output(&mut stdout, &mut stderr, output, wait);
    tokio::pin!(send_input);
    tokio::pin!(receive_output);
    tokio::select! {
        result = &mut receive_output => result,
        result = &mut send_input => { result?; receive_output.await }
    }
}

#[cfg(unix)]
async fn forward_command_output(
    stdout: &mut (impl AsyncRead + Unpin),
    stderr: &mut (impl AsyncRead + Unpin),
    output: &mut (impl AsyncWrite + Unpin),
    wait: impl std::future::Future<Output = Result<i32>>,
) -> Result<i32> {
    tokio::pin!(wait);
    let mut out_open = true;
    let mut err_open = true;
    let mut out_buf = vec![0; 32768];
    let mut err_buf = vec![0; 32768];
    let mut status = None;
    while out_open || err_open || status.is_none() {
        tokio::select! {
            // rsync can terminate its transport as soon as it sees a fatal
            // stdout protocol message. Forward already-ready diagnostics
            // first, rather than letting that message overtake stderr.
            biased;
            read = stderr.read(&mut err_buf), if err_open => {
                let len = read?; if len == 0 { err_open = false; } else { write_frame(output, STDERR, &err_buf[..len]).await?; }
            }
            read = stdout.read(&mut out_buf), if out_open => {
                let len = read?; if len == 0 { out_open = false; } else { write_frame(output, STDOUT, &out_buf[..len]).await?; }
            }
            exit = &mut wait, if status.is_none() => { status = Some(exit?); }
        }
    }
    status.context("missing command exit")
}

#[cfg(not(unix))]
async fn execute<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    _: &str,
    _: &ExecutionUser,
    _: &mut R,
    _: &mut W,
) -> Result<i32> {
    bail!("native guest execution requires Unix")
}

/// Stream a command over one authenticated TLS connection. No buffering of command output.
/// Durable agent/session protocols can run unchanged as the command's byte streams.
#[allow(clippy::too_many_arguments)]
pub async fn exec_io<R: AsyncRead + Unpin, W: AsyncWrite + Unpin, E: AsyncWrite + Unpin>(
    address: &str,
    server_name: &str,
    pin: &str,
    identity: &Path,
    command: &str,
    stdin: &mut R,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<i32> {
    let key =
        ssh_key::PrivateKey::read_openssh_file(identity).context("read native SSH identity")?;
    exec_key_io(
        address,
        server_name,
        pin,
        &key,
        command,
        stdin,
        stdout,
        stderr,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
async fn exec_key_io<R: AsyncRead + Unpin, W: AsyncWrite + Unpin, E: AsyncWrite + Unpin>(
    address: &str,
    server_name: &str,
    pin: &str,
    key: &ssh_key::PrivateKey,
    command: &str,
    stdin: &mut R,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<i32> {
    exec_key_enroll_io(
        address,
        server_name,
        pin,
        key,
        command,
        None,
        stdin,
        stdout,
        stderr,
    )
    .await
}
async fn connect_request(
    address: &str,
    server_name: &str,
    pin: &str,
    key: &ssh_key::PrivateKey,
    command: &str,
    enroll: Option<String>,
    acceptance: Option<AcceptanceRequest>,
) -> Result<(
    tokio_rustls::client::TlsStream<TcpStream>,
    Option<serde_json::Value>,
)> {
    let timeout = if acceptance.is_some() {
        AUTH_TIMEOUT * 2
    } else {
        AUTH_TIMEOUT
    };
    tokio::time::timeout(timeout, async {
        let socket = TcpStream::connect(address).await?;
        socket.set_nodelay(true)?;
        let mut tls = TlsConnector::from(client_config(pin)?)
            .connect(ServerName::try_from(server_name.to_owned())?, socket)
            .await?;
        ensure!(
            tls.get_ref().1.alpn_protocol() == Some(NAMESPACE.as_bytes()),
            "native ALPN required"
        );
        let exporter = tls
            .get_ref()
            .1
            .export_keying_material([0u8; 32], LABEL, None)?;
        let (kind, challenge) = read_frame(&mut tls).await?;
        ensure!(
            kind == CHALLENGE && challenge.len() == 32,
            "invalid native challenge"
        );
        let proof = prove_request(
            key,
            &exporter,
            &challenge,
            command,
            enroll,
            acceptance.clone(),
        )?;
        write_frame(
            &mut tls,
            if acceptance.is_some() {
                ACCEPT_AUTH
            } else {
                AUTH
            },
            &serde_json::to_vec(&proof)?,
        )
        .await?;
        let (kind, body) = read_frame(&mut tls).await?;
        if kind == AUTH_REJECTED && body == AUTH_REJECTION_BODY {
            return Err(NativeAuthenticationRejected.into());
        }
        let receipt = if let Some(request) = &acceptance {
            ensure!(
                kind == ACCEPT_RECEIPT && body.len() <= 16384,
                "native acceptance was not confirmed"
            );
            let value: serde_json::Value = serde_json::from_slice(&body)?;
            let receipt_pin = value
                .get("certificate_sha256")
                .and_then(|v| v.as_str())
                .context("receipt lacks certificate pin")?;
            ensure!(
                value.get("accepted").and_then(|v| v.as_bool()) == Some(true)
                    && value.get("nonce").and_then(|v| v.as_str())
                        == Some(request.challenge.as_str())
                    && parse_pin(receipt_pin)? == parse_pin(pin)?,
                "native acceptance receipt binding mismatch"
            );
            Some(value)
        } else {
            ensure!(
                kind == READY && body.is_empty(),
                "native authentication rejected"
            );
            None
        };
        Ok::<_, anyhow::Error>((tls, receipt))
    })
    .await
    .context("native connection/authentication timeout")
    .and_then(|result| result)
    .context("native connection/authentication failed (request outcome may be unknown)")
}
#[allow(clippy::too_many_arguments)]
async fn exec_key_enroll_io<R: AsyncRead + Unpin, W: AsyncWrite + Unpin, E: AsyncWrite + Unpin>(
    address: &str,
    server_name: &str,
    pin: &str,
    key: &ssh_key::PrivateKey,
    command: &str,
    enroll: Option<String>,
    stdin: &mut R,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<i32> {
    let (mut tls, _) =
        connect_request(address, server_name, pin, key, command, enroll, None).await?;
    let (mut read, mut write) = tokio::io::split(&mut tls);
    let send = async {
        let mut buffer = vec![0; 32768];
        loop {
            let len = stdin
                .read(&mut buffer)
                .await
                .map_err(|error| CommandInputError::Source(error.into()))?;
            if len == 0 {
                write_frame(&mut write, EOF, &[])
                    .await
                    .map_err(CommandInputError::Transport)?;
                break;
            }
            write_frame(&mut write, STDIN, &buffer[..len])
                .await
                .map_err(CommandInputError::Transport)?;
        }
        Ok::<_, CommandInputError>(())
    };
    let receive = async {
        loop {
            let (kind, bytes) = read_frame(&mut read).await?;
            match kind {
                STDOUT => {
                    stdout.write_all(&bytes).await?;
                    stdout.flush().await?;
                }
                STDERR => {
                    stderr.write_all(&bytes).await?;
                    stderr.flush().await?;
                }
                EXIT => {
                    ensure!(bytes.len() == 4, "invalid native exit frame");
                    return Ok::<_, anyhow::Error>(i32::from_be_bytes(bytes.try_into().unwrap()));
                }
                ERROR => bail!("native server: {}", String::from_utf8_lossy(&bytes)),
                _ => bail!("invalid native output frame"),
            }
        }
    };
    tokio::time::timeout(
        EXEC_TIMEOUT + IO_TIMEOUT,
        drain_command_response(send, receive),
    )
    .await
    .context("native command deadline exceeded")?
}

enum CommandInputError {
    Source(anyhow::Error),
    Transport(anyhow::Error),
}

// A command can exit without consuming all of stdin. Its EXIT and final stderr
// may already be buffered when the concurrent upload observes the closed TLS
// connection. Never cancel the response reader on that upload failure.
async fn drain_command_response(
    send: impl std::future::Future<Output = std::result::Result<(), CommandInputError>>,
    receive: impl std::future::Future<Output = Result<i32>>,
) -> Result<i32> {
    tokio::pin!(send);
    tokio::pin!(receive);
    tokio::select! {
        result = &mut receive => result,
        result = &mut send => match result {
            Ok(()) => receive.await,
            Err(failure) => {
                let (error, closed_transport) = match failure {
                    CommandInputError::Source(error) => (error, false),
                    CommandInputError::Transport(error) => {
                        let closed = error.downcast_ref::<std::io::Error>().is_some_and(|e| {
                            matches!(e.kind(), std::io::ErrorKind::BrokenPipe
                                | std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::ConnectionAborted
                                | std::io::ErrorKind::NotConnected)
                        });
                        (error, closed)
                    }
                };
                // Bound draining if this is a genuine input/transport failure
                // rather than a peer that has finished the command.
                match tokio::time::timeout(IO_TIMEOUT, receive).await {
                    Ok(Ok(code)) if closed_transport => Ok(code),
                    Ok(Ok(_)) => Err(error.context("native stdin failed")),
                    Ok(Err(response_error)) => Err(response_error.context(format!(
                        "native stdin failed: {error:#}"
                    ))),
                    Err(_) => Err(error.context("native response drain timeout after stdin failure")),
                }
            }
        }
    }
}

fn resume_identity(path: &Path) -> Result<ssh_key::PrivateKey> {
    use std::io::Read;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        let key = ssh_key::PrivateKey::random(
            &mut ssh_key::rand_core::OsRng,
            ssh_key::Algorithm::Ed25519,
        )?;
        let pem = key.to_openssh(ssh_key::LineEnding::LF)?;
        match write_private_new(path, pem.as_bytes()) {
            Ok(()) => {}
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::AlreadyExists) => {}
            Err(error) => return Err(error),
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_FRAME as u64,
        "invalid resume identity file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
            "resume identity must be private and owned by current user"
        );
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(ssh_key::PrivateKey::from_openssh(bytes)?)
}

fn endpoint_address(endpoint: &Endpoint) -> String {
    if endpoint.address.contains(':') && !endpoint.address.starts_with('[') {
        format!("[{}]:{}", endpoint.address, endpoint.port)
    } else {
        format!("{}:{}", endpoint.address, endpoint.port)
    }
}

/// Ask the authenticated guest supervisor to record its control-plane receipt.
/// This never runs a shell command or acknowledges promotion on the caller's behalf.
pub async fn accept_endpoint(endpoint: &Endpoint, nonce: &str) -> Result<serde_json::Value> {
    ensure!(
        nonce.len() == 64
            && nonce
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "acceptance nonce must be 64 lowercase hex digits"
    );
    parse_pin(&endpoint.certificate_sha256)?;
    let request = AcceptanceRequest {
        challenge: nonce.into(),
        certificate_sha256: endpoint.certificate_sha256.clone(),
    };
    let address = endpoint_address(endpoint);
    let resume = resume_identity(&endpoint.resume_path)?;
    let result = connect_request(
        &address,
        &endpoint.server_name,
        &endpoint.certificate_sha256,
        &resume,
        "",
        None,
        Some(request.clone()),
    )
    .await;
    let (_, receipt) = match result {
        Ok(value) => value,
        Err(error)
            if error
                .downcast_ref::<NativeAuthenticationRejected>()
                .is_some() =>
        {
            let initial = ssh_key::PrivateKey::read_openssh_file(&endpoint.key_path)?;
            connect_request(
                &address,
                &endpoint.server_name,
                &endpoint.certificate_sha256,
                &initial,
                "",
                Some(resume.public_key().to_openssh()?),
                Some(request),
            )
            .await?
        }
        Err(error) => return Err(error),
    };
    receipt.context("missing native acceptance receipt")
}

pub async fn exec_endpoint_io<
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    E: AsyncWrite + Unpin,
>(
    endpoint: &Endpoint,
    script: &str,
    stdin: &mut R,
    stdout: &mut W,
    stderr: &mut E,
) -> Result<i32> {
    let address = endpoint_address(endpoint);
    let resume = resume_identity(&endpoint.resume_path)?;
    let result = exec_key_io(
        &address,
        &endpoint.server_name,
        &endpoint.certificate_sha256,
        &resume,
        script,
        stdin,
        stdout,
        stderr,
    )
    .await;
    match result {
        Ok(code) => Ok(code),
        Err(error)
            if error
                .downcast_ref::<NativeAuthenticationRejected>()
                .is_some() =>
        {
            let key = ssh_key::PrivateKey::read_openssh_file(&endpoint.key_path)?;
            exec_key_enroll_io(
                &address,
                &endpoint.server_name,
                &endpoint.certificate_sha256,
                &key,
                script,
                Some(resume.public_key().to_openssh()?),
                stdin,
                stdout,
                stderr,
            )
            .await
        }
        Err(error) => Err(error),
    }
}

/// Hidden setup CLI. `client` returns the remote exit code and `accept` prints a receipt.
/// `init --cert PATH --key PATH`, `serve ...`, or
/// `client --address HOST:443 --server-name SLOT --pin HEX --identity PATH --command SHELL`.
pub fn cli(args: &[String]) -> Result<i32> {
    let action = args
        .first()
        .context("expected native init, serve, client, rsync, or accept")?;
    let option = |name: &str| -> Result<String> {
        let position = args
            .iter()
            .position(|s| s == name)
            .with_context(|| format!("missing {name}"))?;
        args.get(position + 1)
            .cloned()
            .with_context(|| format!("missing value for {name}"))
    };
    match action.as_str() {
        "accept" => {
            ensure!(
                args.len() == 3,
                "native accept requires endpoint JSON and nonce"
            );
            let endpoint: Endpoint =
                serde_json::from_str(&args[1]).context("invalid native endpoint JSON")?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .max_blocking_threads(4)
                .enable_all()
                .build()?;
            let result = runtime.block_on(accept_endpoint(&endpoint, &args[2]));
            runtime.shutdown_background();
            println!("{}", serde_json::to_string(&result?)?);
            Ok(0)
        }
        "client" | "rsync" if args.get(1).is_some_and(|s| s.starts_with('{')) => {
            let endpoint: Endpoint =
                serde_json::from_str(&args[1]).context("invalid native endpoint JSON")?;
            let script = if action == "rsync" {
                ensure!(
                    args.len() >= 4,
                    "native rsync requires destination and remote command"
                );
                args[3..]
                    .iter()
                    .map(|arg| format!("'{}'", arg.replace('\'', "'\\''")))
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                ensure!(args.len() >= 3, "native client requires command");
                args[2..].join(" ")
            };
            let runtime = tokio::runtime::Builder::new_current_thread()
                .max_blocking_threads(4)
                .enable_all()
                .build()?;
            let result = runtime.block_on(exec_endpoint_io(
                &endpoint,
                &script,
                &mut tokio::io::stdin(),
                &mut tokio::io::stdout(),
                &mut tokio::io::stderr(),
            ));
            // Tokio's standard-input read is blocking and cannot be cancelled.
            // __native exits immediately after returning, so do not wait for an
            // inherited open terminal/pipe when the remote command has exited.
            runtime.shutdown_background();
            result
        }
        "init" => {
            let certified = rcgen::generate_simple_self_signed(vec!["justrust-native".into()])?;
            let cert = certified.cert.der().to_vec();
            let key = certified.signing_key.serialize_der();
            write_private_new(Path::new(&option("--key")?), &key)?;
            write_private_new(Path::new(&option("--cert")?), &cert)?;
            println!("{}", certificate_pin(&cert));
            Ok(0)
        }
        "serve" => {
            let config = ServeConfig {
                bind: option("--bind")?,
                cert: option("--cert")?.into(),
                key: option("--key")?.into(),
                slot_token_file: option("--slot-token-file")?.into(),
                slot_keys_url: option("--slot-keys-url")?,
            };
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .max_blocking_threads(64)
                .enable_all()
                .build()?
                .block_on(serve(config))?;
            Ok(0)
        }
        "client" => {
            let address = option("--address")?;
            let name = option("--server-name")?;
            let pin = option("--pin")?;
            let identity = PathBuf::from(option("--identity")?);
            let command = option("--command")?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .max_blocking_threads(4)
                .enable_all()
                .build()?;
            let result = runtime.block_on(exec_io(
                &address,
                &name,
                &pin,
                &identity,
                &command,
                &mut tokio::io::stdin(),
                &mut tokio::io::stdout(),
                &mut tokio::io::stderr(),
            ));
            runtime.shutdown_background();
            result
        }
        _ => bail!("unknown native action"),
    }
}
fn write_private_new(path: &Path, data: &[u8]) -> Result<()> {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let name = path
        .file_name()
        .context("private key path needs file name")?
        .to_string_lossy();
    let temporary = path.with_file_name(format!(
        ".{name}.{}.{}.new",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let result = (|| -> Result<()> {
        std::io::Write::write_all(&mut file, data)?;
        file.sync_all()?;
        // Hard link is atomic and refuses replacement, unlike rename.
        std::fs::hard_link(&temporary, path)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn key() -> ssh_key::PrivateKey {
        ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ed25519)
            .unwrap()
    }
    fn user() -> ExecutionUser {
        ExecutionUser {
            uid: unsafe { libc::geteuid() },
            gid: unsafe { libc::getegid() },
            home: "/tmp".into(),
        }
    }
    async fn loopback(keys: Vec<String>) -> (String, String, tokio::task::JoinHandle<Result<()>>) {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let pin = certificate_pin(cert.cert.der());
        let config =
            server_config(cert.cert.der().to_vec(), cert.signing_key.serialize_der()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let server = tokio::spawn(serve_listener(
            listener,
            config,
            KeySource::Fixed(keys),
            user(),
            pin.clone(),
        ));
        (address, pin, server)
    }
    #[tokio::test]
    async fn ready_stderr_precedes_fatal_stdout_and_exit() {
        // Both pipes and exit are ready on the first poll, without process or
        // timer scheduling. A fatal rsync stdout message can kill the transport,
        // so its already-ready diagnostic must be sent before that message.
        for _ in 0..64 {
            let mut stdout = b"fatal rsync protocol message".as_slice();
            let diagnostic = vec![b'e'; 3 * 32768 + 17];
            let mut stderr = diagnostic.as_slice();
            let mut output = Vec::new();
            assert_eq!(
                forward_command_output(&mut stdout, &mut stderr, &mut output, async { Ok(3) })
                    .await
                    .unwrap(),
                3
            );
            let mut frames = output.as_slice();
            let mut received_stderr = Vec::new();
            for _ in 0..4 {
                let (kind, body) = read_frame(&mut frames).await.unwrap();
                assert_eq!(kind, STDERR, "fatal stdout overtook ready diagnostic");
                received_stderr.extend_from_slice(&body);
            }
            assert_eq!(received_stderr, diagnostic);
            let (kind, body) = read_frame(&mut frames).await.unwrap();
            assert_eq!(kind, STDOUT);
            assert_eq!(body, b"fatal rsync protocol message");
            assert!(frames.is_empty());
        }
    }

    #[tokio::test]
    async fn stderr_priority_does_not_block_stdout_or_hide_wait_errors() {
        let (mut stderr_source, mut stderr) = tokio::io::duplex(128);
        let (mut frames, mut output) = tokio::io::duplex(128);
        let task = tokio::spawn(async move {
            forward_command_output(
                &mut b"stdout while stderr pending".as_slice(),
                &mut stderr,
                &mut output,
                async { Ok(37) },
            )
            .await
        });
        // No stderr is available yet. Stdout must still be forwarded, without
        // waiting for stderr EOF or the child's exit.
        let (kind, body) = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut frames))
            .await
            .expect("ready stdout blocked behind pending stderr")
            .unwrap();
        assert_eq!(kind, STDOUT);
        assert_eq!(body, b"stdout while stderr pending");
        stderr_source.write_all(b"late diagnostic").await.unwrap();
        stderr_source.shutdown().await.unwrap();
        let (kind, body) = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut frames))
            .await
            .expect("late stderr was not drained")
            .unwrap();
        assert_eq!(kind, STDERR);
        assert_eq!(body, b"late diagnostic");
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .expect("output drain did not finish after stderr EOF")
                .unwrap()
                .unwrap(),
            37
        );

        let mut output = Vec::new();
        let error = forward_command_output(
            &mut b"".as_slice(),
            &mut b"diagnostic before wait error".as_slice(),
            &mut output,
            async { bail!("injected child wait failure") },
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("injected child wait failure"));
        let (kind, body) = read_frame(&mut output.as_slice()).await.unwrap();
        assert_eq!(kind, STDERR);
        assert_eq!(body, b"diagnostic before wait error");
    }

    #[tokio::test]
    async fn upload_close_before_response_drains_stderr_and_preserves_exit() {
        // The response cannot finish until the upload has failed. This forces
        // the select branch that used to drop the reader, without timing sleeps.
        for kind in [
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::NotConnected,
        ] {
            let (sent, failed) = tokio::sync::oneshot::channel();
            let mut stderr = Vec::new();
            let result = drain_command_response(
                async {
                    sent.send(()).unwrap();
                    Err(CommandInputError::Transport(
                        std::io::Error::from(kind).into(),
                    ))
                },
                async {
                    failed.await.unwrap();
                    stderr.extend_from_slice(b"remote command failed\n");
                    Ok(37)
                },
            )
            .await
            .unwrap();
            assert_eq!(result, 37);
            assert_eq!(stderr, b"remote command failed\n");
        }
    }

    #[tokio::test]
    async fn upload_failure_drains_but_does_not_hide_genuine_errors() {
        for source in [true, false] {
            let (sent, failed) = tokio::sync::oneshot::channel();
            let mut stderr = Vec::new();
            let error = drain_command_response(
                async {
                    sent.send(()).unwrap();
                    let error = std::io::Error::new(
                        if source {
                            std::io::ErrorKind::BrokenPipe
                        } else {
                            std::io::ErrorKind::Other
                        },
                        "injected input failure",
                    )
                    .into();
                    Err(if source {
                        CommandInputError::Source(error)
                    } else {
                        CommandInputError::Transport(error)
                    })
                },
                async {
                    failed.await.unwrap();
                    stderr.extend_from_slice(b"final stderr");
                    Ok(0)
                },
            )
            .await
            .unwrap_err();
            assert!(format!("{error:#}").contains("injected input failure"));
            assert_eq!(stderr, b"final stderr");
        }
        let (sent, failed) = tokio::sync::oneshot::channel();
        let error = drain_command_response(
            async {
                sent.send(()).unwrap();
                Err(CommandInputError::Transport(
                    std::io::Error::from(std::io::ErrorKind::BrokenPipe).into(),
                ))
            },
            async {
                failed.await.unwrap();
                bail!("injected malformed EXIT")
            },
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("injected malformed EXIT"));
    }

    #[tokio::test]
    async fn tls_exec_binary_stdin_stderr_and_exit() {
        let key = key();
        let (address, pin, server) = loopback(vec![key.public_key().to_openssh().unwrap()]).await;
        let input = vec![0u8, 255, 1, 128, 10];
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = exec_key_io(
            &address,
            "localhost",
            &pin,
            &key,
            "cat; printf problem >&2; exit 37",
            &mut input.as_slice(),
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();
        assert_eq!(result, 37);
        assert_eq!(stdout, input);
        assert_eq!(stderr, b"problem");
        server.abort();
    }
    #[tokio::test]
    async fn tls_large_bidirectional_stream_does_not_deadlock() {
        let key = key();
        let (address, pin, server) = loopback(vec![key.public_key().to_openssh().unwrap()]).await;
        let input = vec![b'x'; 1024 * 1024];
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = tokio::time::timeout(
            Duration::from_secs(20),
            exec_key_io(
                &address,
                "localhost",
                &pin,
                &key,
                "head -c 1048576 /dev/zero; cat",
                &mut input.as_slice(),
                &mut stdout,
                &mut stderr,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result, 0);
        assert_eq!(stdout.len(), 2 * input.len());
        assert_eq!(&stdout[input.len()..], input.as_slice());
        server.abort();
    }
    #[tokio::test]
    async fn tls_wrong_pin_and_unauthorized_identity_rejected() {
        let allowed = key();
        let stranger = key();
        let (address, pin, server) =
            loopback(vec![allowed.public_key().to_openssh().unwrap()]).await;
        for (pin, key, retryable) in [("00".repeat(32), &allowed, false), (pin, &stranger, true)] {
            let error = exec_key_io(
                &address,
                "localhost",
                &pin,
                key,
                "exit 0",
                &mut &b""[..],
                &mut Vec::new(),
                &mut Vec::new(),
            )
            .await
            .unwrap_err();
            assert_eq!(
                error
                    .downcast_ref::<NativeAuthenticationRejected>()
                    .is_some(),
                retryable
            );
        }
        server.abort();
    }
    #[tokio::test]
    async fn tls12_is_rejected() {
        let (address, pin, server) = loopback(vec![]).await;
        let mut config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS12])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedCertificate(parse_pin(&pin).unwrap())))
        .with_no_client_auth();
        config.alpn_protocols = vec![NAMESPACE.as_bytes().to_vec()];
        let result = TlsConnector::from(Arc::new(config))
            .connect(
                ServerName::try_from("localhost").unwrap(),
                TcpStream::connect(&address).await.unwrap(),
            )
            .await;
        assert!(result.is_err());
        server.abort();
    }
    #[tokio::test]
    async fn concurrent_commands_preserve_signal_exit_and_clear_environment() {
        let key = key();
        let (address, pin, server) = loopback(vec![key.public_key().to_openssh().unwrap()]).await;
        let mut output = Vec::new();
        let mut errors = Vec::new();
        let mut output2 = Vec::new();
        let mut errors2 = Vec::new();
        let mut input1 = &b""[..];
        let mut input2 = &b""[..];
        let (first, second) = tokio::join!(
            exec_key_io(
                &address,
                "localhost",
                &pin,
                &key,
                "printf '%s' \"${JUSTRUST_API_KEY-unset}:${POOL_TOKEN-unset}\"; kill -TERM $$",
                &mut input1,
                &mut output,
                &mut errors
            ),
            exec_key_io(
                &address,
                "localhost",
                &pin,
                &key,
                "printf concurrent",
                &mut input2,
                &mut output2,
                &mut errors2
            )
        );
        assert_eq!(first.unwrap(), 143);
        assert_eq!(output, b"unset:unset");
        assert_eq!(second.unwrap(), 0);
        assert_eq!(output2, b"concurrent");
        server.abort();
    }
    #[test]
    fn exporter_nonce_and_command_binding_reject_replay() {
        let key = key();
        let allowed = vec![key.public_key().to_openssh().unwrap()];
        let mut proof = prove(&key, &[1; 32], &[2; 32], "true").unwrap();
        verify(&proof, &[1; 32], &[2; 32], &allowed).unwrap();
        assert!(verify(&proof, &[3; 32], &[2; 32], &allowed).is_err());
        assert!(verify(&proof, &[1; 32], &[3; 32], &allowed).is_err());
        proof.command = "false".into();
        assert!(verify(&proof, &[1; 32], &[2; 32], &allowed).is_err());
    }
    #[test]
    fn resume_survives_key_expiry_but_not_lease_rotation_or_restart() {
        let initial = key();
        let resume = key();
        let mut authorization = Authorization {
            keys: vec![initial.public_key().to_openssh().unwrap()],
            account_id: "account-a".into(),
            lease_id: "lease-1".into(),
        };
        let enrollment = prove_enroll(
            &initial,
            &[1; 32],
            &[2; 32],
            "true",
            Some(resume.public_key().to_openssh().unwrap()),
        )
        .unwrap();
        let mut state = ResumeKeys::default();
        state
            .authorize(&enrollment, &[1; 32], &[2; 32], &authorization)
            .unwrap();
        authorization.keys.clear();
        let reconnect = prove(&resume, &[3; 32], &[4; 32], "true").unwrap();
        state
            .authorize(&reconnect, &[3; 32], &[4; 32], &authorization)
            .unwrap();
        assert!(
            state
                .authorize(&reconnect, &[5; 32], &[4; 32], &authorization)
                .is_err()
        );
        assert!(
            ResumeKeys::default()
                .authorize(&reconnect, &[3; 32], &[4; 32], &authorization)
                .is_err()
        );
        authorization.lease_id = "lease-2".into();
        assert!(
            state
                .authorize(&reconnect, &[3; 32], &[4; 32], &authorization)
                .is_err()
        );
    }
    #[test]
    fn concurrent_resume_identity_creation_is_atomic_and_private() {
        use std::os::unix::fs::PermissionsExt;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "jr-native-resume-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("resume");
        let keys = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..16)
                .map(|_| {
                    scope.spawn(|| {
                        resume_identity(&path)
                            .unwrap()
                            .public_key()
                            .to_openssh()
                            .unwrap()
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(keys.iter().all(|key| key == &keys[0]));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(resume_identity(&path).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn real_tls_replay_on_fresh_connection_is_rejected() {
        let key = key();
        let (address, pin, server) = loopback(vec![key.public_key().to_openssh().unwrap()]).await;
        let connector = TlsConnector::from(client_config(&pin).unwrap());
        let mut first = connector
            .connect(
                ServerName::try_from("localhost").unwrap(),
                TcpStream::connect(&address).await.unwrap(),
            )
            .await
            .unwrap();
        let (_, challenge) = read_frame(&mut first).await.unwrap();
        let exporter = first
            .get_ref()
            .1
            .export_keying_material([0u8; 32], LABEL, None)
            .unwrap();
        let proof = prove(&key, &exporter, &challenge, "exit 0").unwrap();
        let bytes = serde_json::to_vec(&proof).unwrap();
        write_frame(&mut first, AUTH, &bytes).await.unwrap();
        assert_eq!(read_frame(&mut first).await.unwrap().0, READY);
        write_frame(&mut first, EOF, &[]).await.unwrap();
        assert_eq!(read_frame(&mut first).await.unwrap().0, EXIT);
        let mut second = connector
            .connect(
                ServerName::try_from("localhost").unwrap(),
                TcpStream::connect(&address).await.unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(read_frame(&mut second).await.unwrap().0, CHALLENGE);
        write_frame(&mut second, AUTH, &bytes).await.unwrap();
        assert_eq!(read_frame(&mut second).await.unwrap().0, AUTH_REJECTED);
        server.abort();
    }
    #[tokio::test]
    async fn lost_or_truncated_ready_never_retries_command() {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        for ready_bytes in [vec![], vec![READY, 0, 0], vec![READY, 0, 0, 0, 0]] {
            let directory = std::env::temp_dir().join(format!(
                "jr-native-ambiguous-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            let resume_path = directory.join("resume");
            let effects = directory.join("effects");
            let script = format!(
                "printf executed >> '{}'",
                effects.to_string_lossy().replace('\'', "'\\''")
            );
            let resume = resume_identity(&resume_path).unwrap();
            let initial = key();
            let key_path = directory.join("initial");
            write_private_new(
                &key_path,
                initial
                    .to_openssh(ssh_key::LineEnding::LF)
                    .unwrap()
                    .as_bytes(),
            )
            .unwrap();
            let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let config = server_config(
                certificate.cert.der().to_vec(),
                certificate.signing_key.serialize_der(),
            )
            .unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let endpoint = Endpoint {
                address: address.ip().to_string(),
                port: address.port(),
                server_name: "localhost".into(),
                certificate_sha256: certificate_pin(certificate.cert.der()),
                key_path,
                resume_path,
            };
            let (client_done, completed) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut tls = TlsAcceptor::from(config).accept(socket).await.unwrap();
                let exporter = tls
                    .get_ref()
                    .1
                    .export_keying_material([0u8; 32], LABEL, None)
                    .unwrap();
                let challenge = [7u8; 32];
                write_frame(&mut tls, CHALLENGE, &challenge).await.unwrap();
                let (kind, bytes) = read_frame(&mut tls).await.unwrap();
                assert_eq!(kind, AUTH);
                let proof: Proof = serde_json::from_slice(&bytes).unwrap();
                verify(
                    &proof,
                    &exporter,
                    &challenge,
                    &[resume.public_key().to_openssh().unwrap()],
                )
                .unwrap();
                // Model the execution boundary: the command has been admitted,
                // but its READY response can be lost or only partially received.
                tls.write_all(&ready_bytes).await.unwrap();
                tls.flush().await.unwrap();
                assert!(
                    tokio::process::Command::new("/bin/bash")
                        .arg("-c")
                        .arg(&proof.command)
                        .status()
                        .await
                        .unwrap()
                        .success()
                );
                drop(tls);
                tokio::select! {
                    _ = listener.accept() => panic!("ambiguous command was retried on another TLS connection"),
                    _ = completed => {},
                }
            });
            let error = exec_endpoint_io(
                &endpoint,
                &script,
                &mut &b""[..],
                &mut Vec::new(),
                &mut Vec::new(),
            )
            .await
            .unwrap_err();
            let _ = client_done.send(());
            assert!(
                error
                    .downcast_ref::<NativeAuthenticationRejected>()
                    .is_none()
            );
            server.await.unwrap();
            assert_eq!(std::fs::read(&effects).unwrap(), b"executed");
            std::fs::remove_dir_all(directory).unwrap();
        }
    }
    #[tokio::test]
    async fn explicit_rejection_enrolls_once_then_resumes_without_initial_key() {
        let initial = key();
        let (address, pin, server) =
            loopback(vec![initial.public_key().to_openssh().unwrap()]).await;
        let address: std::net::SocketAddr = address.parse().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "jr-native-enrollment-{}-{}",
            std::process::id(),
            pin
        ));
        std::fs::create_dir(&directory).unwrap();
        let key_path = directory.join("initial");
        write_private_new(
            &key_path,
            initial
                .to_openssh(ssh_key::LineEnding::LF)
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        let endpoint = Endpoint {
            address: address.ip().to_string(),
            port: address.port(),
            server_name: "localhost".into(),
            certificate_sha256: pin,
            key_path: key_path.clone(),
            resume_path: directory.join("resume"),
        };
        let effects = directory.join("effects");
        let script = format!(
            "printf executed >> '{}'",
            effects.to_string_lossy().replace('\'', "'\\''")
        );
        for attempt in 0..2 {
            assert_eq!(
                exec_endpoint_io(
                    &endpoint,
                    &script,
                    &mut &b""[..],
                    &mut Vec::new(),
                    &mut Vec::new()
                )
                .await
                .unwrap(),
                0
            );
            if attempt == 0 {
                std::fs::remove_file(&key_path).unwrap();
            }
        }
        assert_eq!(std::fs::read(&effects).unwrap(), b"executedexecuted");
        server.abort();
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn disconnect_cancels_running_command() {
        let key = key();
        let (address, pin, server) = loopback(vec![key.public_key().to_openssh().unwrap()]).await;
        let mut tls = TlsConnector::from(client_config(&pin).unwrap())
            .connect(
                ServerName::try_from("localhost").unwrap(),
                TcpStream::connect(&address).await.unwrap(),
            )
            .await
            .unwrap();
        let (_, challenge) = read_frame(&mut tls).await.unwrap();
        let exporter = tls
            .get_ref()
            .1
            .export_keying_material([0u8; 32], LABEL, None)
            .unwrap();
        let proof = prove(&key, &exporter, &challenge, "printf '%s\\n' $$; sleep 30").unwrap();
        write_frame(&mut tls, AUTH, &serde_json::to_vec(&proof).unwrap())
            .await
            .unwrap();
        assert_eq!(read_frame(&mut tls).await.unwrap().0, READY);
        write_frame(&mut tls, EOF, &[]).await.unwrap();
        let (kind, body) = read_frame(&mut tls).await.unwrap();
        assert_eq!(kind, STDOUT);
        let pid: i32 = String::from_utf8(body).unwrap().trim().parse().unwrap();
        assert!(pid > 1);
        drop(tls);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if unsafe { libc::kill(pid, 0) } == -1
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("disconnected command was not killed/reaped");
        server.abort();
    }
    #[tokio::test]
    async fn oversized_frame_rejected_before_body_allocation() {
        let mut bytes = vec![STDIN];
        bytes.extend_from_slice(&((MAX_FRAME + 1) as u32).to_be_bytes());
        assert!(read_frame(&mut bytes.as_slice()).await.is_err());
    }
    #[test]
    fn acceptance_proof_binds_nonce_pin_and_operation() {
        let key = key();
        let allowed = vec![key.public_key().to_openssh().unwrap()];
        let request = AcceptanceRequest {
            challenge: "a".repeat(64),
            certificate_sha256: "b".repeat(64),
        };
        let mut proof =
            prove_request(&key, &[1; 32], &[2; 32], "", None, Some(request.clone())).unwrap();
        verify(&proof, &[1; 32], &[2; 32], &allowed).unwrap();
        proof.acceptance.as_mut().unwrap().challenge = "c".repeat(64);
        assert!(verify(&proof, &[1; 32], &[2; 32], &allowed).is_err());
        proof.acceptance = Some(request.clone());
        proof.acceptance.as_mut().unwrap().certificate_sha256 = "d".repeat(64);
        assert!(verify(&proof, &[1; 32], &[2; 32], &allowed).is_err());
        proof.acceptance = None;
        assert!(verify(&proof, &[1; 32], &[2; 32], &allowed).is_err());
        let proof = prove_request(
            &key,
            &[1; 32],
            &[2; 32],
            "echo forbidden",
            None,
            Some(request),
        )
        .unwrap();
        assert!(verify(&proof, &[1; 32], &[2; 32], &allowed).is_err());
    }
    #[tokio::test]
    async fn tls_acceptance_returns_scoped_receipt_and_preserves_exec() {
        let key = key();
        let (address, pin, server) = loopback(vec![key.public_key().to_openssh().unwrap()]).await;
        let request = AcceptanceRequest {
            challenge: "a".repeat(64),
            certificate_sha256: pin.clone(),
        };
        let (_, receipt) =
            connect_request(&address, "localhost", &pin, &key, "", None, Some(request))
                .await
                .unwrap();
        let receipt = receipt.unwrap();
        assert_eq!(receipt["accepted"], true);
        assert_eq!(receipt["nonce"], "a".repeat(64));
        assert_eq!(receipt["certificate_sha256"], pin);
        assert_eq!(receipt["account_id"], "test");
        assert_eq!(receipt["lease_id"], "test-lease");
        let wrong_pin = AcceptanceRequest {
            challenge: "a".repeat(64),
            certificate_sha256: "00".repeat(32),
        };
        let error = connect_request(&address, "localhost", &pin, &key, "", None, Some(wrong_pin))
            .await
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<NativeAuthenticationRejected>()
                .is_none()
        );
        let mut output = Vec::new();
        assert_eq!(
            exec_key_io(
                &address,
                "localhost",
                &pin,
                &key,
                "printf unchanged",
                &mut &b""[..],
                &mut output,
                &mut Vec::new()
            )
            .await
            .unwrap(),
            0
        );
        assert_eq!(output, b"unchanged");
        server.abort();
    }
    #[tokio::test]
    async fn real_tls_acceptance_is_confirmed_with_supervisor_slot_credential() {
        use std::io::{Read, Write};
        let key = key();
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let pin = certificate_pin(cert.cert.der());
        let config =
            server_config(cert.cert.der().to_vec(), cert.signing_key.serialize_der()).unwrap();
        let api = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api_address = api.local_addr().unwrap();
        let public = key.public_key().to_openssh().unwrap();
        let expected_pin = pin.clone();
        let mock = std::thread::spawn(move || {
            for call in 0..2 {
                let (mut socket, _) = api.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0u8; 1];
                    socket.read_exact(&mut byte).unwrap();
                    header.push(byte[0]);
                    assert!(header.len() <= 16384);
                }
                let header = String::from_utf8(header).unwrap();
                let lower = header.to_ascii_lowercase();
                assert!(
                    lower.contains("\r\nauthorization: bearer root-only-supervisor-secret\r\n")
                );
                let body = if call == 0 {
                    assert!(
                        header.starts_with(
                            "GET /v1/build/pool/slot-keys?format=native-v1 HTTP/1.1\r\n"
                        )
                    );
                    serde_json::json!({"keys":[public],"account_id":"account-verified","lease_id":"lease-verified"})
                } else {
                    assert!(
                        header.starts_with("POST /v1/build/pool/native-acceptance HTTP/1.1\r\n")
                    );
                    let length: usize = lower
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    assert!(length <= 4096);
                    let mut bytes = vec![0; length];
                    socket.read_exact(&mut bytes).unwrap();
                    let request: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(
                        request,
                        serde_json::json!({"nonce":"a".repeat(64),"certificate_sha256":expected_pin})
                    );
                    serde_json::json!({"accepted":true})
                };
                let body = serde_json::to_vec(&body).unwrap();
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                socket.write_all(&body).unwrap();
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let source = KeySource::Slot {
            url: format!("http://{api_address}/v1/build/pool/slot-keys?format=native-v1"),
            token: "root-only-supervisor-secret".into(),
        };
        let server = tokio::spawn(serve_listener(
            listener,
            config,
            source,
            user(),
            pin.clone(),
        ));
        let request = AcceptanceRequest {
            challenge: "a".repeat(64),
            certificate_sha256: pin.clone(),
        };
        let (_, receipt) =
            connect_request(&address, "localhost", &pin, &key, "", None, Some(request))
                .await
                .unwrap();
        let receipt = receipt.unwrap();
        assert_eq!(receipt["account_id"], "account-verified");
        assert_eq!(receipt["lease_id"], "lease-verified");
        assert_eq!(receipt["accepted"], true);
        mock.join().unwrap();
        server.abort();
    }
}
