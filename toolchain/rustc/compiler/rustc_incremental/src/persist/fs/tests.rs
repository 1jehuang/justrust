use super::*;
use rustc_data_structures::flock::LockedDir;

#[test]
fn retained_paths_do_not_retain_session_locks() {
    let dir = rustc_fs_util::TempDirBuilder::new().tempdir_in(std::env::temp_dir()).unwrap();
    let old_path = dir.path().join("old");
    let new_path = dir.path().join("working");
    let old_lock = dir.path().join("old.lock");
    let new_lock = dir.path().join("working.lock");
    let paths = IncrCompSessionPaths {
        old_session_directory: Some(old_path.clone()),
        new_session_directory: new_path.clone(),
    };
    let session = IncrCompSession {
        old_session_directory: Some(
            LockedDir::try_lock(old_path.clone(), &old_lock, true, true).unwrap(),
        ),
        new_session_directory: LockedDir::try_lock(new_path.clone(), &new_lock, true, true)
            .unwrap(),
        paths,
    };
    let retained = session.paths.clone();
    assert!(LockedDir::try_lock(old_path.clone(), &old_lock, false, true).is_err());
    assert!(LockedDir::try_lock(new_path.clone(), &new_lock, false, true).is_err());

    drop(session);

    let _old = LockedDir::try_lock(old_path.clone(), &old_lock, false, true).unwrap();
    let _new = LockedDir::try_lock(new_path.clone(), &new_lock, false, true).unwrap();
    assert_eq!(old_dep_graph_path(&retained), Some(old_path.join(DEP_GRAPH_FILENAME)));
    assert_eq!(dep_graph_path(&retained), new_path.join(DEP_GRAPH_FILENAME));
    assert_eq!(staging_dep_graph_path(&retained), new_path.join(STAGING_DEP_GRAPH_FILENAME));
}

#[test]
fn paths_without_previous_session_have_no_old_artifacts() {
    let paths = IncrCompSessionPaths {
        old_session_directory: None,
        new_session_directory: PathBuf::from("working"),
    };
    assert_eq!(old_dep_graph_path(&paths), None);
    assert_eq!(old_work_products_path(&paths), None);
    assert_eq!(old_query_cache_path(&paths), None);
    assert_eq!(work_products_path(&paths), PathBuf::from("working").join(WORK_PRODUCTS_FILENAME));
    assert_eq!(query_cache_path(&paths), PathBuf::from("working").join(QUERY_CACHE_FILENAME));
}

#[test]
fn test_timestamp_serialization() {
    for i in 0..1_000u64 {
        let time = UNIX_EPOCH + Duration::new(i * 1_434_578, (i as u32) * 239_000);
        let s = timestamp_to_string(time);
        assert_eq!(Ok(time), string_to_timestamp(&s));
    }
}

#[test]
fn test_find_source_directory_in_iter() {
    // Find newest
    assert_eq!(
        find_source_directory_in_iter(
            [
                PathBuf::from("crate-dir/s-3234-0000-svh"),
                PathBuf::from("crate-dir/s-2234-0000-svh"),
                PathBuf::from("crate-dir/s-1234-0000-svh")
            ]
            .into_iter(),
        ),
        Some(PathBuf::from("crate-dir/s-3234-0000-svh"))
    );

    // Filter out "-working"
    assert_eq!(
        find_source_directory_in_iter(
            [
                PathBuf::from("crate-dir/s-3234-0000-working"),
                PathBuf::from("crate-dir/s-2234-0000-svh"),
                PathBuf::from("crate-dir/s-1234-0000-svh")
            ]
            .into_iter(),
        ),
        Some(PathBuf::from("crate-dir/s-2234-0000-svh"))
    );

    // Handle empty
    assert_eq!(find_source_directory_in_iter([].into_iter()), None);

    // Handle only working
    assert_eq!(
        find_source_directory_in_iter(
            [
                PathBuf::from("crate-dir/s-3234-0000-working"),
                PathBuf::from("crate-dir/s-2234-0000-working"),
                PathBuf::from("crate-dir/s-1234-0000-working")
            ]
            .into_iter(),
        ),
        None
    );
}

#[test]
fn test_replace_hard_link_with_copy_unshares_hard_link() {
    let dir = rustc_fs_util::TempDirBuilder::new().tempdir_in(std::env::temp_dir()).unwrap();
    let file = dir.path().join("file");
    let link = dir.path().join("link");
    std_fs::write(&file, b"original").unwrap();
    std_fs::hard_link(&file, &link).unwrap();

    replace_hard_link_with_copy(&link).unwrap();

    std_fs::write(&file, b"changed").unwrap();
    assert_eq!(std_fs::read(&link).unwrap(), b"original");
    assert!(!link.with_added_extension("tmp").exists());
}

#[test]
fn lineage_directories_of_one_crate_id_are_told_from_other_crates() {
    let same = |name: &str| jr_same_crate_id("mid-0abc", std::ffi::OsStr::new(name));
    // The unnamed lineage and named ones.
    assert!(same("mid-0abc"));
    assert!(same("mid-0abc-1f2e3d4c5b6a7988"));
    // Another id, another crate whose name starts the same, and names that
    // are not a lineage.
    assert!(!same("mid-0abd"));
    assert!(!same("mid-0abd-1f2e3d4c5b6a7988"));
    assert!(!same("mid-0abcd"));
    assert!(!same("mid-0abc-"));
    assert!(!same("mid-0abc-1f2e-3d4c"));
    assert!(!same("mid_extra-0abc"));
    assert!(!same("mi"));
}

#[test]
fn a_new_lineage_is_seeded_from_the_newest_sibling_session_of_its_crate_id() {
    let dir = rustc_fs_util::TempDirBuilder::new().tempdir_in(std::env::temp_dir()).unwrap();
    let incr = dir.path();
    let session = |crate_dir: &str, name: &str| {
        let path = incr.join(crate_dir).join(name);
        std_fs::create_dir_all(&path).unwrap();
        std_fs::write(lock_file_path(&path), b"").unwrap();
        path
    };
    let own = incr.join("mid-0abc-new");
    std_fs::create_dir_all(&own).unwrap();
    // Nothing to start from yet.
    assert!(jr_sibling_source(&own, "mid-0abc").is_none());

    session("mid-0abc-old", "s-1234-0000-svh");
    let newest = session("mid-0abc", "s-3234-0000-svh");
    // Not candidates: unfinished, another crate id, another crate, and this
    // lineage's own directory.
    session("mid-0abc-busy", "s-9234-0000-working");
    session("mid-0abd-other", "s-8234-0000-svh");
    session("midway-0abc", "s-7234-0000-svh");
    session("mid-0abc-new", "s-6234-0000-working");

    let seed = jr_sibling_source(&own, "mid-0abc").expect("a sibling session");
    assert_eq!(&*seed, newest.as_path());
    drop(seed);

    // A session whose owner holds it exclusively (being deleted) is skipped,
    // not waited for.
    let deleting =
        LockedDir::try_lock(newest.clone(), &lock_file_path(&newest), false, true).unwrap();
    assert!(jr_sibling_source(&own, "mid-0abc").is_none());
    drop(deleting);
    assert!(jr_sibling_source(&own, "mid-0abc").is_some());
}
