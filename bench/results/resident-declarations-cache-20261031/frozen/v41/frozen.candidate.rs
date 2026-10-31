//! Frozen dependency artifacts (`docs/glancer.md`, item 1; #40): the def map, the impls and
//! the lang items of each crate whose source cannot change, written to the store once and read
//! back instead of resolving and finding them again.
//!
//! A crate is frozen when its source is the toolchain's (the standard library) or a
//! registry's or git checkout's, and every crate it depends on is frozen too. Its key is a
//! Merkle hash: where the source is below the toolchain, registry or checkout, its edition,
//! kind, crate attributes, cfg, environment, and the names and keys of its dependencies; the store adds the toolchain
//! and ra-jit's own build. `OUT_DIR` counts by its place below the target directory and by what
//! the build script wrote there, so the same crate in another checkout has the same key.
//!
//! `RA_JIT_FROZEN` says what is done with what the store has: by default it is served;
//! `check` resolves and finds as well and fails the build on any difference, `verify` writes
//! what was read and fails on any difference, `off` neither reads nor writes.

use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering::Relaxed},
};

use ide_db::base_db::Crate;
use rustc_hash::FxHashMap;
use sha2::{Digest, Sha256};

use hir_def::nameres::frozen::{self as hir_frozen, Frozen, Mode};
use hir_ty::method_resolution::frozen as impls_frozen;
use ide_db::base_db::SourceDatabase as _;

/// This experiment does not reuse the historical frozen key: that key assumes package
/// contents are immutable. Declaration discovery instead certifies the entire current
/// source-root inventory, including files not named by the previous declaration result.
/// Root hashing is a native Salsa query so crates sharing a root do not rehash its text.
#[salsa::tracked(returns(copy))]
fn declarations_root<'db>(
    db: &'db dyn hir::db::HirDatabase,
    root: ide_db::base_db::InternedSourceRootId<'db>,
) -> Option<[u8; 32]> {
    let source = db.source_root(root.id(db)).source_root(db);
    let mut files = source
        .iter()
        .map(|file| Some((source.path_for_file(&file)?.to_string(), file)))
        .collect::<Option<Vec<_>>>()?;
    files.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut hash = Sha256::new();
    declaration_field(&mut hash, b"declarations full source root 1");
    declaration_field(&mut hash, &(files.len() as u64).to_le_bytes());
    for (path, file) in files {
        // No lossy read, empty-on-error read, or disk-only source can certify the DB.
        if !db.has_file_text(file) {
            return None;
        }
        declaration_field(&mut hash, path.as_bytes());
        declaration_field(&mut hash, db.file_text(file).text(db).as_bytes());
    }
    Some(hash.finalize().into())
}

fn declaration_field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

/// Same breadth-first dependency indexing as Enc/Dec. These indices are portable graph
/// positions, never Salsa IDs. The inventory is also used to retain diagnostic read effects.
fn declarations_closure(db: &ide_db::RootDatabase, krate: Crate) -> Vec<Crate> {
    let mut closure = vec![krate];
    let mut at = 0;
    while let Some(&next) = closure.get(at) {
        at += 1;
        for dep in &next.data(db).dependencies {
            if !closure.contains(&dep.crate_id) {
                closure.push(dep.crate_id);
            }
        }
    }
    closure
}

/// Strict initial eligibility: actual language-library origins and their non-local,
/// non-proc-macro dependencies, all loaded under the same toolchain source directory.
/// Paths only decide eligibility. Full contents, cfg and environment decide identity.
#[cfg(test)]
fn declarations_eligible(db: &ide_db::RootDatabase, closure: &[Crate]) -> bool {
    declarations_eligibility(db, closure).is_ok()
}

fn declarations_eligibility(
    db: &ide_db::RootDatabase,
    closure: &[Crate],
) -> Result<(), &'static str> {
    let Some(&krate) = closure.first() else { return Err("empty-closure") };
    if !krate.data(db).origin.is_lang() {
        return Err("not-language-library");
    }
    let Some(root) = crate::crate_root_file(db, krate.into()) else { return Err("no-real-root") };
    let Some(root) = root.to_str() else { return Err("non-utf8-root") };
    let marker = "/lib/rustlib/src/rust/";
    let Some(at) = root.find(marker) else { return Err("not-toolchain-source") };
    let toolchain_source = std::path::Path::new(&root[..at + marker.len()]);
    for &krate in closure {
        let data = krate.data(db);
        if data.origin.is_local() {
            return Err("local-dependency");
        }
        if data.target_kind.is_proc_macro() {
            return Err("proc-macro-dependency");
        }
        let source = db.file_source_root(data.root_file_id).source_root_id(db);
        let source = db.source_root(source).source_root(db);
        if !source.is_library {
            return Err("non-library-source-root");
        }
        if !source.iter().all(|file| {
            source.path_for_file(&file).and_then(|path| path.as_path()).is_some_and(|path| {
                std::path::Path::new(path.as_str()).starts_with(toolchain_source)
            })
        }) {
            return Err("mixed-or-virtual-source-path");
        }
    }
    Ok(())
}

/// Complete current DB source/config/dependency certificate. Absolute paths and exact
/// environment values intentionally sacrifice cross-checkout reuse for exactness.
fn declarations_certificate(db: &ide_db::RootDatabase, closure: &[Crate]) -> Option<[u8; 32]> {
    let mut hash = Sha256::new();
    declaration_field(&mut hash, b"ra-jit exact declarations 41.1");
    declaration_field(&mut hash, &(closure.len() as u64).to_le_bytes());
    // Process inputs are validated by the complete actual-read witness in the record.
    // Unobserved runtime nonce variables must not invalidate an otherwise exact source key.
    for &krate in closure {
        let data = krate.data(db);
        let root = db.file_source_root(data.root_file_id).source_root_id(db);
        let source = db.source_root(root).source_root(db);
        declaration_field(
            &mut hash,
            source.path_for_file(&data.root_file_id)?.to_string().as_bytes(),
        );
        declaration_field(
            &mut hash,
            &declarations_root(db, ide_db::base_db::InternedSourceRootId::new(db, root))?,
        );
        declaration_field(
            &mut hash,
            format!(
                "{:?} {:?} {:?} {:?} {:?} {:?}",
                data.edition,
                data.origin,
                data.crate_attrs,
                data.target_kind,
                data.proc_macro_cwd,
                krate.workspace_data(db)
            )
            .as_bytes(),
        );
        let mut cfg: Vec<_> = krate.cfg_options(db).into_iter().map(|it| it.to_string()).collect();
        cfg.sort_unstable();
        declaration_field(&mut hash, format!("{cfg:?}").as_bytes());
        declaration_field(&mut hash, format!("{:?}", krate.env(db)).as_bytes());
        declaration_field(&mut hash, format!("{:?}", krate.extra_data(db)).as_bytes());
        declaration_field(&mut hash, &(data.dependencies.len() as u64).to_le_bytes());
        for dep in &data.dependencies {
            let index = closure.iter().position(|it| *it == dep.crate_id)?;
            declaration_field(&mut hash, &(index as u64).to_le_bytes());
            declaration_field(&mut hash, dep.name.as_str().as_bytes());
            declaration_field(&mut hash, &[dep.is_prelude() as u8, dep.is_sysroot() as u8]);
        }
        declaration_field(&mut hash, b"end crate");
    }
    Some(hash.finalize().into())
}

/// External byte reads and unresolved include-source requests have no complete native
/// declaration read certificate yet. They always fall through, never become empty input.
fn declarations_reads_supported(db: &ide_db::RootDatabase, closure: &[Crate]) -> bool {
    closure.iter().all(|&krate| db.included_by(krate).is_empty())
        && db.included_sources().is_empty()
}

/// Only the effective value actually seen by the native builtin macro matters. None is
/// distinct from an empty value. Hashes keep values out of records and diagnostic output.
fn declarations_env_digest(db: &ide_db::RootDatabase, krate: Crate, name: &str) -> [u8; 32] {
    let value = krate.env(db).get(name).or_else(|| ide_db::base_db::process_env_var(db, name));
    let mut hash = Sha256::new();
    declaration_field(&mut hash, b"native macro effective env 1");
    declaration_field(&mut hash, &[value.is_some() as u8]);
    if let Some(value) = value {
        declaration_field(&mut hash, value.as_bytes());
    }
    hash.finalize().into()
}

fn encode_declarations(
    db: &ide_db::RootDatabase,
    krate: Crate,
    certificate: &[u8; 32],
    value: &crate::Declared,
) -> Result<Vec<u8>, String> {
    // Encode first: locating block owners may discover additional real macro reads.
    let payload = encode_declarations_payload(db, krate, certificate, value)?;
    let closure = declarations_closure(db, krate);
    let mut reads: Vec<_> =
        db.env_read().into_iter().filter(|(of, _)| closure.contains(of)).collect();
    reads.sort_unstable_by(|a, b| {
        closure
            .iter()
            .position(|it| *it == a.0)
            .cmp(&closure.iter().position(|it| *it == b.0))
            .then_with(|| a.1.cmp(&b.1))
    });
    let mut enc = hir_frozen::Enc::new(db, krate);
    enc.str("exact declarations observed inputs 41.1")?;
    enc.usize(reads.len())?;
    for (of, name) in reads {
        enc.krate(of)?;
        enc.str(&name)?;
        for byte in declarations_env_digest(db, of, &name) {
            enc.u8(byte);
        }
    }
    enc.usize(payload.len())?;
    for byte in payload {
        enc.u8(byte);
    }
    enc.finish()
}

fn decode_declarations(
    db: &ide_db::RootDatabase,
    krate: Crate,
    certificate: &[u8; 32],
    bytes: &[u8],
) -> Result<crate::Declared, String> {
    let mut dec = hir_frozen::Dec::new_checked(db, krate, bytes)?;
    if dec.str()? != "exact declarations observed inputs 41.1" {
        return Err("declaration observed-input format".into());
    }
    // Validate every actual native environment read before decoding any definitions.
    // Under exact source/cfg/dependency identity, a changed branch must first change one
    // of these observed reads. Proc macros and external filesystem reads remain refused.
    for _ in 0..dec.len()? {
        let of = dec.krate()?;
        let name = dec.str()?.to_owned();
        for byte in declarations_env_digest(db, of, &name) {
            if dec.u8()? != byte {
                return Err("declaration observed environment changed".into());
            }
        }
    }
    let mut payload = Vec::new();
    for _ in 0..dec.len()? {
        payload.push(dec.u8()?);
    }
    dec.finish()?;
    decode_declarations_payload(db, krate, certificate, &payload)
}

fn encode_declarations_payload(
    db: &ide_db::RootDatabase,
    krate: Crate,
    certificate: &[u8; 32],
    value: &crate::Declared,
) -> Result<Vec<u8>, String> {
    let mut enc = hir_frozen::Enc::new(db, krate);
    enc.str("exact declarations 39.1")?;
    for &byte in certificate {
        enc.u8(byte);
    }
    enc.usize(value.functions.len())?;
    for &id in &value.functions {
        enc.def(id.into())?;
    }
    enc.usize(value.statics.len())?;
    for &id in &value.statics {
        enc.def(id.into())?;
    }
    enc.usize(value.in_provided.len())?;
    for &id in &value.in_provided {
        enc.def(id.into())?;
    }
    enc.usize(value.bodies.len())?;
    for &id in &value.bodies {
        enc.body_owner(id)?;
    }
    enc.usize(value.provided.len())?;
    for &id in &value.provided {
        enc.body_owner(id)?;
    }
    enc.bool(value.under_provided);
    let closure = declarations_closure(db, krate);
    // Retain the actual native macro environment read sets, including unset diagnostics.
    for reads in [db.env_read(), db.env_unset()] {
        let mut reads: Vec<_> = reads.into_iter().filter(|(of, _)| closure.contains(of)).collect();
        reads.sort_unstable_by(|a, b| {
            closure
                .iter()
                .position(|it| *it == a.0)
                .cmp(&closure.iter().position(|it| *it == b.0))
                .then_with(|| a.1.cmp(&b.1))
        });
        enc.usize(reads.len())?;
        for (of, name) in reads {
            enc.krate(of)?;
            enc.str(&name)?;
        }
    }
    enc.finish()
}

fn decode_declarations_payload(
    db: &ide_db::RootDatabase,
    krate: Crate,
    certificate: &[u8; 32],
    bytes: &[u8],
) -> Result<crate::Declared, String> {
    let mut dec = hir_frozen::Dec::new_checked(db, krate, bytes)?;
    if dec.str()? != "exact declarations 39.1" {
        return Err("declaration format".into());
    }
    for &byte in certificate {
        if dec.u8()? != byte {
            return Err("declaration inputs changed".into());
        }
    }
    let mut value = crate::Declared::default();
    for _ in 0..dec.len()? {
        let hir_def::ModuleDefId::FunctionId(id) = dec.def()? else {
            return Err("not a declared function".into());
        };
        value.functions.push(id);
    }
    for _ in 0..dec.len()? {
        let hir_def::ModuleDefId::StaticId(id) = dec.def()? else {
            return Err("not a declared static".into());
        };
        value.statics.push(id);
    }
    for _ in 0..dec.len()? {
        let hir_def::ModuleDefId::FunctionId(id) = dec.def()? else {
            return Err("not a provided function".into());
        };
        value.in_provided.push(id);
    }
    for _ in 0..dec.len()? {
        value.bodies.push(dec.body_owner()?);
    }
    for _ in 0..dec.len()? {
        value.provided.push(dec.body_owner()?);
    }
    value.under_provided = dec.bool()?;
    let mut effects = Vec::new();
    for unset in [false, true] {
        for _ in 0..dec.len()? {
            effects.push((unset, dec.krate()?, dec.str()?.to_owned()));
        }
    }
    dec.finish()?;
    // Never replay partial records. These effects keep unset diagnostics and watchers intact.
    for (unset, of, name) in effects {
        if unset {
            db.env_is_unset(of, &name);
        } else {
            db.env_is_read(of, &name);
        }
    }
    Ok(value)
}

static DECLARATIONS_HITS: AtomicUsize = AtomicUsize::new(0);

fn declarations_fallback(db: &ide_db::RootDatabase, krate: hir::Crate, why: &str) {
    if std::env::var_os("RA_JIT_STATS").is_some() {
        let of: Crate = krate.into();
        let name =
            of.extra_data(db).display_name.as_ref().map(|it| it.to_string()).unwrap_or_default();
        let closure = declarations_closure(db, of);
        let external_reads: usize = closure.iter().map(|&it| db.included_by(it).len()).sum();
        let pending_sources = db.included_sources().len();
        eprintln!(
            "declarations-cache: crate={name:?} origin={:?} event={why} external_reads={external_reads} pending_sources={pending_sources}",
            of.data(db).origin
        );
    }
}

fn declarations_codec_error(db: &ide_db::RootDatabase, krate: hir::Crate, phase: &str, why: &str) {
    if std::env::var_os("RA_JIT_STATS").is_none() {
        return;
    }
    // Never echo error text, which can contain paths substituted from environment values.
    // Give an actionable codec category and stable fingerprint without exposing those values.
    let kind =
        ["environment", "block", "file", "macro", "literal", "module", "body", "format", "input"]
            .into_iter()
            .find(|kind| why.contains(kind))
            .unwrap_or("other");
    let hash = Sha256::digest(why.as_bytes());
    declarations_fallback(
        db,
        krate,
        &format!("{phase}-error kind={kind} reason_sha256={}", crate::content::hex(&hash)),
    );
}

#[cfg(test)]
mod declaration_tests {
    use super::*;
    use test_fixture::WithFixture;

    const SOURCE: &str = r#"
//- /decoy/lib.rs crate:decoy
fn a() {} fn b() {} fn c() {} fn d() {}
//- /target/lib.rs crate:target cfg:feature="selected"
macro_rules! nested { () => { fn expanded() { static MACRO_STATIC: u8 = 2; } }; }
fn outer() {
    fn inner() { fn deeper() {} }
    static BLOCK_STATIC: u8 = { fn static_body() {} 1 };
    nested!();
}
trait Trait { fn provided() { fn under_provided() { nested!(); } } }
struct S;
impl S { fn method() { fn in_method() {} } }
#[cfg(feature = "selected")] fn selected() {}
static TOP: u8 = { fn top_static_body() {} 0 };
"#;

    fn fixture(source: &str) -> (ide_db::RootDatabase, hir::Crate, hir::Crate) {
        let (db, _) = ide_db::RootDatabase::with_many_files(source);
        let find = |name: &str| {
            ide_db::base_db::all_crates(&db)
                .iter()
                .copied()
                .find(|it| {
                    it.extra_data(&db)
                        .display_name
                        .as_ref()
                        .is_some_and(|it| it.to_string() == name)
                })
                .unwrap()
        };
        let (target, decoy) = (find("target"), find("decoy"));
        (db, target.into(), decoy.into())
    }

    fn certificate(db: &ide_db::RootDatabase, krate: hir::Crate) -> [u8; 32] {
        declarations_certificate(db, &declarations_closure(db, krate.into())).unwrap()
    }

    fn fixture_real_paths(db: &mut ide_db::RootDatabase, krate: hir::Crate) {
        let root = db.file_source_root(krate.root_file(db)).source_root_id(db);
        let source = db.source_root(root).source_root(db);
        let mut files = vfs::file_set::FileSet::default();
        for file in source.iter() {
            files.insert(
                file,
                vfs::VfsPath::new_real_path(source.path_for_file(&file).unwrap().to_string()),
            );
        }
        db.set_source_root_with_durability(
            root,
            triomphe::Arc::new(ide_db::base_db::SourceRoot::new_local(files)),
            ide_db::base_db::salsa::Durability::LOW,
        );
    }

    #[test]
    fn declarations_certificate_tracks_dependency_content_and_edges() {
        let source = format!("{SOURCE}\n//- /dep/lib.rs crate:dep\npub fn dependency() {{}}\n");
        let with_dep = source.replace("crate:target cfg:", "crate:target deps:dep cfg:");
        let (without, target, _) = fixture(&source);
        let without = certificate(&without, target);
        let (with, target, _) = fixture(&with_dep);
        let with = certificate(&with, target);
        assert_ne!(without, with);
        let changed = with_dep.replace("fn dependency() {}", "fn replacement() {}");
        let (db, target, _) = fixture(&changed);
        assert_ne!(certificate(&db, target), with);
    }

    #[test]
    fn declarations_external_byte_reads_are_native_unsupported_inputs() {
        let (mut db, target, _) = fixture(SOURCE);
        fixture_real_paths(&mut db, target);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("external.txt");
        std::fs::write(&file, b"actual external macro input").unwrap();
        let closure = declarations_closure(&db, target.into());
        assert!(declarations_reads_supported(&db, &closure));
        let read = db
            .included_file(
                target.into(),
                ide_db::base_db::AnchoredPath {
                    anchor: target.root_file(&db),
                    path: file.to_str().unwrap(),
                },
            )
            .unwrap();
        assert_eq!(read.bytes(&db).as_deref(), Some(&b"actual external macro input"[..]));
        assert!(!declarations_reads_supported(&db, &closure));
    }

    #[test]
    fn declarations_round_trip_remaps_fresh_database_and_preserves_native_walk() {
        let (writer, target, _) = fixture(SOURCE);
        let (bytes, key, original) = hir::attach_db(&writer, || {
            let original = crate::declared_in_original(&writer, target);
            assert!(!original.functions.is_empty());
            assert!(!original.statics.is_empty());
            assert!(!original.in_provided.is_empty());
            let key = certificate(&writer, target);
            let bytes = encode_declarations(&writer, target.into(), &key, &original).unwrap();
            (bytes, key, original)
        });
        let (reader, target, decoy) = fixture(SOURCE);
        hir::attach_db(&reader, || {
            // Resolve unrelated actual functions first, changing interned definition IDs.
            crate::declared_in_original(&reader, decoy);
            assert_eq!(certificate(&reader, target), key);
            let read = decode_declarations(&reader, target.into(), &key, &bytes).unwrap();
            let oracle = crate::declared_in_original(&reader, target);
            assert!(read == oracle, "portable declarations differ from fresh original walk");
            assert_ne!(original.functions[0], read.functions[0], "fixture must remap IDs");
        });
    }

    #[test]
    fn declarations_portable_record_preserves_order_duplicates_and_body_queues() {
        let (db, target, _) = fixture(SOURCE);
        hir::attach_db(&db, || {
            let mut value = crate::declared_in_original(&db, target);
            value.functions.reverse();
            value.functions.push(value.functions[0]);
            value.statics.push(value.statics[0]);
            value.in_provided.push(value.in_provided[0]);
            value
                .bodies
                .extend(value.functions.iter().take(2).copied().map(hir_def::DefWithBodyId::from));
            value.provided.extend(
                value.in_provided.iter().take(2).copied().map(hir_def::DefWithBodyId::from),
            );
            value.under_provided = true;
            let key = certificate(&db, target);
            let bytes = encode_declarations(&db, target.into(), &key, &value).unwrap();
            let read = decode_declarations(&db, target.into(), &key, &bytes).unwrap();
            assert!(value == read, "declaration vector order/multiplicity/state changed");
        });
    }

    #[test]
    fn declarations_reject_truncation_corruption_and_wrong_certificate() {
        let (db, target, _) = fixture(SOURCE);
        hir::attach_db(&db, || {
            let value = crate::declared_in_original(&db, target);
            let key = certificate(&db, target);
            let bytes = encode_declarations(&db, target.into(), &key, &value).unwrap();
            for cut in 0..bytes.len() {
                assert!(
                    decode_declarations(&db, target.into(), &key, &bytes[..cut]).is_err(),
                    "cut {cut}"
                );
            }
            let mut extra = bytes.clone();
            extra.push(0);
            assert!(decode_declarations(&db, target.into(), &key, &extra).is_err());
            let mut key_changed = key;
            key_changed[0] ^= 1;
            assert!(decode_declarations(&db, target.into(), &key_changed, &bytes).is_err());
            let mut corrupt = bytes;
            corrupt[0] = 0xff;
            assert!(decode_declarations(&db, target.into(), &key, &corrupt).is_err());
        });
    }

    #[test]
    fn declarations_certificate_tracks_source_cfg_path_inventory_and_environment() {
        let (mut db, target, _) = fixture(SOURCE);
        let original = certificate(&db, target);
        for source in [
            SOURCE.replace("fn deeper() {}", "fn changed() {}"),
            SOURCE.replace("cfg:feature=\"selected\"", "cfg:feature=\"other\""),
            format!("{SOURCE}\n//- /target/added.rs\nfn new_file() {{}}\n"),
            SOURCE.replace("/target/lib.rs", "/target/moved.rs"),
        ] {
            let (changed, target, _) = fixture(&source);
            assert_ne!(certificate(&changed, target), original);
        }
        // Invalidation in the same DB must also update the tracked full-root digest.
        let file = target.root_file(&db);
        let text = db.file_text(file).text(&db).to_string();
        db.set_file_text_with_durability(
            file,
            &text.replace("fn deeper() {}", "fn changed() {}"),
            ide_db::base_db::salsa::Durability::LOW,
        );
        assert_ne!(certificate(&db, target), original);
        db.set_file_text_with_durability(file, &text, ide_db::base_db::salsa::Durability::LOW);
        assert_eq!(certificate(&db, target), original);
        // Keep the crate identity while changing the real native configuration inputs.
        let krate: Crate = target.into();
        let mut vars = krate.env(&db).clone();
        vars.set("OUT_DIR", "/different/build/out");
        db.set_env(krate, vars);
        assert_ne!(certificate(&db, target), original);
        let before_process = certificate(&db, target);
        let mut process = ide_db::base_db::Env::default();
        process.set("NEW_PROCESS_INPUT", "new value");
        ide_db::base_db::set_process_env(&mut db, process);
        assert_eq!(
            certificate(&db, target),
            before_process,
            "unused process inputs are validated by the observed-read witness, not source identity"
        );
    }

    #[test]
    fn declarations_observed_env_witness_allows_nonce_changes_but_rejects_changed_reads() {
        let source = format!(
            "{SOURCE}\n#[rustc_builtin_macro]\nmacro_rules! env {{ () => {{}} }}\nfn process_input() {{ let _ = env!(\"OBSERVED_INPUT\"); }}\n"
        );
        let (mut writer, target, _) = fixture(&source);
        let mut vars = ide_db::base_db::Env::default();
        vars.set("OBSERVED_INPUT", "original");
        vars.set("UNOBSERVED_NONCE", "writer");
        ide_db::base_db::set_process_env(&mut writer, vars);
        let (key, bytes) = hir::attach_db(&writer, || {
            let value = crate::declared_in_original(&writer, target);
            assert!(writer.env_read().contains(&(target.into(), "OBSERVED_INPUT".into())));
            let key = certificate(&writer, target);
            (key, encode_declarations(&writer, target.into(), &key, &value).unwrap())
        });
        let (mut reader, target, _) = fixture(&source);
        let mut vars = ide_db::base_db::Env::default();
        vars.set("OBSERVED_INPUT", "original");
        vars.set("UNOBSERVED_NONCE", "reader");
        ide_db::base_db::set_process_env(&mut reader, vars.clone());
        assert_eq!(certificate(&reader, target), key);
        hir::attach_db(&reader, || {
            let read = decode_declarations(&reader, target.into(), &key, &bytes).unwrap();
            assert!(read == crate::declared_in_original(&reader, target));
        });
        vars.set("OBSERVED_INPUT", "changed");
        ide_db::base_db::set_process_env(&mut reader, vars);
        assert_eq!(certificate(&reader, target), key);
        hir::attach_db(&reader, || {
            let error = decode_declarations(&reader, target.into(), &key, &bytes).err().unwrap();
            assert_eq!(error, "declaration observed environment changed");
        });
        let absent = ide_db::base_db::Env::default();
        ide_db::base_db::set_process_env(&mut reader, absent);
        let missing = declarations_env_digest(&reader, target.into(), "OBSERVED_INPUT");
        let mut empty = ide_db::base_db::Env::default();
        empty.set("OBSERVED_INPUT", "");
        ide_db::base_db::set_process_env(&mut reader, empty);
        assert_ne!(missing, declarations_env_digest(&reader, target.into(), "OBSERVED_INPUT"));
    }

    #[test]
    fn declarations_replay_macro_environment_diagnostics_only_after_complete_decode() {
        let source = format!(
            "{SOURCE}\n#[rustc_builtin_macro]\nmacro_rules! env {{ () => {{}} }}\nfn reads_env() {{ let _ = env!(\"READ_BUT_UNSET\"); }}\n"
        );
        let (writer, target, _) = fixture(&source);
        let (bytes, key) = hir::attach_db(&writer, || {
            let value = crate::declared_in_original(&writer, target);
            assert!(writer.env_read().contains(&(target.into(), "READ_BUT_UNSET".into())));
            assert!(writer.env_unset().contains(&(target.into(), "READ_BUT_UNSET".into())));
            let key = certificate(&writer, target);
            (encode_declarations(&writer, target.into(), &key, &value).unwrap(), key)
        });
        let (reader, target, _) = fixture(&source);
        hir::attach_db(&reader, || {
            assert!(
                decode_declarations(&reader, target.into(), &key, &bytes[..bytes.len() - 1])
                    .is_err()
            );
            assert!(!reader.env_unset().iter().any(|(_, name)| name == "READ_BUT_UNSET"));
            decode_declarations(&reader, target.into(), &key, &bytes).unwrap();
            assert!(reader.env_read().contains(&(target.into(), "READ_BUT_UNSET".into())));
            assert!(reader.env_unset().contains(&(target.into(), "READ_BUT_UNSET".into())));
        });
    }

    #[test]
    fn declarations_do_not_promote_local_fixture_paths_to_immutable_libraries() {
        let (mut db, target, _) = fixture(SOURCE);
        let closure = declarations_closure(&db, target.into());
        assert!(!declarations_eligible(&db, &closure));
        assert!(declarations_reads_supported(&db, &closure));
        // include_source records native real paths, not virtual fixture paths.
        fixture_real_paths(&mut db, target);
        let root = target.root_file(&db);
        db.include_source(ide_db::base_db::AnchoredPath { anchor: root, path: "missing.rs" });
        assert!(!declarations_reads_supported(&db, &closure));
    }
}

/// Called only behind the exact default-off flag in the measured native declaration owner.
/// Historical frozen modes retain their existing behavior. In particular off never reads
/// or writes this experiment and check/verify continue to execute the original walk.
pub(crate) fn declarations_or_walk(
    db: &ide_db::RootDatabase,
    krate: hir::Crate,
) -> crate::Declared {
    if mode() != Some(Mode::Serve) {
        declarations_fallback(db, krate, "frozen-mode");
        return crate::declared_in_original(db, krate);
    }
    let Some(store) = ra_jit_store::Store::global() else {
        declarations_fallback(db, krate, "no-store");
        return crate::declared_in_original(db, krate);
    };
    let closure = declarations_closure(db, krate.into());
    if let Err(why) = declarations_eligibility(db, &closure) {
        declarations_fallback(db, krate, why);
        return crate::declared_in_original(db, krate);
    }
    if !declarations_reads_supported(db, &closure) {
        declarations_fallback(db, krate, "unsupported-external-or-pending-read");
        return crate::declared_in_original(db, krate);
    }
    let Some(key) = declarations_certificate(db, &closure) else {
        declarations_fallback(db, krate, "uncertifiable-input");
        return crate::declared_in_original(db, krate);
    };
    match store.get(&key) {
        Some(bytes) => match decode_declarations(db, krate.into(), &key, &bytes) {
            Ok(value) if declarations_reads_supported(db, &closure) => {
                let hits = DECLARATIONS_HITS.fetch_add(1, Relaxed) + 1;
                if std::env::var_os("RA_JIT_STATS").is_some() {
                    let of: Crate = krate.into();
                    let name = of
                        .extra_data(db)
                        .display_name
                        .as_ref()
                        .map(|it| it.to_string())
                        .unwrap_or_default();
                    eprintln!(
                        "declarations-cache: native_hit=1 native_hits={hits} crate={name:?} origin={:?}",
                        of.data(db).origin
                    );
                }
                return value;
            }
            Ok(_) => declarations_fallback(db, krate, "decode-discovered-unsupported-read"),
            Err(why) => declarations_codec_error(db, krate, "decode", &why),
        },
        None => declarations_fallback(db, krate, "store-miss"),
    }
    let value = crate::declared_in_original(db, krate);
    // Both walking and encoding may discover new native reads. Attribute each refusal,
    // and never publish a record under inputs different from its lookup certificate.
    if !declarations_reads_supported(db, &closure) {
        declarations_fallback(db, krate, "walk-discovered-unsupported-read");
    } else if declarations_certificate(db, &closure) != Some(key) {
        declarations_fallback(db, krate, "walk-certificate-changed");
    } else {
        match encode_declarations(db, krate.into(), &key, &value) {
            Err(why) => declarations_codec_error(db, krate, "encode", &why),
            Ok(_) if !declarations_reads_supported(db, &closure) => {
                declarations_fallback(db, krate, "encode-discovered-unsupported-read");
            }
            Ok(_) if declarations_certificate(db, &closure) != Some(key) => {
                declarations_fallback(db, krate, "encode-certificate-changed");
            }
            Ok(bytes) => {
                store.insert(&key, &bytes);
                declarations_fallback(db, krate, "write-attempt=1");
            }
        }
    }
    value
}

/// What a build did with what is frozen of the crates it resolved.
#[derive(Default)]
pub(crate) struct Served {
    /// The key of each crate that can be frozen.
    keys: FxHashMap<Crate, [u8; 32]>,
    /// The crates each [`Part`] of which came from the store, in the order of [`Part::ALL`].
    served: [Vec<Crate>; 3],
    /// The parts the store has but which name a file that is not loaded.
    unreadable: usize,
    mode: Option<Mode>,
}

/// The crates whose source is not all the toolchain's, registry's or checkout's: a file of
/// each has a text of blaze's (`--run-tool`'s std), which its key has.
static OVERLAID: Mutex<Vec<(Crate, &'static str)>> = Mutex::new(Vec::new());

/// Says that a file of `krate` has `text` in place of the one where its source is.
pub(crate) fn overlaid(krate: Crate, text: &'static str) {
    crate::lock(&OVERLAID).push((krate, text));
}

fn mode() -> Option<Mode> {
    match std::env::var("RA_JIT_FROZEN").as_deref() {
        Ok("off" | "0") => None,
        Ok("check") => Some(Mode::Check),
        Ok("verify") => Some(Mode::Verify),
        _ => Some(Mode::Serve),
    }
}

/// Where a crate's source is when it is one that cannot change: the part of the path from
/// the toolchain's source, the registry's or a git checkout's on.
fn immutable(path: &str) -> Option<&str> {
    ["/lib/rustlib/src/rust/", "/registry/src/", "/git/checkouts/"]
        .iter()
        .find_map(|marker| path.find(marker).map(|at| &path[at + marker.len()..]))
}

/// What stands for the directory a build script wrote to in a frozen crate's key: the part of
/// it from cargo's `build` directory on, whose `<package>-<hash>` cargo names by the package,
/// its features, profile and target, and not by where the checkout is.
fn build_output(out: &str) -> String {
    let path = out.replace('\\', "/");
    match path.rfind("/build/") {
        Some(at) => format!("<target>{}", &path[at..]),
        None => "<out>".to_owned(),
    }
}

/// The key of `krate`, when it and every crate it depends on cannot change.
fn key(
    db: &ide_db::RootDatabase,
    krate: Crate,
    keys: &mut FxHashMap<Crate, Option<[u8; 32]>>,
) -> Option<[u8; 32]> {
    if let Some(&it) = keys.get(&krate) {
        return it;
    }
    // A cycle, which only dev-dependencies make, is not frozen.
    keys.insert(krate, None);
    let found = key_of(db, krate, keys);
    keys.insert(krate, found);
    found
}

fn key_of(
    db: &ide_db::RootDatabase,
    krate: Crate,
    keys: &mut FxHashMap<Crate, Option<[u8; 32]>>,
) -> Option<[u8; 32]> {
    let root = crate::crate_root_file(db, krate.into())?;
    let root = root.to_str()?;
    let source = immutable(root)?;
    // The directory the crate's paths are under, which its environment names.
    let package = &root[..root.len() - source.len()];
    let data = krate.data(db);
    let mut hasher = Sha256::new();
    let mut field = |bytes: &[u8]| {
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    };
    field(b"ra-jit frozen def map 6");
    field(source.as_bytes());
    field(format!("{:?} {:?} {:?}", data.edition, data.target_kind, data.crate_attrs).as_bytes());
    let mut cfgs: Vec<String> =
        krate.cfg_options(db).into_iter().map(|it| it.to_string()).collect();
    cfgs.sort_unstable();
    field(cfgs.join("\n").as_bytes());
    for (_, text) in crate::lock(&OVERLAID).iter().filter(|it| it.0 == krate) {
        field(b"overlaid");
        field(text.as_bytes());
    }
    // A build script writes below the target directory of the checkout that ran it, so what it
    // wrote stands for where: two checkouts share the crate when its script wrote the same.
    let env = krate.env(db);
    let out = env.get("OUT_DIR");
    let out_stands_for = out.as_deref().map(build_output);
    // `CARGO` is wherever the cargo found on `PATH` is, which differs between a build script's
    // compile and the daemon's, and no crate's names are resolved by it.
    let mut vars: Vec<String> = env
        .iter()
        .filter(|(name, _)| *name != "CARGO")
        .map(|(name, value)| {
            let value = match (&out, &out_stands_for) {
                (Some(out), Some(stands_for)) => value.replace(out.as_str(), stands_for),
                _ => value.to_owned(),
            };
            format!("{name}={}", value.replace(package, "<source>/"))
        })
        .collect();
    vars.sort_unstable();
    field(vars.join("\n").as_bytes());
    if let Some(out) = &out {
        for (path, hash) in crate::content::written_to(db, krate, out) {
            field(path.as_bytes());
            field(&hash);
        }
    }
    let deps: Vec<_> =
        data.dependencies.iter().map(|it| (it.crate_id, it.name.to_string())).collect();
    for (dep, name) in deps {
        let dep_key = key(db, dep, keys)?;
        field(name.as_bytes());
        field(&dep_key);
    }
    Some(hasher.finalize().into())
}

/// What is frozen of a crate, each under its own key: the def map's is the crate's key.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    DefMap,
    Impls,
    LangItems,
}

impl Part {
    const ALL: [Part; 3] = [Part::DefMap, Part::Impls, Part::LangItems];

    fn name(self) -> &'static str {
        match self {
            Part::DefMap => "def maps",
            Part::Impls => "impls",
            Part::LangItems => "lang items",
        }
    }

    /// The key of this part of the crate whose key is `key`.
    fn key(self, key: &[u8; 32]) -> [u8; 32] {
        let tag: &[u8] = match self {
            Part::DefMap => return *key,
            Part::Impls => b"ra-jit frozen impls 1",
            Part::LangItems => b"ra-jit frozen lang items 1",
        };
        let mut hasher = Sha256::new();
        hasher.update(tag);
        hasher.update(key);
        hasher.finalize().into()
    }

    /// Why `bytes` cannot be read in this database: a file they name is not loaded.
    fn unreadable(self, db: &ide_db::RootDatabase, krate: Crate, bytes: &[u8]) -> Option<String> {
        if self != Part::Impls {
            return hir_frozen::unreadable(db, krate, bytes);
        }
        // The impls are two parts, each naming its files.
        let len = bytes.get(..4).and_then(|it| <[u8; 4]>::try_from(it).ok());
        let Some(len) = len else { return Some("no length".to_owned()) };
        let first = 4 + u32::from_le_bytes(len) as usize;
        let (Some(traits), Some(inherent)) = (bytes.get(4..first), bytes.get(first..)) else {
            return Some("a part past the end".to_owned());
        };
        [traits, inherent].into_iter().find_map(|part| hir_frozen::unreadable(db, krate, part))
    }

    fn set(self, db: &mut ide_db::RootDatabase, maps: FxHashMap<Crate, Frozen>) {
        match self {
            Part::DefMap => hir_frozen::set(db, maps),
            Part::Impls => impls_frozen::set(db, maps),
            Part::LangItems => hir_frozen::set_lang_items(db, maps),
        }
    }

    fn encode(self, db: &ide_db::RootDatabase, krate: Crate) -> Result<Vec<u8>, String> {
        hir::attach_db(db, || match self {
            Part::DefMap => hir_frozen::encode(db, krate),
            Part::Impls => impls_frozen::encode(db, krate),
            Part::LangItems => hir_frozen::encode_lang_items(db, krate),
        })
    }

    /// Records a difference between what was read and what is live; the def map's is
    /// checked as it is resolved.
    fn check(self, db: &ide_db::RootDatabase, krate: Crate) {
        hir::attach_db(db, || match self {
            Part::DefMap => {}
            Part::Impls => impls_frozen::check(db, krate),
            Part::LangItems => hir_frozen::check_lang_items(db, krate),
        })
    }
}

/// Has name resolution, finding impls and finding lang items read what the store has of
/// `crates` and their dependencies.
pub(crate) fn serve(db: &mut ide_db::RootDatabase, crates: &[Crate]) -> Served {
    let Some(mode) = mode() else { return Served::default() };
    let mut all: FxHashMap<Crate, Option<[u8; 32]>> = FxHashMap::default();
    for &krate in crates {
        for dep in krate.transitive_deps(db) {
            key(db, dep, &mut all);
        }
    }
    let keys: FxHashMap<Crate, [u8; 32]> =
        all.into_iter().filter_map(|(krate, key)| Some((krate, key?))).collect();
    let mut served = Served { keys, mode: Some(mode), ..Served::default() };
    let Some(store) = ra_jit_store::Store::global() else { return served };
    for (index, part) in Part::ALL.into_iter().enumerate() {
        let mut maps = FxHashMap::default();
        for (&krate, key) in &served.keys {
            // A part is read only with the def map it was found from.
            if part != Part::DefMap && !served.served[0].contains(&krate) {
                continue;
            }
            let Some(bytes) = store.get(&part.key(key)) else { continue };
            if part.unreadable(db, krate, &bytes).is_some() {
                served.unreadable += 1;
                continue;
            }
            maps.insert(krate, Frozen { bytes: bytes.into(), mode });
            served.served[index].push(krate);
        }
        part.set(db, maps);
    }
    served
}

/// Writes to the store what can be frozen of the crates that were resolved, and fails on a
/// difference between what was frozen and what is live.
pub(crate) fn store(db: &ide_db::RootDatabase, served: &Served) -> anyhow::Result<()> {
    if served.mode != Some(Mode::Serve) {
        for (index, part) in Part::ALL.into_iter().enumerate() {
            served.served[index].iter().for_each(|&krate| part.check(db, krate));
        }
    }
    let differences = hir_frozen::differences();
    if !differences.is_empty() {
        anyhow::bail!("what was frozen differs:\n{}", differences.join("\n"));
    }
    let Some(store) = served.mode.and(ra_jit_store::Store::global()) else { return Ok(()) };
    let start = std::time::Instant::now();
    let mut unstored = Vec::new();
    for (index, part) in Part::ALL.into_iter().enumerate() {
        for (&krate, key) in &served.keys {
            let key = part.key(key);
            if !served.served[index].contains(&krate) && !store.has(&key) {
                unstored.push((part, krate, key));
            }
        }
    }
    if !unstored.is_empty() {
        store.settled();
    }
    // On every core: a cold build has a thousand to write, and waits for them.
    let (stored, refused) = (AtomicUsize::new(0), Mutex::new(FxHashMap::default()));
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(1, |it| it.get());
    std::thread::scope(|scope| {
        for _ in 0..threads.min(unstored.len()) {
            let (db, unstored, next) = (db.clone(), &unstored, &next);
            let (stored, refused) = (&stored, &refused);
            scope.spawn(move || {
                while let Some(&(part, krate, key)) = unstored.get(next.fetch_add(1, Relaxed)) {
                    match part.encode(&db, krate) {
                        Ok(bytes) => {
                            store.insert(&key, &bytes);
                            stored.fetch_add(1, Relaxed);
                        }
                        Err(why) => {
                            let mut refused = refused.lock().unwrap_or_else(|e| e.into_inner());
                            *refused.entry(format!("{} with {why}", part.name())).or_default() += 1;
                        }
                    }
                }
            });
        }
    });
    let (stored, refused): (usize, FxHashMap<String, usize>) =
        (stored.into_inner(), refused.into_inner().unwrap_or_else(|e| e.into_inner()));
    if stored > 0 {
        store.flush();
    }
    let from: Vec<String> = Part::ALL
        .iter()
        .zip(&served.served)
        .map(|(part, crates)| format!("{} of {}", part.name(), crates.len()))
        .collect();
    let mut said = format!(
        "frozen, of {} crates: {} from the store, {stored} written in {:.1?}",
        served.keys.len(),
        from.join(", "),
        start.elapsed()
    );
    if served.unreadable > 0 {
        said += &format!(", {} naming a file not loaded", served.unreadable);
    }
    if !refused.is_empty() {
        let mut refused: Vec<_> = refused.into_iter().collect();
        refused.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let why: Vec<String> = refused.iter().map(|(why, n)| format!("{n} {why}")).collect();
        said += &format!(", not frozen: {}", why.join(", "));
    }
    eprintln!("{said}");
    Ok(())
}
