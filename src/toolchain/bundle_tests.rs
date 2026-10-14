use super::*;

#[cfg(unix)]
#[test]
fn downloaded_jr_install_lock_serializes_independent_processes() {
    let f = Fixture::new();
    f.publish(&jr(), &valid_entries(), |_| {});
    let children: Vec<_> = (0..3)
        .map(|_| {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "toolchain::bundle::tests::jr_install_process_child",
                    "--nocapture",
                ])
                .env("JR_INSTALL_PROCESS_FIXTURE", &f.0)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in children {
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let tc = super::super::installed_in(&jr(), &f.root())
        .unwrap()
        .unwrap();
    assert_eq!(tc.info().unwrap().installed_at, 0.0);
    assert_eq!(
        std::fs::read_dir(f.root().join("staged")).unwrap().count(),
        1
    );
    assert!(
        !std::fs::read_dir(f.root())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with(".staging-"))
    );
}

#[cfg(unix)]
#[test]
fn jr_install_process_child() {
    let Some(path) = std::env::var_os("JR_INSTALL_PROCESS_FIXTURE") else {
        return;
    };
    let path = PathBuf::from(path);
    let tc = super::super::install_at(
        &jr(),
        DEFAULT_COMPONENTS,
        &mut Vec::new(),
        &path.join("installed"),
        Some(&format!("file://{}", path.join("store").display())),
    )
    .unwrap();
    assert_eq!(tc.info().unwrap().installed_at, 0.0);
    assert_eq!(tc.id, jr().id());
}

#[cfg(unix)]
#[test]
fn unavailable_shared_bundle_falls_back_to_verified_official_dist_offline() {
    let f = Fixture::new();
    let dist = f.0.join("official/dist/2026-10-08");
    std::fs::create_dir_all(&dist).unwrap();
    let entries = valid_entries();
    let mut text = format!(
        "manifest-version = '2'\ndate = '2026-10-08'\n[pkg.rustc]\nversion = '{}'\n",
        manifest(&jr()).rustc_version
    );
    for (component, files) in [
        ("rustc", vec![entries[0].clone(), entries[1].clone()]),
        ("cargo", vec![entries[2].clone()]),
        ("rust-std", vec![entries[4].clone()]),
    ] {
        let mut archive_entries = vec![
            (
                "package/components".to_owned(),
                component.as_bytes().to_vec(),
                b'0',
            ),
            (
                format!("package/{component}/manifest.in"),
                files
                    .iter()
                    .map(|(name, _, _)| format!("file:{name}\n"))
                    .collect::<String>()
                    .into_bytes(),
                b'0',
            ),
        ];
        archive_entries.extend(
            files
                .into_iter()
                .map(|(name, body, kind)| (format!("package/{component}/{name}"), body, kind)),
        );
        let archive = dist.join(format!("{component}.tar.gz"));
        make_archive(&archive, &archive_entries);
        text.push_str(&format!("\n[pkg.{component}.target.{TARGET}]\navailable = true\nurl = 'file://{}'\nhash = '{}'\n", archive.display(), sha256(&archive).unwrap()));
    }
    let manifest = dist.join("channel-rust-nightly.toml");
    std::fs::write(&manifest, text).unwrap();
    std::fs::write(
        dist.join("channel-rust-nightly.toml.sha256"),
        sha256(&manifest).unwrap(),
    )
    .unwrap();
    // Environment controls are scoped to a child test process, not mutated in
    // a concurrently running test suite. No external network is needed.
    let out = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "toolchain::bundle::tests::official_fallback_child",
            "--nocapture",
        ])
        .env(
            "JUSTRUST_DIST_SERVER",
            format!("file://{}", f.0.join("official").display()),
        )
        .env("JR_OFFICIAL_FALLBACK_FIXTURE", &f.0)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[cfg(unix)]
#[test]
fn official_fallback_child() {
    let Some(path) = std::env::var_os("JR_OFFICIAL_FALLBACK_FIXTURE") else {
        return;
    };
    let path = PathBuf::from(path);
    let spec = Spec::parse("nightly-2026-10-08").unwrap();
    let mut log = Vec::new();
    let tc = super::super::install_at(
        &spec,
        &["rustc", "cargo", "rust-std"],
        &mut log,
        &path.join("installed"),
        Some(&format!("file://{}", path.join("unavailable").display())),
    )
    .unwrap();
    assert!(
        String::from_utf8(log)
            .unwrap()
            .contains("using official dist")
    );
    assert_eq!(
        tc.info().unwrap().manifest_sha256,
        sha256(&path.join("official/dist/2026-10-08/channel-rust-nightly.toml")).unwrap()
    );
    assert!(tc.bin("rustc").is_file());
    assert!(
        !path
            .join("installed")
            .join(format!(".staging-{}", std::process::id()))
            .exists()
    );
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let dir = root.join(format!(
            "jr-toolchain-security-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn base(&self) -> String {
        format!("file://{}", self.0.join("store").display())
    }
    fn root(&self) -> PathBuf {
        self.0.join("installed")
    }
    fn stage(&self) -> PathBuf {
        self.0.join("stage")
    }
    fn publish(
        &self,
        spec: &Spec,
        entries: &[(String, Vec<u8>, u8)],
        edit: impl FnOnce(&mut BundleManifest),
    ) -> BundleManifest {
        let store = self.0.join("store");
        let archive = self.0.join("archive.tar.gz");
        make_archive(&archive, entries);
        let mut m = manifest(spec);
        m.archive_sha256 = sha256(&archive).unwrap();
        m.archive_size = std::fs::metadata(&archive).unwrap().len();
        m.unpacked_size = entries
            .iter()
            .filter(|(_, _, kind)| *kind == b'0')
            .map(|(_, body, _)| body.len() as u64)
            .sum();
        edit(&mut m);
        let object = store.join(format!("v1/toolchains/o/{}.tar.gz", m.archive_sha256));
        std::fs::create_dir_all(object.parent().unwrap()).unwrap();
        std::fs::copy(archive, object).unwrap();
        let path = store.join(format!(
            "v1/toolchains/{TARGET}/{}/manifest.json",
            spec.id()
        ));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec(&m).unwrap()).unwrap();
        m
    }
    fn install(&self, spec: &Spec) -> Result<Toolchain> {
        super::super::install_at(
            spec,
            DEFAULT_COMPONENTS,
            &mut Vec::new(),
            &self.root(),
            Some(&self.base()),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn jr() -> Spec {
    Spec::parse("jr-123456789abc").unwrap()
}
fn manifest(spec: &Spec) -> BundleManifest {
    BundleManifest {
        schema_version: 1,
        id: spec.id(),
        target: TARGET.into(),
        rustc_version: "rustc 1.101.0-nightly (123456789 2026-10-07)".into(),
        rustc_commit_hash: format!("123456789abc{}", "0".repeat(28)),
        cargo_upstream_commit: "abcdef123".into(),
        jr_cargo: Some("987654321".into()),
        archive_sha256: "a".repeat(64),
        archive_size: 1,
        unpacked_size: 1,
        components: ["rustc", "cargo", "rust-std"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        manifest_date: "2026-10-08".into(),
        official_manifest_sha256: None,
    }
}
fn script(text: &str) -> Vec<u8> {
    format!("#!/bin/sh\nprintf '%s\\n' '{text}'\n").into_bytes()
}
fn valid_entries() -> Vec<(String, Vec<u8>, u8)> {
    let m = manifest(&jr());
    vec![
        (
            "bin/rustc".into(),
            script(&format!(
                "{}\ncommit-hash: {}\nhost: {}",
                m.rustc_version, m.rustc_commit_hash, TARGET
            )),
            b'0',
        ),
        ("bin/rustdoc".into(), script("rustdoc"), b'0'),
        (
            "bin/cargo".into(),
            script("cargo 1.101.0 (abcdef123 2026-10-07)"),
            b'0',
        ),
        (
            "bin/jr-cargo".into(),
            script("cargo 1.101.0 (abcdef123 2026-10-07) (jr-cargo 987654321)"),
            b'0',
        ),
        (
            format!("lib/rustlib/{TARGET}/lib/libstd-fixture.rlib"),
            b"fixture".to_vec(),
            b'0',
        ),
        (
            "libexec/rust-analyzer-proc-macro-srv".into(),
            script("proc-macro-srv"),
            b'0',
        ),
    ]
}
fn make_archive(path: &Path, entries: &[(String, Vec<u8>, u8)]) {
    let file = std::fs::File::create(path).unwrap();
    let gz = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    let mut tar = tar::Builder::new(gz);
    for (name, body, kind) in entries {
        let mut header = tar::Header::new_ustar();
        // Construct hostile names without the builder's own path sanitizer.
        let bytes = name.as_bytes();
        if bytes.len() <= 100 {
            header.as_mut_bytes()[..bytes.len()].copy_from_slice(bytes);
        } else {
            header.set_path(name).unwrap();
        }
        header.set_mode(0o755);
        header.set_size(body.len() as u64);
        header.set_entry_type(tar::EntryType::new(*kind));
        if *kind == b'1' || *kind == b'2' {
            header.set_link_name("../../outside").unwrap();
        }
        header.set_cksum();
        tar.append(&header, body.as_slice()).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap();
}

#[test]
fn manifest_rejects_wrong_target_id_commit_digest_sizes_and_schema() {
    let good = manifest(&jr());
    assert!(good.validate(&jr()).is_ok());
    for field in 0..10 {
        let mut m = good.clone();
        match field {
            0 => m.target = "unsupported-target".into(),
            1 => m.id = "jr-0000000".into(),
            2 => m.rustc_commit_hash = "0".repeat(40),
            3 => m.archive_sha256 = "../escape".into(),
            4 => m.archive_size = MAX_ARCHIVE + 1,
            5 => m.unpacked_size = MAX_UNPACKED + 1,
            6 => m.schema_version = 2,
            7 => m.components.clear(),
            8 => m.jr_cargo = Some("bad/name".into()),
            9 => m.cargo_upstream_commit = "g".repeat(9),
            _ => unreachable!(),
        }
        assert!(m.validate(&jr()).is_err(), "field {field}");
    }
    assert!(Spec::parse(&format!("jr-{}", "a".repeat(41))).is_err());
}

#[test]
fn trusted_base_requires_https_no_credentials_queries_or_redirect_controls() {
    assert_eq!(
        parse_base("https://public.example/store/")
            .unwrap()
            .as_deref(),
        Some("https://public.example/store")
    );
    for base in [
        "http://public.example",
        "https://user:secret@host",
        "https://host?url=x",
        "https://host#x",
        "https://",
        "https://host\\x",
    ] {
        assert!(parse_base(base).is_err(), "{base}");
    }
    assert!(parse_base("off").unwrap().is_none());
}

#[test]
fn extraction_rejects_traversal_absolute_links_devices_duplicates_and_extensions() {
    for (i, name, kind) in [
        (0, "../outside", b'0'),
        (1, "/outside", b'0'),
        (2, "bin/../../outside", b'0'),
        (3, "bin/link", b'2'),
        (4, "bin/hard", b'1'),
        (5, "bin/device", b'3'),
        (6, "bin/pax", b'x'),
        (7, "bin/long", b'L'),
        (8, "bin\\..\\outside", b'0'),
        (9, "bin/C:escape", b'0'),
        (10, MARKER, b'0'),
    ] {
        let f = Fixture::new();
        let archive = f.0.join("bad.tar.gz");
        make_archive(
            &archive,
            &[(
                name.into(),
                if kind == b'0' { vec![1] } else { vec![] },
                kind,
            )],
        );
        assert!(
            extract(&archive, &f.0.join(format!("prefix{i}")), 1).is_err(),
            "{name}"
        );
        assert!(!f.0.join("outside").exists());
    }
    let f = Fixture::new();
    let archive = f.0.join("dup.tar.gz");
    make_archive(
        &archive,
        &[
            ("bin/a".into(), vec![1], b'0'),
            ("bin/a".into(), vec![2], b'0'),
        ],
    );
    assert!(extract(&archive, &f.stage(), 2).is_err());
}

#[test]
fn extraction_rejects_truncation_corrupt_gzip_and_declared_size_mismatch() {
    for variant in 0..4 {
        let f = Fixture::new();
        let archive = f.0.join("bad.tar.gz");
        make_archive(&archive, &[("bin/a".into(), vec![1, 2, 3], b'0')]);
        let mut bytes = std::fs::read(&archive).unwrap();
        let size = match variant {
            0 => {
                bytes.truncate(bytes.len() / 2);
                3
            }
            1 => {
                let n = bytes.len();
                bytes[n - 8] ^= 1;
                3
            }
            2 => 2,
            _ => 4,
        };
        std::fs::write(&archive, bytes).unwrap();
        assert!(
            extract(&archive, &f.stage(), size).is_err(),
            "variant {variant}"
        );
    }
}

#[cfg(unix)]
#[test]
fn downloaded_jr_is_atomic_staged_inactive_idempotent_and_concurrent() {
    let f = Fixture::new();
    let spec = jr();
    f.publish(&spec, &valid_entries(), |_| {});
    std::fs::create_dir_all(f.root().join(".staging-interrupted/prefix/bin")).unwrap();
    std::fs::create_dir_all(f.root().join("staged").join(spec.id())).unwrap();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| f.install(&spec).unwrap()))
            .collect();
        for h in handles {
            let tc = h.join().unwrap();
            assert_eq!(tc.dir, f.root().join("staged").join(spec.id()));
            assert_eq!(tc.info().unwrap().installed_at, 0.0);
            assert!(super::super::newest_active_jr([(tc, 0.0)]).is_none());
        }
    });
    assert!(!f.root().join(spec.id()).exists());
    assert!(!f.root().join(".staging-interrupted").exists());
    assert!(
        !f.root()
            .join(format!(".staging-{}", std::process::id()))
            .exists()
    );
    std::fs::remove_dir_all(f.0.join("store")).unwrap();
    assert!(f.install(&spec).is_ok());
}

#[cfg(unix)]
#[test]
fn install_rejects_tampered_hash_wrong_target_incomplete_and_wrong_binary_identities() {
    for variant in 0..8 {
        let f = Fixture::new();
        let spec = jr();
        let mut entries = valid_entries();
        match variant {
            2 => entries.retain(|(name, _, _)| name != "bin/cargo"),
            3 => entries.retain(|(name, _, _)| !name.ends_with(".rlib")),
            4 => entries[0].1 = script("rustc forged\ncommit-hash: forged"),
            5 => entries[2].1 = script("cargo 1.101.0 (000000000 2026-10-07)"),
            6 => entries[3].1 = script("cargo 1.101.0 (abcdef123 2026-10-07) (jr-cargo 000000000)"),
            _ => {}
        }
        let m = f.publish(&spec, &entries, |m| {
            if variant == 1 {
                m.target = "unsupported".into();
            }
            if variant == 7 {
                m.components.push("rust-src".into());
            }
        });
        if variant == 0 {
            let path =
                f.0.join(format!("store/v1/toolchains/o/{}.tar.gz", m.archive_sha256));
            let mut bytes = std::fs::read(&path).unwrap();
            bytes[0] ^= 1;
            std::fs::write(path, bytes).unwrap();
        }
        assert!(f.install(&spec).is_err(), "variant {variant}");
        assert!(
            super::super::installed_in(&spec, &f.root())
                .unwrap()
                .is_none()
        );
        assert!(
            !f.root()
                .join(format!(".staging-{}", std::process::id()))
                .exists()
        );
    }
}

#[cfg(unix)]
#[test]
fn official_nightly_bundle_preserves_stock_rustc_and_shared_cargo_provenance() {
    let f = Fixture::new();
    let spec = Spec::parse("nightly-2026-10-08").unwrap();
    f.publish(&spec, &valid_entries(), |m| {
        m.official_manifest_sha256 = Some("b".repeat(64))
    });
    let tc = super::super::install_at(
        &spec,
        &["rustc", "cargo", "rust-std"],
        &mut Vec::new(),
        &f.root(),
        Some(&f.base()),
    )
    .unwrap();
    assert_eq!(tc.dir, f.root().join(spec.id()));
    assert_eq!(tc.info().unwrap().manifest_sha256, "b".repeat(64));
    assert!(tc.info().unwrap().installed_at > 0.0);
    assert!(
        version(&tc.dir, JR_CARGO, "-V")
            .unwrap()
            .contains("jr-cargo 987654321")
    );
    assert!(super::super::newest_active_jr([(tc, 10.0)]).is_none());
}
