use super::*;

fn pinned_executable() -> PinnedSessionExecutable {
    let sha256 = "a".repeat(64);
    PinnedSessionExecutable {
        path: format!("/home/test/.justrust/remote/session-executables/{sha256}/justrust"),
        sha256,
    }
}

#[test]
fn pinned_launcher_rejects_changed_executable_path_or_digest_before_connecting() {
    let target = Target {
        dest: "not-contacted".into(),
        opts: Vec::new(),
    };
    let mut pin = pinned_executable();
    pin.sha256 = "b".repeat(64);
    assert!(command_with_executable(&target, "", &pin).is_err());
    pin = pinned_executable();
    pin.path = "/home/test/.cargo/bin/justrust".into();
    assert!(command_with_executable(&target, "", &pin).is_err());
}

#[test]
fn hosted_transport_defaults_on_with_strict_explicit_rollback_policy() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    assert!(enabled_value(None));
    assert!(enabled_value(Some(OsStr::new("1"))));
    for value in ["0", "", "true", "false", "01", " 1", "1 ", "unexpected"] {
        assert!(!enabled_value(Some(OsStr::new(value))), "{value:?}");
    }
    assert!(!enabled_value(Some(OsStr::from_bytes(&[0xff]))));
}

#[test]
fn binary_hello_is_flushed_before_waiting_for_more_transport_input() {
    let hello = encode(&Msg::Hello { version: 1 }, &[]).unwrap();
    assert!(!hello.contains(&b'\n'));
    let mut buffered = std::io::BufWriter::new(Vec::new());
    publish(&mut buffered, &hello).unwrap();
    assert_eq!(buffered.get_ref(), &hello);

    struct FlushGate {
        flushed: Arc<AtomicBool>,
        bytes: Vec<u8>,
    }
    impl Write for FlushGate {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.flushed.store(true, Ordering::SeqCst);
            Ok(())
        }
    }
    struct AwaitingPeer {
        flushed: Arc<AtomicBool>,
        hello: Option<Vec<u8>>,
    }
    impl Read for AwaitingPeer {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            if let Some(hello) = self.hello.take() {
                bytes[..hello.len()].copy_from_slice(&hello);
                return Ok(hello.len());
            }
            assert!(
                self.flushed.load(Ordering::SeqCst),
                "peer awaits Hello before producing another packet"
            );
            Ok(0)
        }
    }
    let flushed = Arc::new(AtomicBool::new(false));
    let mut peer = AwaitingPeer {
        flushed: flushed.clone(),
        hello: Some(hello.clone()),
    };
    let mut output = std::io::BufWriter::new(FlushGate {
        flushed,
        bytes: Vec::new(),
    });
    copy_live(&mut peer, &mut output).unwrap();
    assert_eq!(output.get_ref().bytes, hello);

    struct FailedFlush;
    impl Write for FailedFlush {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
    }
    assert_eq!(
        publish(&mut FailedFlush, &hello).unwrap_err().kind(),
        std::io::ErrorKind::BrokenPipe
    );
}

#[test]
fn exact_prefix_is_preserved_in_agent_bridge_and_identity() {
    let prefix = "/private/mirror with 'quotes'/../exact";
    let target = Target {
        dest: "host".into(),
        opts: Vec::new(),
    };
    let executable = pinned_executable();
    let command = command_with_executable(&target, prefix, &executable).unwrap();
    let args: Vec<_> = command.get_args().collect();
    let config: serde_json::Value = serde_json::from_str(args[2].to_str().unwrap()).unwrap();
    assert_eq!(config["prefix"], prefix);
    assert_eq!(
        config["executable"],
        serde_json::to_value(&executable).unwrap()
    );
    let agent = agent_command(prefix).unwrap();
    let args: Vec<_> = agent.get_args().map(|arg| arg.to_str().unwrap()).collect();
    assert_eq!(args, ["__agent", "--prefix", prefix]);
    let default_agent = agent_command("").unwrap();
    assert_eq!(default_agent.get_args().collect::<Vec<_>>(), ["__agent"]);
    for mode in ["create", "attach"] {
        let script = session_script(mode, &"a".repeat(64), prefix, &executable);
        assert!(script.starts_with("export PATH=\"$HOME/.cargo/bin:/opt/homebrew/bin:$HOME/.homebrew/bin:/usr/local/bin:$PATH\"; exec "));
        assert!(script.contains(&crate::remote_build::shell_quote(&executable.path)));
        assert!(!script.contains("exec ~/.cargo/bin/justrust"));
        assert!(script.ends_with(&format!(" {}", crate::remote_build::shell_quote(prefix))));
    }
    assert!(command_prefix_rejected(&target, "relative/mirror"));
    assert!(command_prefix_rejected(&target, "/mirror\0other"));
    let mut f = Fixture::new();
    Arc::get_mut(&mut f.broker).unwrap().identity.prefix = prefix.into();
    let serialized = serde_json::to_vec(&f.broker.identity).unwrap();
    let identity: Identity = serde_json::from_slice(&serialized).unwrap();
    assert_eq!(identity.prefix, prefix);
    let (mut stream, worker) = f.attach(None, 0);
    assert!(
        matches!(receive(&mut stream).unwrap().0, Packet::Ready { identity, .. } if identity.prefix == prefix)
    );
    drop(stream);
    worker.join().unwrap().unwrap();
    let (mut stream, worker) = f.attach_prefix(None, 0, "/different/mirror");
    assert!(matches!(
        receive(&mut stream).unwrap().0,
        Packet::Fatal { .. }
    ));
    drop(stream);
    assert!(worker.join().unwrap().is_err());
}

fn command_prefix_rejected(target: &Target, prefix: &str) -> bool {
    command(target, prefix).is_err() && agent_command(prefix).is_err()
}

struct Fixture {
    broker: Arc<Broker>,
    child: Child,
    output: std::process::ChildStdout,
    journal: File,
}

impl Fixture {
    fn new() -> Self {
        Self::with_command(Command::new("cat"))
    }

    fn with_command(mut command: Command) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "jr-session-test-{}-{}",
            std::process::id(),
            token().unwrap()
        ));
        std::fs::create_dir(&dir).unwrap();
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let output = child.stdout.take().unwrap();
        let input = child.stdin.take().unwrap();
        nonblocking(&input).unwrap();
        let journal = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(dir.join("output"))
            .unwrap();
        let broker = Arc::new(Broker {
            identity: Identity {
                version: 1,
                session: "a".repeat(64),
                incarnation: "instance-1".into(),
                machine: "host".into(),
                boot: "boot-1".into(),
                binary: "binary-1".into(),
                prefix: String::new(),
                executable: pinned_executable(),
            },
            dir,
            input: Mutex::new(Some(input)),
            state: Mutex::new(BrokerState {
                accepted: 0,
                length: 0,
                jobs: BTreeMap::new(),
                fatal: None,
                done: false,
                inputs: BTreeMap::new(),
            }),
            changed: Condvar::new(),
        });
        Self {
            broker,
            child,
            output,
            journal,
        }
    }

    fn attach(
        &self,
        identity: Option<Identity>,
        offset: u64,
    ) -> (UnixStream, std::thread::JoinHandle<Result<bool>>) {
        self.attach_prefix(identity, offset, &self.broker.identity.prefix)
    }

    fn attach_prefix(
        &self,
        identity: Option<Identity>,
        offset: u64,
        prefix: &str,
    ) -> (UnixStream, std::thread::JoinHandle<Result<bool>>) {
        let (mut client, server) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let broker = self.broker.clone();
        let handle = std::thread::spawn(move || attachment(&broker, server));
        send(
            &mut client,
            &Packet::Attach {
                session: self.broker.identity.session.clone(),
                prefix: prefix.into(),
                identity,
                offset,
            },
            &[],
        )
        .unwrap();
        (client, handle)
    }

    fn output(&mut self, msg: &Msg, data: &[u8]) -> Vec<u8> {
        self.broker.output(msg, data, &mut self.journal).unwrap();
        encode(msg, data).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.broker.dir);
    }
}

fn invocation(source: &str) -> Msg {
    Msg::RunSync {
        id: 7,
        cwd: "/workspace".into(),
        args: vec!["test".into()],
        env: vec![
            (SOURCE_ID.into(), source.into()),
            ("RUSTFLAGS".into(), "--cfg same_invocation".into()),
        ],
        roots: vec![],
        start: false,
        run_id: "immutable-run-7".into(),
    }
}

#[test]
fn transient_disconnect_reattaches_original_job_and_replays_only_missing_output() {
    let mut f = Fixture::new();
    let first = f.output(
        &Msg::Out { id: 7, stream: 2 },
        b"diagnostic before disconnect\n",
    );
    let run = encode(&invocation("manifest-1"), &[]).unwrap();
    let (mut stream, attachment) = f.attach(None, 0);
    let (
        Packet::Ready {
            identity,
            accepted: 0,
        },
        _,
    ) = receive(&mut stream).unwrap()
    else {
        panic!("no initial receipt");
    };
    send(&mut stream, &Packet::Input { seq: 1 }, &run).unwrap();
    let mut delivered = Vec::new();
    let mut acknowledged = false;
    while delivered.len() < first.len() || !acknowledged {
        match receive(&mut stream).unwrap() {
            (Packet::Output { offset: 0 }, bytes) => delivered.extend(bytes),
            (Packet::Ack { seq: 1 }, _) => acknowledged = true,
            other => panic!("unexpected packet: {other:?}"),
        }
    }
    assert_eq!(delivered, first);
    drop(stream);
    assert!(!attachment.join().unwrap().unwrap());
    // The original agent and invocation remain owned, not restarted.
    assert!(f.child.try_wait().unwrap().is_none());
    let second = f.output(
        &Msg::Out { id: 7, stream: 2 },
        b"test failed after disconnect\n",
    );
    let exit = f.output(
        &Msg::Exit {
            id: 7,
            code: 101,
            timing: None,
        },
        b"{}",
    );
    let (mut stream, attachment) = f.attach(Some(identity), first.len() as u64);
    assert!(matches!(
        receive(&mut stream).unwrap().0,
        Packet::Ready { accepted: 1, .. }
    ));
    // Simulate an ACK lost with the old connection. Even retransmission of
    // the complete Run frame must not reach the agent twice.
    send(&mut stream, &Packet::Input { seq: 1 }, &run).unwrap();
    send(
        &mut stream,
        &Packet::Input { seq: 2 },
        &encode(&Msg::Ping, &[]).unwrap(),
    )
    .unwrap();
    let mut rest = Vec::new();
    while rest.len() < second.len() + exit.len() {
        match receive(&mut stream).unwrap() {
            (Packet::Output { offset }, bytes) => {
                assert_eq!(offset, (first.len() + rest.len()) as u64);
                rest.extend(bytes);
            }
            (Packet::Ack { .. }, _) => {}
            other => panic!("unexpected packet: {other:?}"),
        }
    }
    assert_eq!(rest, [second, exit].concat());
    let (forwarded, _) = read_frame(&mut f.output).unwrap().unwrap();
    let Msg::RunSync { env, run_id, .. } = forwarded else {
        panic!("original run missing");
    };
    assert_eq!(run_id, "immutable-run-7");
    assert!(!env.iter().any(|(key, _)| key == SOURCE_ID));
    assert_eq!(read_frame(&mut f.output).unwrap().unwrap().0, Msg::Ping);
    let receipt: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.broker.dir.join("job-7.json")).unwrap()).unwrap();
    assert_eq!(receipt["state"], "finished");
    assert_eq!(receipt["exit"], 101);
    assert_eq!(receipt["source"], "manifest-1");
    assert_eq!(f.broker.state.lock().unwrap().jobs.len(), 1);
    drop(stream);
    attachment.join().unwrap().unwrap();
}

#[test]
fn changed_machine_boot_binary_or_session_and_future_cursor_are_refused() {
    let f = Fixture::new();
    for field in [
        "boot",
        "machine",
        "binary",
        "session",
        "incarnation",
        "prefix",
        "executable-path",
        "executable-digest",
    ] {
        let mut identity = f.broker.identity.clone();
        match field {
            "boot" => identity.boot = "boot-2".into(),
            "machine" => identity.machine = "another-host".into(),
            "binary" => identity.binary = "binary-2".into(),
            "incarnation" => identity.incarnation = "instance-2".into(),
            "prefix" => identity.prefix = "/another/mirror".into(),
            "executable-path" => identity.executable.path = "/another/executable".into(),
            "executable-digest" => identity.executable.sha256 = "b".repeat(64),
            _ => identity.session = "b".repeat(64),
        }
        let (mut stream, worker) = f.attach(Some(identity), 0);
        assert!(matches!(
            receive(&mut stream).unwrap().0,
            Packet::Fatal { .. }
        ));
        drop(stream);
        assert!(worker.join().unwrap().is_err());
    }
    let (mut stream, worker) = f.attach(Some(f.broker.identity.clone()), 1);
    assert!(matches!(
        receive(&mut stream).unwrap().0,
        Packet::Fatal { .. }
    ));
    drop(stream);
    assert!(worker.join().unwrap().is_err());
    assert!(f.broker.state.lock().unwrap().jobs.is_empty());
}

#[test]
fn accepted_sequence_cannot_change_invocation_or_source_binding() {
    let mut f = Fixture::new();
    let original = encode(&invocation("manifest-1"), &[]).unwrap();
    assert_eq!(f.broker.input(1, &original).unwrap(), 1);
    assert_eq!(f.broker.input(1, &original).unwrap(), 1);
    assert!(
        f.broker
            .input(1, &encode(&invocation("manifest-2"), &[]).unwrap())
            .is_err()
    );
    assert!(
        f.broker.input(2, &original).is_err(),
        "same job ID must not be resubmitted under a fresh sequence"
    );
    f.broker
        .input(2, &encode(&Msg::Ping, &[]).unwrap())
        .unwrap();
    assert!(matches!(
        read_frame(&mut f.output).unwrap().unwrap().0,
        Msg::RunSync { .. }
    ));
    assert_eq!(read_frame(&mut f.output).unwrap().unwrap().0, Msg::Ping);
}

#[test]
fn recovery_is_transport_only_and_exhausts_attempt_and_wall_budgets() {
    let now = Instant::now();
    let error = anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
    let mut budget = RecoveryBudget::default();
    for _ in 0..RECONNECTS {
        assert!(budget.retry(&error, now));
    }
    assert!(!budget.retry(&error, now));
    let mut budget = RecoveryBudget::default();
    assert!(budget.retry(&error, now));
    assert!(!budget.retry(&error, now + GRACE));
    for reason in [
        "compiler failed",
        "test failed: exit 101",
        "identity changed",
        "original session is gone (lostjob)",
    ] {
        assert!(!RecoveryBudget::default().retry(&anyhow::anyhow!(reason), now));
    }
    assert!(!retryable(&anyhow::Error::from(std::io::Error::from(
        std::io::ErrorKind::PermissionDenied
    ))));
}

#[test]
fn explicit_cancel_reaches_original_agent_without_new_run() {
    let mut f = Fixture::new();
    f.broker
        .input(1, &encode(&invocation("manifest"), &[]).unwrap())
        .unwrap();
    f.broker.cancel(Instant::now() + Duration::from_secs(5));
    assert!(matches!(
        read_frame(&mut f.output).unwrap().unwrap().0,
        Msg::RunSync { id: 7, .. }
    ));
    assert_eq!(
        read_frame(&mut f.output).unwrap().unwrap().0,
        Msg::Cancel { id: 7 }
    );
    assert_eq!(f.broker.state.lock().unwrap().jobs.len(), 1);
}

#[test]
fn never_reading_agent_with_full_pipe_is_killed_and_reaped_within_grace_bound() {
    let mut command = Command::new("sleep");
    command.arg("60");
    let mut f = Fixture::with_command(command);
    f.broker
        .input(1, &encode(&invocation("manifest"), &[]).unwrap())
        .unwrap();
    let fd = f.broker.input.lock().unwrap().as_ref().unwrap().as_raw_fd();
    let broker = f.broker.clone();
    let sender = std::thread::spawn(move || {
        let bytes = encode(
            &Msg::Write {
                path: "/workspace/large.rs".into(),
                mode: 0o644,
            },
            &vec![b'x'; 1 << 20],
        )
        .unwrap();
        broker.input(2, &bytes)
    });
    // Prove saturation rather than hoping a small write blocks on this OS.
    let ready_by = Instant::now() + Duration::from_secs(2);
    loop {
        let mut poll = libc::pollfd {
            fd,
            events: libc::POLLOUT,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut poll, 1, 0) };
        if ready == 0 && f.broker.input.try_lock().is_err() {
            break;
        }
        assert!(Instant::now() < ready_by, "fixture did not fill its pipe");
        std::thread::sleep(Duration::from_millis(5));
    }
    let started = Instant::now();
    terminate(&f.broker, &mut f.child, Duration::from_millis(200));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancellation waited on a blocked input mutex"
    );
    assert!(
        f.child.try_wait().unwrap().is_some(),
        "original agent was not reaped"
    );
    assert!(
        sender.join().unwrap().is_err(),
        "partial frame must remain ambiguous, not acknowledged"
    );
    assert_eq!(f.broker.state.lock().unwrap().accepted, 1);
    assert!(
        f.broker.input.try_lock().is_ok(),
        "blocked sender retained the pipe lock"
    );
}

#[test]
fn input_acknowledgements_release_only_accepted_bytes() {
    let mut pending = Pending {
        frames: VecDeque::from([(1, vec![1, 2]), (2, vec![3])]),
        next: 3,
        bytes: 3,
        closed: false,
        error: None,
    };
    acknowledge(&mut pending, 1).unwrap();
    assert_eq!(pending.bytes, 1);
    assert_eq!(pending.frames.front().unwrap().0, 2);
    assert!(acknowledge(&mut pending, 3).is_err());
    acknowledge(&mut pending, 2).unwrap();
    assert_eq!(pending.bytes, 0);
}

#[test]
fn journal_exhaustion_is_explicit_and_never_truncates_existing_diagnostics() {
    let mut f = Fixture::new();
    let diagnostic = f.output(&Msg::Out { id: 7, stream: 2 }, b"full diagnostic\n");
    f.broker.state.lock().unwrap().length = MAX_JOURNAL;
    assert!(
        f.broker
            .output(&Msg::Out { id: 7, stream: 2 }, b"more", &mut f.journal)
            .is_err()
    );
    assert_eq!(
        std::fs::read(f.broker.dir.join("output")).unwrap(),
        diagnostic
    );
}
