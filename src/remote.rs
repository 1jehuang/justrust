//! The remote compile machine: provisioning, power, and status.
//!
//! First step toward the remote compile service (docs/remote.md). justrust
//! owns one EC2 machine per user, created and controlled through the `aws`
//! CLI so we take no SDK dependency. The machine stops itself when idle, so
//! it costs nothing while you are not building.
//!
//! ```text
//! ~/.justrust/remote/
//!   state.json      instance id, region, type, ssh key, hourly price
//!   id_ed25519      ssh key used only for the remote machine
//!   probe.json      last ssh probe (load, cores, toolchain), cached for status
//! ```

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::remote_backend::Backend;

const NAME: &str = "justrust-remote";
const DEFAULT_REGION: &str = "us-west-2";
const DEFAULT_TYPE: &str = "c7i.8xlarge";
const DEFAULT_IDLE_MINUTES: u32 = 30;
// 200 GB filled up: Desktop's warm target dirs are 30 to 37 GB each (14 to
// 19 GB of it incremental), and benchmark stacks keep one per stack. gp3
// costs about $0.08/GB-month, so 500 GB is ~$40/month while the disk exists.
const DISK_GB: u32 = 500;
pub(crate) const SSH_USER: &str = "ubuntu";
/// How long a cached probe is reused by `status` before re-probing over ssh.
const PROBE_TTL: f64 = 30.0;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct State {
    pub instance_id: String,
    pub region: String,
    pub instance_type: String,
    pub spot: bool,
    pub idle_minutes: u32,
    pub created: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Probe {
    pub at: f64,
    pub ok: bool,
    pub rtt_ms: Option<f64>,
    pub cores: Option<u32>,
    pub load1: Option<f64>,
    pub mem_total_gb: Option<f64>,
    pub mem_used_gb: Option<f64>,
    pub disk_used_pct: Option<f64>,
    pub rustc: Option<String>,
    pub ready: bool,
    pub idle_left_min: Option<f64>,
    pub uptime_s: Option<f64>,
    /// Hottest plausible sensor in degrees C. EC2 VMs expose none.
    #[serde(default)]
    pub temp_c: Option<f64>,
    pub error: Option<String>,
    /// ssh destination probed (hosted), so a cached probe of another
    /// host is never shown.
    #[serde(default)]
    pub dest: Option<String>,
    /// EC2 instance metadata from the machine itself (IMDSv2), so every
    /// backend reports what it ran on.
    #[serde(default)]
    pub instance_type: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub spot: bool,
}

/// The local sync daemon, from its socket (no network).
#[derive(Serialize, Debug, Default, Clone, PartialEq)]
pub struct DaemonState {
    /// Holds a live session to the machine's agent.
    pub connected: bool,
    pub machine: String,
    /// Source roots it keeps mirrored.
    pub roots: Vec<String>,
    pub files_pushed: u64,
}

#[derive(Serialize, Debug, Default)]
pub struct Status {
    pub configured: bool,
    /// hosted or aws (legacy), or none.
    pub backend: String,
    /// hosted: the machine's address.
    pub host: Option<String>,
    pub instance_id: Option<String>,
    pub region: Option<String>,
    pub instance_type: Option<String>,
    pub spot: bool,
    /// EC2 state: pending, running, stopping, stopped, terminated, missing.
    pub state: String,
    pub public_ip: Option<String>,
    pub launch_time: Option<String>,
    pub running_s: Option<f64>,
    pub hourly_usd: Option<f64>,
    pub session_usd: Option<f64>,
    pub probe: Option<Probe>,
    pub error: Option<String>,
    /// None when the daemon is not running.
    pub daemon: Option<DaemonState>,
    /// JUSTRUST_REMOTE when set (auto, 0, 1).
    pub routing: Option<String>,
    /// hosted backend: account, credits, server-side host state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hosted: Option<crate::remote_hosted::HostedStatus>,
    /// A Jcode API key is present.
    pub signed_in: bool,
    /// `justrust login --no-wait` waiting for the user to open this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_login_url: Option<String>,
    /// Hosted builds left and the next step, from /me.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builds: Option<crate::remote_hosted::Builds>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_step: Option<String>,
}

pub(crate) fn dir() -> Result<PathBuf> {
    let d = crate::paths::home()?.join("remote");
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

fn key_path() -> Result<PathBuf> {
    Ok(dir()?.join("id_ed25519"))
}

fn load_state() -> Result<Option<State>> {
    let p = dir()?.join("state.json");
    match std::fs::read(&p) {
        Ok(b) => Ok(Some(serde_json::from_slice(&b).context("state.json")?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn save_state(s: &State) -> Result<()> {
    std::fs::write(dir()?.join("state.json"), serde_json::to_vec_pretty(s)?)?;
    Ok(())
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Run `aws ... --output json` in `region` and parse the result.
fn aws(region: &str, args: &[&str]) -> Result<serde_json::Value> {
    let out = Command::new("aws")
        .args(args)
        .args(["--region", region, "--output", "json"])
        .stdin(Stdio::null())
        .output()
        .context("running the aws CLI (is it installed?)")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("aws {}: {}", args.join(" "), err.trim());
    }
    if out.stdout.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(serde_json::Value::Null);
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

fn s(v: &serde_json::Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

struct Described {
    state: String,
    ip: Option<String>,
    launch: Option<String>,
    az: Option<String>,
}

fn describe(st: &State) -> Result<Described> {
    let v = aws(
        &st.region,
        &[
            "ec2",
            "describe-instances",
            "--instance-ids",
            &st.instance_id,
        ],
    );
    let v = match v {
        Ok(v) => v,
        Err(e) if e.to_string().contains("InvalidInstanceID") => {
            return Ok(Described {
                state: "missing".into(),
                ip: None,
                launch: None,
                az: None,
            });
        }
        Err(e) => return Err(e),
    };
    let i = &v["Reservations"][0]["Instances"][0];
    let d = Described {
        state: s(&i["State"]["Name"]).unwrap_or_else(|| "missing".into()),
        ip: s(&i["PublicIpAddress"]),
        launch: s(&i["LaunchTime"]),
        az: s(&i["Placement"]["AvailabilityZone"]),
    };
    remember_ip(&d);
    Ok(d)
}

/// Keep the cached IP (what routing and the sync daemon connect to) equal
/// to what AWS reports: the public IP changes on every start, and a stale
/// one made every remote call time out until something rewrote the file.
fn remember_ip(d: &Described) {
    let Ok(dir) = dir() else { return };
    let f = dir.join("ip");
    match (&d.ip, d.state.as_str()) {
        (Some(ip), "running") => {
            if std::fs::read_to_string(&f).ok().as_deref().map(str::trim) != Some(ip.as_str()) {
                let _ = std::fs::write(&f, ip);
            }
        }
        (_, "stopped" | "stopping" | "terminated" | "shutting-down" | "missing") => {
            let _ = std::fs::remove_file(&f);
        }
        _ => {}
    }
}

fn parse_time(t: &str) -> Option<f64> {
    chrono::DateTime::parse_from_rfc3339(t)
        .ok()
        .map(|d| d.timestamp() as f64)
}

/// Current price per hour: spot price in the instance's AZ, or a table of
/// on-demand prices. Cached for an hour.
fn hourly_price(st: &State, az: Option<&str>) -> Option<f64> {
    let cache = dir().ok()?.join("price.json");
    if let Ok(b) = std::fs::read(&cache)
        && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b)
        && now() - v["at"].as_f64().unwrap_or(0.0) < 3600.0
        && v["type"].as_str() == Some(&st.instance_type)
    {
        return v["usd"].as_f64();
    }
    let usd = if st.spot {
        let mut args = vec![
            "ec2",
            "describe-spot-price-history",
            "--instance-types",
            &st.instance_type,
            "--product-descriptions",
            "Linux/UNIX",
            "--max-items",
            "1",
        ];
        if let Some(az) = az {
            args.extend(["--availability-zone", az]);
        }
        aws(&st.region, &args).ok().and_then(|v| {
            v["SpotPriceHistory"][0]["SpotPrice"]
                .as_str()
                .and_then(|p| p.parse().ok())
        })
    } else {
        on_demand_price(&st.instance_type)
    }?;
    let _ = std::fs::write(
        &cache,
        serde_json::json!({"at": now(), "type": st.instance_type, "usd": usd}).to_string(),
    );
    Some(usd)
}

fn on_demand_price(t: &str) -> Option<f64> {
    // us-west-2 Linux on-demand, USD/hour.
    Some(match t {
        "c7i.4xlarge" => 0.714,
        "c7i.8xlarge" => 1.428,
        "c7i.16xlarge" => 2.856,
        "c7a.8xlarge" => 1.642,
        "c8i.8xlarge" => 1.499,
        _ => return None,
    })
}

/// Status of whichever backend is configured, plus the sync daemon.
/// `probe`: refresh a probe older than PROBE_TTL over ssh. The daemon part
/// never touches the network.
pub fn collect(probe: bool) -> Status {
    let mut out = match crate::remote_backend::load() {
        None => Status {
            state: "unconfigured".into(),
            backend: "none".into(),
            ..Default::default()
        },
        Some(Backend::Aws) => collect_aws(probe),
        Some(Backend::Hosted) => collect_hosted(probe),
    };
    out.daemon = daemon_state();
    out.signed_in = crate::remote_hosted::signed_in();
    out.pending_login_url = crate::account::load_pending().map(|p| p.url);
    out.builds = out.hosted.as_ref().and_then(|h| h.builds.clone());
    out.next_step = out
        .builds
        .as_ref()
        .and_then(|b| b.next_step.clone())
        .or_else(|| (!out.signed_in).then(|| "login".into()));
    out.routing = std::env::var("JUSTRUST_REMOTE")
        .ok()
        .filter(|v| !v.is_empty());
    out
}

fn daemon_state() -> Option<DaemonState> {
    let (roots, machine, files_pushed, connected) = crate::remote_daemon::status()?;
    Some(DaemonState {
        connected,
        machine,
        roots,
        files_pushed,
    })
}

fn collect_hosted(probe: bool) -> Status {
    let h = crate::remote_hosted::status_cached(probe);
    let mut out = Status {
        configured: true,
        backend: "hosted".into(),
        host: h.address.clone(),
        state: h
            .host_state
            .clone()
            .unwrap_or_else(|| if h.connected { "running" } else { "unknown" }.into()),
        error: h.error.clone(),
        ..Default::default()
    };
    if probe
        && h.connected
        && let Some(t) = crate::remote_hosted::cached_target()
    {
        let cached = cached_probe_file(HOSTED_PROBE)
            .filter(|p| p.dest.as_deref() == Some(&t.dest) && now() - p.at < PROBE_TTL);
        out.probe = Some(cached.unwrap_or_else(|| {
            let p = run_probe_ssh(&t);
            save_probe(HOSTED_PROBE, &p);
            p
        }));
    } else if probe {
        // Machine identity outlives the session: keep the last one seen.
        out.probe = cached_probe_file(HOSTED_PROBE).map(|mut p| {
            p.ok = false;
            p.error = Some("no session open (`justrust remote up`)".into());
            p
        });
    }
    if let Some(p) = &out.probe {
        out.instance_type = p.instance_type.clone();
        out.region = p.region.clone();
        out.spot = p.spot;
        out.public_ip = h.address.clone();
    }
    out.hosted = Some(h);
    out
}

fn collect_aws(probe: bool) -> Status {
    let mut out = Status {
        state: "unconfigured".into(),
        backend: "aws".into(),
        ..Default::default()
    };
    let st = match load_state() {
        Ok(Some(st)) => st,
        Ok(None) => return out,
        Err(e) => {
            out.state = "error".into();
            out.error = Some(e.to_string());
            return out;
        }
    };
    out.configured = true;
    out.instance_id = Some(st.instance_id.clone());
    out.region = Some(st.region.clone());
    out.instance_type = Some(st.instance_type.clone());
    out.spot = st.spot;
    let d = match describe(&st) {
        Ok(d) => d,
        Err(e) => {
            out.state = "error".into();
            out.error = Some(first_line(&e.to_string()));
            out.probe = cached_probe();
            return out;
        }
    };
    out.state = d.state.clone();
    out.public_ip = d.ip.clone();
    out.launch_time = d.launch.clone();
    out.hourly_usd = hourly_price(&st, d.az.as_deref());
    if d.state == "running" {
        out.running_s = d.launch.as_deref().and_then(parse_time).map(|t| now() - t);
        if let (Some(r), Some(p)) = (out.running_s, out.hourly_usd) {
            out.session_usd = Some(r / 3600.0 * p);
        }
        if let Some(ip) = &d.ip {
            out.probe = match cached_probe() {
                Some(p) if !probe || now() - p.at < PROBE_TTL => Some(p),
                _ if probe => {
                    let p = run_probe(ip);
                    // A timeout usually means our public IP changed (new
                    // network) and the security group no longer admits it.
                    if !p.ok
                        && p.error.as_deref().is_some_and(|e| e.contains("timed out"))
                        && ensure_security_group(&st.region).is_ok()
                    {
                        Some(run_probe(ip))
                    } else {
                        Some(p)
                    }
                }
                other => other,
            };
        }
    }
    out
}

fn first_line(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(160)
        .collect()
}

const AWS_PROBE: &str = "probe.json";
const HOSTED_PROBE: &str = "probe-hosted.json";

fn cached_probe() -> Option<Probe> {
    cached_probe_file(AWS_PROBE)
}

fn cached_probe_file(name: &str) -> Option<Probe> {
    let b = std::fs::read(dir().ok()?.join(name)).ok()?;
    serde_json::from_slice(&b).ok()
}

/// ssh options shared by every connection to the machine. ControlMaster
/// keeps one authenticated connection open for 10 minutes, so each later
/// ssh or rsync costs one round trip instead of a full handshake.
pub(crate) fn ssh_opts() -> Result<Vec<String>> {
    let d = dir()?;
    Ok(vec![
        "-i".into(),
        key_path()?.display().to_string(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        format!("UserKnownHostsFile={}", d.join("known_hosts").display()),
        "-o".into(),
        "ConnectTimeout=5".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "LogLevel=ERROR".into(),
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        format!("ControlPath={}", crate::remote_backend::control_path(&d)),
        "-o".into(),
        "ControlPersist=600".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
    ])
}

pub(crate) fn ssh_base(ip: &str) -> Result<Command> {
    let mut c = Command::new("ssh");
    c.args(ssh_opts()?).arg(format!("{SSH_USER}@{ip}"));
    Ok(c)
}

/// Last known IP of a running machine (no network). Cleared on stop.
pub(crate) fn cached_ip() -> Option<String> {
    let ip = std::fs::read_to_string(dir().ok()?.join("ip")).ok()?;
    let ip = ip.trim();
    (!ip.is_empty()).then(|| ip.to_string())
}

/// Append a line to ~/.justrust/remote/events.log.
pub(crate) fn log_event(msg: &str) {
    if let Ok(d) = dir() {
        let _ = crate::paths::append_line(
            &d.join("events.log"),
            &format!("{} {msg}", chrono::Local::now().format("%F %T")),
        );
    }
}

pub(crate) fn instance_id() -> Option<String> {
    load_state().ok().flatten().map(|s| s.instance_id)
}

pub(crate) fn describe_short() -> Option<String> {
    let st = load_state().ok().flatten()?;
    Some(format!("{} {}", st.instance_type, st.region))
}

/// The machine's IP, starting it first if it is stopped. Fast path: the
/// last known IP answers over the shared ssh connection (no aws call).
pub(crate) fn ensure_running() -> Result<String> {
    let ip_file = dir()?.join("ip");
    if let Ok(ip) = std::fs::read_to_string(&ip_file) {
        let ip = ip.trim().to_string();
        if !ip.is_empty() && ssh_ready(&ip) {
            return Ok(ip);
        }
    }
    let st = load_state()?.context("no remote machine yet: run `justrust remote up`")?;
    let mut d = describe(&st)?;
    if d.state != "running" || d.ip.is_none() {
        eprintln!("justrust remote: machine is {}, starting it...", d.state);
        up(UpOptions {
            region: None,
            instance_type: None,
            on_demand: false,
            idle_minutes: None,
        })?;
        d = describe(&st)?;
    }
    let ip = d.ip.context("machine has no public IP")?;
    // Wait for sshd and first-boot setup.
    for i in 0..100 {
        if ssh_ready(&ip) {
            std::fs::write(&ip_file, &ip)?;
            return Ok(ip);
        }
        if i == 0 {
            eprintln!("justrust remote: waiting for the machine to be ready...");
        }
        std::thread::sleep(Duration::from_secs(3));
    }
    bail!("remote machine at {ip} did not become ready")
}

fn ssh_ready(ip: &str) -> bool {
    ssh_base(ip)
        .and_then(|mut c| {
            Ok(c.arg("test -f /var/lib/justrust/ready")
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .status()?)
        })
        .map(|s| s.success())
        .unwrap_or(false)
}

fn tcp_rtt(ip: &str) -> Option<f64> {
    let addr: std::net::SocketAddr = format!("{ip}:22").parse().ok()?;
    let mut best: Option<f64> = None;
    for _ in 0..3 {
        let t = Instant::now();
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok() {
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            best = Some(best.map_or(ms, |b: f64| b.min(ms)));
        }
    }
    best
}

const PROBE_SCRIPT: &str = r#"
nproc
cut -d' ' -f1 /proc/loadavg
awk '/MemTotal/{t=$2}/MemAvailable/{a=$2}END{printf "%.2f %.2f\n", t/1048576, (t-a)/1048576}' /proc/meminfo
df --output=pcent / | tail -1 | tr -dc 0-9; echo
(~/.cargo/bin/rustc --version 2>/dev/null || rustc --version 2>/dev/null || echo none)
test -f /var/lib/justrust/ready -o -f /opt/jcode-build/ready && echo ready || echo setup
if [ -f /var/lib/justrust/idle-left ]; then cat /var/lib/justrust/idle-left
elif [ -x /opt/jcode-build/idle.sh ]; then s=$(cat /run/jcode-build-idle-since 2>/dev/null || date +%s); echo $(( (1800 - $(date +%s) + s) / 60 ))
else echo -; fi
cut -d' ' -f1 /proc/uptime
cat /sys/class/thermal/thermal_zone*/temp /sys/class/hwmon/hwmon*/temp*_input 2>/dev/null | awk '$1>1000&&$1<150000{if($1>m)m=$1}END{if(m)printf "%.1f\n",m/1000; else print "-"}'
t=$(curl -sf -m 1 -X PUT -H 'X-aws-ec2-metadata-token-ttl-seconds: 60' http://169.254.169.254/latest/api/token 2>/dev/null)
m() { [ -n "$t" ] && curl -sf -m 1 -H "X-aws-ec2-metadata-token: $t" "http://169.254.169.254/latest/meta-data/$1" 2>/dev/null; echo; }
m instance-type
m placement/region
m instance-life-cycle
"#;

fn run_probe(ip: &str) -> Probe {
    let mut p = Probe {
        at: now(),
        rtt_ms: tcp_rtt(ip),
        ..Default::default()
    };
    let res = ssh_base(ip).and_then(|mut c| {
        Ok(c.arg(PROBE_SCRIPT)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output()?)
    });
    fill_probe(&mut p, res);
    save_probe(AWS_PROBE, &p);
    p
}

/// Probe the hosted machine. The round trip is one `ssh true` over the
/// shared connection the probe just opened (one network round trip plus ssh
/// process startup).
fn run_probe_ssh(t: &crate::remote_backend::Target) -> Probe {
    let mut p = Probe {
        at: now(),
        dest: Some(t.dest.clone()),
        ..Default::default()
    };
    let res = t
        .command()
        .arg(PROBE_SCRIPT)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(anyhow::Error::from);
    fill_probe(&mut p, res);
    // The fleet hands out a session only once the host is ready.
    p.ready = p.ok;
    if p.ok {
        let t0 = Instant::now();
        let ok = t
            .command()
            .arg("true")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            p.rtt_ms = Some(t0.elapsed().as_secs_f64() * 1000.0);
        }
    }
    p
}

fn save_probe(name: &str, p: &Probe) {
    if let Ok(d) = dir() {
        let _ = std::fs::write(d.join(name), serde_json::to_vec(p).unwrap_or_default());
    }
}

fn fill_probe(p: &mut Probe, res: Result<std::process::Output>) {
    match res {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let l: Vec<&str> = text.lines().map(str::trim).collect();
            let get = |i: usize| l.get(i).copied().unwrap_or("");
            p.ok = true;
            p.cores = get(0).parse().ok();
            p.load1 = get(1).parse().ok();
            let mut m = get(2).split_whitespace();
            p.mem_total_gb = m.next().and_then(|x| x.parse().ok());
            p.mem_used_gb = m.next().and_then(|x| x.parse().ok());
            p.disk_used_pct = get(3).parse().ok();
            p.rustc = Some(get(4).to_string()).filter(|r| r != "none" && !r.is_empty());
            p.ready = get(5) == "ready";
            p.idle_left_min = get(6).parse().ok();
            p.uptime_s = get(7).parse().ok();
            p.temp_c = get(8).parse().ok();
            let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
            p.instance_type = some(get(9));
            p.region = some(get(10));
            p.spot = get(11) == "spot";
        }
        Ok(o) => p.error = Some(first_line(&String::from_utf8_lossy(&o.stderr))),
        Err(e) => p.error = Some(first_line(&e.to_string())),
    }
}

// ---------------------------------------------------------------- rendering

fn dur(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

pub fn render_text(s: &Status) -> String {
    let mut o = String::new();
    if let Some(u) = &s.pending_login_url {
        let _ = writeln!(
            o,
            "sign-in pending: ask your user to open this link, builds go remote once approved:\n  {u}"
        );
    }
    match s.backend.as_str() {
        "hosted" => render_hosted(&mut o, s),
        _ if !s.configured => {
            let _ = writeln!(o, "remote builds: not set up (every build runs locally)");
            let _ = writeln!(
                o,
                "  justrust login                      a Jcode subscription build machine"
            );
        }
        _ => render_aws_header(&mut o, s),
    }
    if let Some(p) = &s.probe
        && (s.state == "running" || s.backend == "hosted")
    {
        render_probe(&mut o, s, p);
    }
    if let Some(e) = &s.error {
        let _ = writeln!(o, "  error: {e}");
    }
    render_daemon(&mut o, s);
    o
}

fn render_hosted(o: &mut String, s: &Status) {
    let h = s.hosted.clone().unwrap_or_default();
    let _ = writeln!(
        o,
        "remote builds: hosted (Jcode subscription), machine {}{}",
        s.state,
        if h.connected { ", session open" } else { "" }
    );
    if let Some(m) = &h.me {
        let _ = writeln!(
            o,
            "  account: {} ({}{})",
            m.email.as_deref().unwrap_or("?"),
            m.tier.as_deref().unwrap_or("?"),
            m.status
                .as_deref()
                .map(|st| format!(", {st}"))
                .unwrap_or_default()
        );
    }
    if let Some(b) = &h.builds {
        let _ = writeln!(o, "  builds: {}", b.describe());
    } else if let Some(hrs) = h.hours_left {
        let _ = writeln!(o, "  credits: {hrs:.1} h of build-machine runtime left");
    }
    if let Some(a) = &s.host {
        let _ = writeln!(o, "  address: {a}");
    }
    if let Some(t) = &s.instance_type {
        let _ = writeln!(
            o,
            "  machine: {t} in {}{}",
            s.region.as_deref().unwrap_or("?"),
            if s.spot { " (spot)" } else { "" }
        );
    }
}

fn render_daemon(o: &mut String, s: &Status) {
    match &s.daemon {
        Some(d) => {
            let _ = writeln!(
                o,
                "sync daemon: {} to {}, {} root{} mirrored, {} file{} pushed",
                if d.connected { "connected" } else { "idle" },
                d.machine,
                d.roots.len(),
                if d.roots.len() == 1 { "" } else { "s" },
                d.files_pushed,
                if d.files_pushed == 1 { "" } else { "s" },
            );
            for r in &d.roots {
                let _ = writeln!(o, "  {r}");
            }
        }
        None if s.configured => {
            let _ = writeln!(
                o,
                "sync daemon: not running (starts with the next remote build)"
            );
        }
        None => {}
    }
    if let Some(r) = &s.routing {
        let _ = writeln!(o, "routing: JUSTRUST_REMOTE={r}");
    }
}

fn render_aws_header(o: &mut String, s: &Status) {
    let _ = writeln!(
        o,
        "remote compile machine: {}",
        match s.state.as_str() {
            "running" if s.probe.as_ref().is_some_and(|p| p.ok && !p.ready) =>
                "running (setting up)",
            other => other,
        }
    );
    if !s.configured {
        return;
    }
    let _ = writeln!(
        o,
        "  {} {} in {}{}",
        s.instance_id.as_deref().unwrap_or("?"),
        s.instance_type.as_deref().unwrap_or("?"),
        s.region.as_deref().unwrap_or("?"),
        if s.spot { " (spot)" } else { "" }
    );
    if let Some(ip) = &s.public_ip {
        let _ = writeln!(o, "  ip {ip}");
    }
    if let Some(p) = s.hourly_usd {
        let _ = write!(o, "  ${p:.3}/h");
        if let (Some(r), Some(c)) = (s.running_s, s.session_usd) {
            let _ = write!(o, ", up {} (${c:.2} this session)", dur(r));
        }
        let _ = writeln!(o);
    }
}

fn render_probe(o: &mut String, s: &Status, p: &Probe) {
    {
        if p.ok {
            if let Some(r) = p.rtt_ms {
                let _ = writeln!(o, "  round trip {r:.0} ms");
            }
            if let (Some(c), Some(l)) = (p.cores, p.load1) {
                let _ = writeln!(o, "  {c} cores, load {l:.2}");
            }
            if let (Some(u), Some(t)) = (p.mem_used_gb, p.mem_total_gb) {
                let _ = writeln!(o, "  memory {u:.1} / {t:.1} GiB");
            }
            if let Some(d) = p.disk_used_pct {
                let _ = writeln!(o, "  disk {d:.0}% used");
            }
            match p.temp_c {
                Some(t) => {
                    let _ = writeln!(o, "  temperature {t:.0}°C");
                }
                None => {
                    let _ = writeln!(o, "  temperature not exposed by the VM");
                }
            }
            // The AWS header already shows uptime from the launch time.
            if s.running_s.is_none()
                && let Some(u) = p.uptime_s
            {
                let _ = writeln!(o, "  up {}", dur(u));
            }
            let _ = writeln!(
                o,
                "  toolchain {}",
                p.rustc.as_deref().unwrap_or("not installed yet")
            );
            // idle-left can be stale right after a wake (written before the
            // last shutdown, before the first timer tick of this boot).
            if let Some(m) = p.idle_left_min.filter(|m| *m >= 0.0) {
                let _ = writeln!(o, "  stops itself in {m:.0} min if idle");
            }
            let _ = writeln!(o, "  probed {} ago", dur(now() - p.at));
        } else {
            let _ = writeln!(
                o,
                "  ssh not reachable yet: {}",
                p.error.as_deref().unwrap_or("?")
            );
        }
    }
}

/// Compact bar text: latency, temperature, cpu, ram, disk, uptime, probe age.
/// Temperature is left out when the machine exposes no sensor (EC2 VMs).
fn metrics_line(p: &Probe, running_s: Option<f64>, at: f64) -> String {
    let mut parts = Vec::new();
    if let Some(r) = p.rtt_ms {
        parts.push(format!("{r:.0}ms"));
    }
    if let Some(t) = p.temp_c {
        parts.push(format!("{t:.0}°C"));
    }
    if let (Some(l), Some(c)) = (p.load1, p.cores)
        && c > 0
    {
        parts.push(format!("cpu {:.0}%", l / c as f64 * 100.0));
    }
    if let (Some(u), Some(t)) = (p.mem_used_gb, p.mem_total_gb)
        && t > 0.0
    {
        parts.push(format!("ram {:.0}%", u / t * 100.0));
    }
    if let Some(d) = p.disk_used_pct {
        parts.push(format!("disk {d:.0}%"));
    }
    if let Some(r) = running_s.or(p.uptime_s) {
        parts.push(format!("up {}", dur(r)));
    }
    parts.push(format!("{} ago", dur(at - p.at)));
    parts.join(" · ")
}

pub fn render_waybar(s: &Status) -> String {
    let icon = "\u{f233}"; // server
    let (mut text, class) = if !s.configured {
        (String::new(), "unconfigured")
    } else if s.backend == "hosted" {
        let left = s
            .hosted
            .as_ref()
            .and_then(|h| match (&h.builds, h.hours_left) {
                (Some(b), _) => Some(format!(" {}b", b.left())),
                (None, Some(h)) => Some(format!(" {h:.0}h")),
                _ => None,
            })
            .unwrap_or_default();
        match (&s.probe, s.state.as_str()) {
            _ if s.error.is_some() => (format!("{icon} hosted ?"), "error"),
            (Some(p), _) if p.ok => (
                format!("{icon} hosted {}{left}", metrics_line(p, None, now())),
                busy_class(p),
            ),
            (_, "running") => (format!("{icon} hosted on{left}"), "running"),
            (_, "provisioning" | "starting" | "booting") => {
                (format!("{icon} hosted starting"), "pending")
            }
            _ => (format!("{icon} hosted off{left}"), "stopped"),
        }
    } else {
        match s.state.as_str() {
            "running" => match &s.probe {
                Some(p) if p.ok && !p.ready => (format!("{icon} setup"), "setup"),
                Some(p) if p.ok => {
                    let text = format!("{icon} {}", metrics_line(p, s.running_s, now()));
                    (text, busy_class(p))
                }
                _ => (format!("{icon} booting"), "pending"),
            },
            "pending" => (format!("{icon} starting"), "pending"),
            "stopping" => (format!("{icon} stopping"), "pending"),
            "stopped" => (format!("{icon} off"), "stopped"),
            "error" => (format!("{icon} ?"), "error"),
            other => (format!("{icon} {other}"), "error"),
        }
    };
    if !text.is_empty()
        && let Some(d) = &s.daemon
    {
        text.push_str(" · ");
        text.push_str(&sync_badge(d));
    }
    let tip = render_text(s);
    format!(
        "{{\"text\": {}, \"tooltip\": {}, \"class\": \"{class}\"}}",
        serde_json::to_string(&text).unwrap_or_default(),
        serde_json::to_string(tip.trim_end()).unwrap_or_default()
    )
}

fn busy_class(p: &Probe) -> &'static str {
    let busy = match (p.load1, p.cores) {
        (Some(l), Some(c)) if c > 0 => (l / c as f64 * 100.0).round(),
        _ => 0.0,
    };
    if busy >= 10.0 { "busy" } else { "running" }
}

/// Bar badge for the sync daemon: `sync 2r 14↑` while it holds a session
/// (roots mirrored, files pushed), `sync idle` when it has none.
fn sync_badge(d: &DaemonState) -> String {
    if d.connected {
        format!("sync {}r {}↑", d.roots.len(), d.files_pushed)
    } else {
        "sync idle".into()
    }
}

pub fn status(json: bool, waybar: bool, watch: Option<f64>) -> Result<()> {
    loop {
        let st = collect(true);
        let out = if waybar {
            render_waybar(&st)
        } else if json {
            serde_json::to_string(&st)?
        } else {
            render_text(&st)
        };
        if watch.is_some() && !waybar && !json {
            print!("\x1b[H\x1b[2J");
        }
        println!("{}", out.trim_end());
        use std::io::Write;
        let _ = std::io::stdout().flush();
        match watch {
            Some(secs) => std::thread::sleep(Duration::from_secs_f64(secs.max(1.0))),
            None => return Ok(()),
        }
    }
}

// ---------------------------------------------------------------- lifecycle

pub struct UpOptions {
    pub region: Option<String>,
    pub instance_type: Option<String>,
    pub on_demand: bool,
    pub idle_minutes: Option<u32>,
}

fn ensure_key() -> Result<String> {
    let key = key_path()?;
    if !key.exists() {
        let ok = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", NAME, "-f"])
            .arg(&key)
            .status()?
            .success();
        if !ok {
            bail!("ssh-keygen failed");
        }
    }
    Ok(std::fs::read_to_string(key.with_extension("pub"))?
        .trim()
        .to_string())
}

fn my_ip() -> Result<String> {
    let out = Command::new("curl")
        .args(["-fsS", "--max-time", "5", "https://checkip.amazonaws.com"])
        .output()
        .context("curl")?;
    let ip = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if ip.parse::<std::net::Ipv4Addr>().is_err() {
        bail!("could not determine this machine's public IP");
    }
    Ok(ip)
}

fn ensure_key_pair(region: &str, pubkey: &str) -> Result<()> {
    if aws(region, &["ec2", "describe-key-pairs", "--key-names", NAME]).is_ok() {
        return Ok(());
    }
    let file = dir()?.join("id_ed25519.pub");
    std::fs::write(&file, pubkey)?;
    aws(
        region,
        &[
            "ec2",
            "import-key-pair",
            "--key-name",
            NAME,
            "--public-key-material",
            &format!("fileb://{}", file.display()),
        ],
    )?;
    Ok(())
}

/// Security group allowing ssh only from this machine's current IP.
fn ensure_security_group(region: &str) -> Result<String> {
    let v = aws(
        region,
        &[
            "ec2",
            "describe-security-groups",
            "--filters",
            &format!("Name=group-name,Values={NAME}"),
        ],
    )?;
    let id = match s(&v["SecurityGroups"][0]["GroupId"]) {
        Some(id) => id,
        None => {
            let v = aws(
                region,
                &[
                    "ec2",
                    "create-security-group",
                    "--group-name",
                    NAME,
                    "--description",
                    "justrust remote compile machine, ssh from the owner IP only",
                ],
            )?;
            s(&v["GroupId"]).context("create-security-group returned no id")?
        }
    };
    allow_my_ip(region, &id)?;
    Ok(id)
}

fn allow_my_ip(region: &str, sg: &str) -> Result<()> {
    let ip = my_ip()?;
    let r = aws(
        region,
        &[
            "ec2",
            "authorize-security-group-ingress",
            "--group-id",
            sg,
            "--protocol",
            "tcp",
            "--port",
            "22",
            "--cidr",
            &format!("{ip}/32"),
        ],
    );
    match r {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains("Duplicate") => Ok(()),
        Err(e) => Err(e),
    }
}

/// Make `/bin/sh` bash on the build machine. Ubuntu's dash drops environment
/// variables whose names are not shell identifiers, such as cargo's
/// `CARGO_BIN_EXE_<bin-name>` for a bin with a `-`. A `#!/bin/sh` rustc
/// wrapper (Jcode Desktop's `scripts/rustc-parallel-frontend`) then hides it
/// from rustc and `env!("CARGO_BIN_EXE_jcode-desktop")` fails to compile on
/// the machine while it builds on the laptop, where /bin/sh is bash
/// (FINDINGS 41). Idempotent; run at provisioning and on every justrust
/// install, so existing machines get it too.
pub(crate) const SH_IS_BASH: &str = "if [ \"$(readlink -f /bin/sh)\" != /usr/bin/bash ] && [ -x /usr/bin/bash ]; then ln -sfn bash /bin/sh.justrust && mv -f /bin/sh.justrust /bin/sh; fi";

/// Cloud-init: build tools, rustup with the stable toolchain, mold, and an
/// idle watchdog that powers the machine off (stop, not terminate) after
/// `idle` minutes with no ssh session and low load.
fn user_data(idle: u32) -> String {
    format!(
        r#"#!/bin/bash
set -eux
mkdir -p /var/lib/justrust
cat >/usr/local/bin/justrust-idle-check <<'EOF'
#!/bin/bash
# Stops the machine after IDLE minutes without an interactive login or CPU
# load. Status probes are short non-tty ssh commands, so they do not count.
IDLE={idle}
f=/var/lib/justrust/last-active
[[ "$(cat $f 2>/dev/null)" =~ ^[0-9]+$ ]] || date +%s >$f
load=$(cut -d' ' -f1 /proc/loadavg)
if who | grep -q . || awk "BEGIN{{exit !($load > 1.0)}}"; then
  date +%s >$f
fi
left=$(( IDLE*60 - ($(date +%s) - $(cat $f)) ))
echo $(( left / 60 )) >/var/lib/justrust/idle-left
[ $left -le 0 ] && /sbin/shutdown -h now "justrust: idle"
exit 0
EOF
chmod +x /usr/local/bin/justrust-idle-check
cat >/etc/systemd/system/justrust-idle.service <<'EOF'
[Service]
Type=oneshot
ExecStart=/usr/local/bin/justrust-idle-check
EOF
cat >/etc/systemd/system/justrust-idle.timer <<'EOF'
[Timer]
OnBootSec=1min
OnUnitActiveSec=1min
[Install]
WantedBy=timers.target
EOF
cat >/etc/systemd/system/justrust-boot.service <<'EOF'
[Unit]
Before=justrust-idle.timer
[Service]
Type=oneshot
ExecStart=/bin/sh -c 'date +%%s >/var/lib/justrust/last-active; rm -f /var/lib/justrust/idle-left'
[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable --now justrust-boot.service justrust-idle.timer
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y build-essential clang lld mold pkg-config libssl-dev cmake git rsync \
  libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev libasound2-dev libfontconfig1-dev libfreetype-dev \
  libxcb1-dev libx11-dev libx11-xcb-dev libdbus-1-dev libegl-dev libgl-dev libvulkan-dev protobuf-compiler \
  poppler-utils xvfb
{SH_IS_BASH}
sudo -u {SSH_USER} -H bash -c 'curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable -c clippy,rustfmt'
touch /var/lib/justrust/ready
"#
    )
}

pub fn up(opts: UpOptions) -> Result<()> {
    if crate::remote_backend::load() == Some(Backend::Hosted) {
        return crate::remote_hosted::up();
    }
    if let Some(st) = load_state()? {
        let d = describe(&st)?;
        match d.state.as_str() {
            "running" | "pending" => {
                println!("{} is already {}", st.instance_id, d.state);
                return Ok(());
            }
            "stopped" | "stopping" => {
                // Our public IP may have changed since the last start.
                ensure_security_group(&st.region)?;
                if d.state == "stopping" {
                    println!("waiting for {} to finish stopping...", st.instance_id);
                    aws(
                        &st.region,
                        &[
                            "ec2",
                            "wait",
                            "instance-stopped",
                            "--instance-ids",
                            &st.instance_id,
                        ],
                    )?;
                }
                aws(
                    &st.region,
                    &["ec2", "start-instances", "--instance-ids", &st.instance_id],
                )?;
                println!("starting {}", st.instance_id);
                return wait_running(&st);
            }
            _ => {
                println!("{} is {}, creating a new machine", st.instance_id, d.state);
            }
        }
    }
    let region = opts.region.unwrap_or_else(|| DEFAULT_REGION.into());
    let itype = opts.instance_type.unwrap_or_else(|| DEFAULT_TYPE.into());
    let idle = opts.idle_minutes.unwrap_or(DEFAULT_IDLE_MINUTES);
    let pubkey = ensure_key()?;
    ensure_key_pair(&region, &pubkey)?;
    let sg = ensure_security_group(&region)?;
    let ami = aws(
        &region,
        &[
            "ssm",
            "get-parameter",
            "--name",
            "/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id",
        ],
    )?;
    let ami = s(&ami["Parameter"]["Value"]).context("no Ubuntu AMI id")?;
    let ud_file = dir()?.join("user-data.sh");
    std::fs::write(&ud_file, user_data(idle))?;
    let bdm = format!(
        "DeviceName=/dev/sda1,Ebs={{VolumeSize={DISK_GB},VolumeType=gp3,Iops=6000,Throughput=500,DeleteOnTermination=true}}"
    );
    let tags = format!(
        "ResourceType=instance,Tags=[{{Key=Name,Value={NAME}}},{{Key=justrust,Value=remote}}]"
    );
    let ud_arg = format!("file://{}", ud_file.display());
    let mut args = vec![
        "ec2",
        "run-instances",
        "--image-id",
        &ami,
        "--instance-type",
        &itype,
        "--key-name",
        NAME,
        "--security-group-ids",
        &sg,
        "--block-device-mappings",
        &bdm,
        "--tag-specifications",
        &tags,
        "--user-data",
        &ud_arg,
        "--instance-initiated-shutdown-behavior",
        "stop",
        "--metadata-options",
        "HttpTokens=required",
    ];
    let spot = !opts.on_demand;
    let market = "MarketType=spot,SpotOptions={SpotInstanceType=persistent,InstanceInterruptionBehavior=stop}";
    if spot {
        args.extend(["--instance-market-options", market]);
    }
    println!(
        "creating {itype}{} in {region}...",
        if spot { " (spot)" } else { "" }
    );
    let v = aws(&region, &args)?;
    let id = s(&v["Instances"][0]["InstanceId"]).context("run-instances returned no id")?;
    let st = State {
        instance_id: id.clone(),
        region,
        instance_type: itype,
        spot,
        idle_minutes: idle,
        created: chrono::Utc::now().to_rfc3339(),
    };
    save_state(&st)?;
    let _ = std::fs::remove_file(dir()?.join("probe.json"));
    let _ = std::fs::remove_file(dir()?.join("known_hosts"));
    println!("created {id}");
    wait_running(&st)
}

fn wait_running(st: &State) -> Result<()> {
    aws(
        &st.region,
        &[
            "ec2",
            "wait",
            "instance-running",
            "--instance-ids",
            &st.instance_id,
        ],
    )?;
    let d = describe(st)?;
    println!(
        "{} running at {}",
        st.instance_id,
        d.ip.as_deref().unwrap_or("?")
    );
    println!(
        "first boot installs the toolchain (a few minutes). Watch: justrust remote status --watch 5"
    );
    Ok(())
}

/// Explains why an aws-only command has nothing to do for this backend.
fn not_aws_note(cmd: &str) -> Option<String> {
    match crate::remote_backend::load()? {
        Backend::Aws => None,
        Backend::Hosted => Some(format!(
            "`remote {cmd}` applies to the AWS machine (`justrust remote up`). Remote builds use the hosted machine: `justrust remote down` stops it, and it is never destroyed from here"
        )),
    }
}

pub fn down() -> Result<()> {
    if crate::remote_backend::load() == Some(Backend::Hosted) {
        crate::remote_daemon::stop();
        return crate::remote_hosted::down();
    }
    let note = not_aws_note("down");
    let Some(st) = load_state()? else {
        match note {
            Some(n) => {
                println!("{n}");
                return Ok(());
            }
            None => bail!("no remote machine (justrust remote up)"),
        }
    };
    if let Some(n) = &note {
        println!("{n}");
    }
    aws(
        &st.region,
        &["ec2", "stop-instances", "--instance-ids", &st.instance_id],
    )?;
    println!("stopping {}", st.instance_id);
    // Routing must not think a stopped machine is up.
    let _ = std::fs::remove_file(dir()?.join("ip"));
    if note.is_none() {
        crate::remote_daemon::stop();
    }
    Ok(())
}

pub fn destroy() -> Result<()> {
    let note = not_aws_note("destroy");
    let Some(st) = load_state()? else {
        match note {
            Some(n) => {
                println!("{n}");
                return Ok(());
            }
            None => bail!("no remote machine"),
        }
    };
    if let Some(n) = &note {
        println!("{n}");
    }
    if st.spot {
        // A persistent spot request would relaunch the instance.
        let v = aws(
            &st.region,
            &[
                "ec2",
                "describe-instances",
                "--instance-ids",
                &st.instance_id,
            ],
        )?;
        if let Some(req) = s(&v["Reservations"][0]["Instances"][0]["SpotInstanceRequestId"]) {
            aws(
                &st.region,
                &[
                    "ec2",
                    "cancel-spot-instance-requests",
                    "--spot-instance-request-ids",
                    &req,
                ],
            )?;
        }
    }
    aws(
        &st.region,
        &[
            "ec2",
            "terminate-instances",
            "--instance-ids",
            &st.instance_id,
        ],
    )?;
    std::fs::remove_file(dir()?.join("state.json"))?;
    let _ = std::fs::remove_file(dir()?.join("probe.json"));
    let _ = std::fs::remove_file(dir()?.join("ip"));
    if note.is_none() {
        crate::remote_daemon::stop();
        let _ = crate::remote_backend::clear();
    }
    println!("terminated {}", st.instance_id);
    Ok(())
}

pub fn ssh(cmd: Vec<String>) -> Result<()> {
    let mut c = match crate::remote_backend::load() {
        Some(Backend::Hosted) => crate::remote_hosted::ensure_ready()?.command(),
        _ => aws_ssh_command()?,
    };
    if cmd.is_empty() {
        c.arg("-t");
    }
    c.args(cmd);
    let status = c.status()?;
    std::process::exit(status.code().unwrap_or(1));
}

fn aws_ssh_command() -> Result<Command> {
    let st = load_state()?
        .context("no remote machine: sign in to hosted builds with `justrust login`")?;
    let d = describe(&st)?;
    let ip = d.ip.with_context(|| {
        format!(
            "{} is {}, start it with justrust remote up",
            st.instance_id, d.state
        )
    })?;
    ssh_base(&ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waybar_unconfigured_is_empty() {
        let s = Status {
            state: "unconfigured".into(),
            ..Default::default()
        };
        let w = render_waybar(&s);
        assert!(w.contains("\"text\": \"\""));
        assert!(w.contains("unconfigured"));
    }

    #[test]
    fn waybar_running_shows_load_and_rtt() {
        let s = Status {
            configured: true,
            state: "running".into(),
            probe: Some(Probe {
                ok: true,
                ready: true,
                cores: Some(32),
                load1: Some(16.0),
                rtt_ms: Some(42.0),
                ..Default::default()
            }),
            ..Default::default()
        };
        let w = render_waybar(&s);
        assert!(w.contains("42ms") && w.contains("cpu 50%"), "{w}");
        assert!(w.contains("\"busy\""));
    }

    #[test]
    fn metrics_line_shows_every_field() {
        let p = Probe {
            at: 100.0,
            ok: true,
            rtt_ms: Some(29.0),
            temp_c: Some(54.4),
            cores: Some(32),
            load1: Some(8.0),
            mem_total_gb: Some(64.0),
            mem_used_gb: Some(16.0),
            disk_used_pct: Some(7.0),
            ..Default::default()
        };
        assert_eq!(
            metrics_line(&p, Some(2280.0), 118.0),
            "29ms · 54°C · cpu 25% · ram 25% · disk 7% · up 38m · 18s ago"
        );
        let no_temp = Probe { temp_c: None, ..p };
        assert!(!metrics_line(&no_temp, None, 118.0).contains('°'));
    }

    #[test]
    fn waybar_stopped() {
        let s = Status {
            configured: true,
            state: "stopped".into(),
            ..Default::default()
        };
        assert!(render_waybar(&s).contains(" off"));
    }

    fn ok_probe() -> Probe {
        Probe {
            at: now(),
            ok: true,
            ready: true,
            rtt_ms: Some(12.0),
            cores: Some(16),
            load1: Some(0.5),
            rustc: Some("rustc 1.90.0".into()),
            uptime_s: Some(7200.0),
            ..Default::default()
        }
    }

    fn daemon(connected: bool) -> DaemonState {
        DaemonState {
            connected,
            machine: "box".into(),
            roots: vec!["/home/me/a".into(), "/home/me/b".into()],
            files_pushed: 14,
        }
    }

    #[test]
    fn text_unconfigured_explains_setup() {
        let s = Status {
            backend: "none".into(),
            state: "unconfigured".into(),
            ..Default::default()
        };
        let t = render_text(&s);
        assert!(t.contains("not set up"), "{t}");
        assert!(t.contains("justrust login"), "{t}");
        assert!(!t.contains("ssh"), "{t}");
        assert!(!t.contains("sync daemon"), "{t}");
    }

    #[test]
    fn text_hosted_shows_account_and_credits() {
        let s = Status {
            configured: true,
            backend: "hosted".into(),
            state: "running".into(),
            host: Some("203.0.113.7".into()),
            hosted: Some(crate::remote_hosted::HostedStatus {
                me: Some(crate::remote_hosted::Me {
                    email: Some("a@b.c".into()),
                    tier: Some("pro".into()),
                    status: Some("active".into()),
                    build_hosts: true,
                    builds: None,
                }),
                hours_left: Some(12.5),
                connected: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let t = render_text(&s);
        assert!(
            t.contains("hosted (Jcode subscription), machine running, session open"),
            "{t}"
        );
        assert!(t.contains("a@b.c (pro, active)"), "{t}");
        assert!(t.contains("12.5 h"), "{t}");
        assert!(t.contains("sync daemon: not running"), "{t}");
        assert!(render_waybar(&s).contains("hosted on 12h"));
        let e = Status {
            error: Some(crate::remote_hosted::SIGNED_OUT.into()),
            ..s
        };
        assert!(render_text(&e).contains("error: not signed in"));
        assert!(render_waybar(&e).contains("hosted ?"));
    }

    #[test]
    fn hosted_probe_waybar_shows_metrics_builds_and_sync_badge() {
        let s = Status {
            configured: true,
            backend: "hosted".into(),
            state: "running".into(),
            probe: Some(ok_probe()),
            daemon: Some(daemon(true)),
            hosted: Some(crate::remote_hosted::HostedStatus {
                builds: Some(crate::remote_hosted::Builds {
                    included: Some(5000),
                    included_used: Some(10),
                    can_build: true,
                    ..Default::default()
                }),
                connected: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let v: serde_json::Value = serde_json::from_str(&render_waybar(&s)).unwrap();
        let text = v["text"].as_str().unwrap();
        assert!(text.contains("hosted 12ms"), "{text}");
        assert!(text.contains("4990b"), "{text}");
        assert!(text.ends_with("sync 2r 14↑"), "{text}");
        assert_eq!(v["class"], "running");
        let t = render_text(&s);
        assert!(t.contains("round trip 12 ms"), "{t}");
        assert!(t.contains("16 cores, load 0.50"), "{t}");
        assert!(t.contains("up 2h00m"), "{t}");
        assert!(t.contains("sync daemon: connected to box"), "{t}");
        let s = Status {
            instance_type: Some("c7i.8xlarge".into()),
            region: Some("us-west-2".into()),
            ..s
        };
        assert!(render_text(&s).contains("machine: c7i.8xlarge in us-west-2\n"));
    }

    #[test]
    fn probe_parses_instance_metadata() {
        let out = "32\n1.5\n61.77 2.00\n55\nrustc 1.90.0\nready\n29\n100.5\n-\nc7i.8xlarge\nus-west-2\nspot\n";
        let mut p = Probe::default();
        fill_probe(
            &mut p,
            Ok(std::process::Output {
                status: std::process::ExitStatus::default(),
                stdout: out.into(),
                stderr: vec![],
            }),
        );
        assert!(p.ok && p.ready);
        assert_eq!(p.idle_left_min, Some(29.0));
        assert_eq!(p.instance_type.as_deref(), Some("c7i.8xlarge"));
        assert_eq!(p.region.as_deref(), Some("us-west-2"));
        assert!(p.spot);
        // No metadata service (not EC2): empty lines.
        let mut q = Probe::default();
        fill_probe(
            &mut q,
            Ok(std::process::Output {
                status: std::process::ExitStatus::default(),
                stdout: "4\n0.1\n8 1\n10\nnone\nsetup\n-\n5\n-\n\n\n\n".into(),
                stderr: vec![],
            }),
        );
        assert_eq!(q.instance_type, None);
        assert!(!q.spot && !q.ready);
    }

    #[test]
    fn text_aws_keeps_machine_details_and_daemon() {
        let s = Status {
            configured: true,
            backend: "aws".into(),
            state: "stopped".into(),
            instance_id: Some("i-123".into()),
            instance_type: Some("c7i.8xlarge".into()),
            region: Some("us-west-2".into()),
            spot: true,
            daemon: Some(daemon(false)),
            routing: Some("auto".into()),
            ..Default::default()
        };
        let t = render_text(&s);
        assert!(t.contains("remote compile machine: stopped"), "{t}");
        assert!(t.contains("i-123 c7i.8xlarge in us-west-2 (spot)"), "{t}");
        assert!(t.contains("sync daemon: idle to box"), "{t}");
        assert!(t.contains("routing: JUSTRUST_REMOTE=auto"), "{t}");
    }

    #[test]
    fn waybar_unconfigured_ignores_daemon() {
        let s = Status {
            backend: "none".into(),
            state: "unconfigured".into(),
            daemon: Some(daemon(true)),
            ..Default::default()
        };
        let v: serde_json::Value = serde_json::from_str(&render_waybar(&s)).unwrap();
        assert_eq!(v["text"], "");
    }

    #[test]
    fn user_data_has_idle_minutes() {
        assert!(user_data(17).contains("IDLE=17"));
    }

    /// systemd expands `%s` in unit files to the user's shell.
    #[test]
    fn unit_files_escape_percent() {
        let ud = user_data(30);
        let unit = ud
            .split("justrust-boot.service <<'EOF'")
            .nth(1)
            .and_then(|r| r.split("\nEOF").next())
            .unwrap();
        assert!(unit.contains("%%s") && !unit.replace("%%", "").contains('%'));
    }

    /// The boot unit must clear the previous boot's idle countdown.
    #[test]
    fn boot_unit_resets_idle_left() {
        let ud = user_data(30);
        let unit = ud
            .split("justrust-boot.service <<'EOF'")
            .nth(1)
            .and_then(|r| r.split("\nEOF").next())
            .unwrap();
        assert!(unit.contains("rm -f /var/lib/justrust/idle-left"), "{unit}");
    }
}
