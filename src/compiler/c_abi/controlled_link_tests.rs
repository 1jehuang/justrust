use super::*;
#[test]
fn controlled_link_descriptor_metadata_is_reproducible_across_roots() {
    let directory = Temp::new();
    let source = directory.0.join("metadata.c");
    let executable = directory.0.join("metadata");
    fs::write(&source, r#"
static long call(long n,long a,long b,long c){long r;__asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c):"rcx","r11","memory");return r;}
void _start(void){
 unsigned char bytes[144];long fd=call(2,(long)"input",0,0);
 if(fd<0||call(5,fd,(long)bytes,0)||call(1,1,(long)bytes,144)!=144)call(60,1,0,0);
 if(call(5,1,(long)bytes,0)||call(1,1,(long)bytes,144)!=144)call(60,2,0,0);
 if(call(5,2,(long)bytes,0)||call(1,1,(long)bytes,144)!=144)call(60,3,0,0);
 if(call(5,0,(long)bytes,0)||call(1,1,(long)bytes,144)!=144)call(60,4,0,0);
 call(60,0,0,0);
}
"#).unwrap();
    let compiled = Command::new("cc")
        .args([
            "-nostdlib",
            "-static",
            "-fno-stack-protector",
            "-no-pie",
            "-O2",
        ])
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let mut results = Vec::new();
    for (name, modified) in [("first", 1), ("second", 999)] {
        let root = directory.0.join(name);
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("out")).unwrap();
        fs::copy(&executable, root.join("probe")).unwrap();
        fs::write(root.join("input"), b"frozen").unwrap();
        File::open(root.join("input"))
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(modified))
            .unwrap();
        let mut command = Command::new(root.join("probe"));
        command.env_clear();
        let output = super::super::link_sandbox_run::output(&mut command, &root).unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout.len(), 4 * 144);
        for bytes in output.stdout.chunks_exact(144) {
            assert_eq!(&bytes[28..48], &[0; 20]);
            assert_eq!(&bytes[72..], &[0; 72]);
        }
        results.push(output.stdout);
    }
    assert_eq!(results[0], results[1]);
    assert_ne!(&results[0][144..160], &results[0][288..304]);
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn controlled_link_listener_transfer_rejects_missing_rights_and_nonlistener() {
    use std::os::unix::net::UnixDatagram;
    let (sender, receiver) = UnixDatagram::pair().unwrap();
    sender.send(b"x").unwrap();
    assert!(super::super::link_sandbox_io::receive(&receiver).is_err());
    sender.send(b"too long").unwrap();
    assert!(super::super::link_sandbox_io::receive(&receiver).is_err());
    let file = File::open("/dev/null").unwrap();
    unsafe { super::super::link_sandbox_run::send_listener(sender.as_raw_fd(), file.as_raw_fd()) }
        .unwrap();
    let descriptor = super::super::link_sandbox_io::receive(&receiver).unwrap();
    assert!(super::super::link_sandbox_io::bootstrap_exec(&descriptor).is_err());
}
#[test]
fn controlled_link_output_acquisition_rejects_symlinks_and_fifos() {
    let directory = Temp::new();
    let input = directory.0.join("input");
    let output = directory.0.join("output");
    fs::write(&input, b"frozen").unwrap();
    std::os::unix::fs::symlink(&input, &output).unwrap();
    assert_eq!(
        acquire_regular(&output, 16, "captured input").unwrap(),
        b"frozen"
    );
    assert!(acquire_output(&output, 16, "native executable").is_err());
    fs::remove_file(&output).unwrap();
    fs::write(&output, b"regular").unwrap();
    assert_eq!(
        acquire_output(&output, 16, "native executable").unwrap(),
        b"regular"
    );
    fs::remove_file(&output).unwrap();
    unsafe extern "C" {
        fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32;
    }
    let path = std::ffi::CString::new(output.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { mkfifo(path.as_ptr(), 0o600) }, 0);
    assert!(acquire_output(&output, 16, "native executable").is_err());
}
#[test]
fn controlled_link_reproducible_across_staged_roots() {
    let directory = Temp::new();
    let source = directory.0.join("main.c");
    let object = directory.0.join("main.o");
    fs::write(&source, "int main(void) { return 0; }\n").unwrap();
    let compiled = Command::new("cc")
        .args(["-c", "-fno-ident", "-o"])
        .arg(&object)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let capsule = ControlledToolchain::capture(&[]).unwrap();
    let own = fs::read(object).unwrap();
    let first = directory.0.join("first");
    let second = directory.0.join("second");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&second).unwrap();
    let a = capsule.execute(&first, &[], &own, &[]).unwrap();
    let b = capsule.execute(&second, &[], &own, &[]).unwrap();
    assert_eq!(a, b);
}
use std::{
    fs,
    os::unix::{fs::PermissionsExt, io::AsRawFd},
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "jr-controlled-link-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn controlled_link_bounded_reader_probes_growth_and_propagates_errors() {
    struct Growing(usize);
    impl Read for Growing {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            bytes.fill(1);
            self.0 += bytes.len();
            Ok(bytes.len())
        }
    }
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected read failure",
            ))
        }
    }
    assert_eq!(bounded_read(&b"abc"[..], 3).unwrap(), b"abc");
    assert_eq!(bounded_read(&b"ab"[..], 3).unwrap(), b"ab");
    assert!(bounded_read(&b""[..], 0).unwrap().is_empty());
    assert!(matches!(
        bounded_read(&b"x"[..], 0),
        Err(AcquisitionError::Limit)
    ));
    assert!(matches!(
        bounded_read(Broken, usize::MAX),
        Err(AcquisitionError::Limit)
    ));
    let mut growing = Growing(0);
    assert!(matches!(
        bounded_read(&mut growing, 3),
        Err(AcquisitionError::Limit)
    ));
    assert_eq!(growing.0, 4);
    assert!(
        matches!(bounded_read(Broken, 3), Err(AcquisitionError::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied)
    );
}

#[test]
fn controlled_link_regular_handle_bounds_growth_and_ignores_path_replacement() {
    use std::io::Write;
    let temp = Temp::new();
    unsafe extern "C" {
        fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32;
    }
    let fifo = temp.0.join("fifo");
    let fifo_name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
    assert!(matches!(
        open_regular(&fifo, 3),
        Err(AcquisitionError::NotRegular)
    ));
    let path = temp.0.join("input");
    fs::write(&path, b"abc").unwrap();
    let opened = open_regular(&path, 3).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"x")
        .unwrap();
    assert!(matches!(
        bounded_read(opened, 3),
        Err(AcquisitionError::Limit)
    ));
    fs::write(&path, b"abc").unwrap();
    let opened = open_regular(&path, 3).unwrap();
    fs::rename(&path, temp.0.join("original")).unwrap();
    fs::write(&path, b"replacement exceeds limit").unwrap();
    assert_eq!(bounded_read(opened, 3).unwrap(), b"abc");
    assert!(matches!(
        open_regular(&temp.0, 3),
        Err(AcquisitionError::NotRegular)
    ));
    assert!(matches!(
        open_regular(Path::new("/dev/null"), 3),
        Err(AcquisitionError::NotRegular)
    ));
    assert!(
        matches!(open_regular(&temp.0.join("missing"), 3), Err(AcquisitionError::Io(error)) if error.kind() == io::ErrorKind::NotFound)
    );
    let output = temp.0.join("program");
    fs::write(&output, b"abc").unwrap();
    assert_eq!(
        acquire_regular(&output, 3, "native executable").unwrap(),
        b"abc"
    );
    File::options()
        .write(true)
        .open(&output)
        .unwrap()
        .set_len(MAX_EXECUTABLE_BYTES as u64 + 1)
        .unwrap();
    assert!(
        acquire_regular(&output, MAX_EXECUTABLE_BYTES, "native executable")
            .unwrap_err()
            .starts_with("resource limit:")
    );
}

#[test]
fn controlled_link_capsule_aggregate_budget_includes_all_identity_framing() {
    use std::io::Write;
    fn empty() -> ControlledToolchain {
        ControlledToolchain {
            files: BTreeMap::new(),
            library_dir: "/libs".into(),
            libraries: vec!["c".into(), "m".into()],
        }
    }
    fn archive(path: &Path, size: usize) {
        let mut file = File::create(path).unwrap();
        file.write_all(b"!<arch>\n").unwrap();
        file.set_len(size as u64).unwrap();
    }
    let temp = Temp::new();
    let mut capsule = empty();
    let framing = 9 + 8 + capsule.policy().len() + 8 + 5 + 16 + 2 * (8 + 1) + 16;
    assert_eq!(capsule.identity_bytes(), Some(framing));
    let first = temp.0.join("first.a");
    let second = temp.0.join("second.a");
    archive(&first, 7 * 1024 * 1024);
    archive(&second, 7 * 1024 * 1024);
    capsule.capture_file(&first).unwrap();
    assert!(
        capsule
            .capture_file(&second)
            .unwrap_err()
            .starts_with("resource limit:")
    );
    assert_eq!(capsule.files.len(), 1);
    assert!(capsule.identity_bytes().unwrap() < MAX_CAPSULE_IDENTITY_BYTES);
    let mut exact = empty();
    let path = temp.0.join("exact.a");
    let remaining =
        MAX_CAPSULE_IDENTITY_BYTES - framing - 16 - path.as_os_str().as_encoded_bytes().len();
    archive(&path, remaining);
    exact.capture_file(&path).unwrap();
    assert_eq!(exact.identity_bytes(), Some(MAX_CAPSULE_IDENTITY_BYTES));
    exact.capture_file(&path).unwrap();
    assert!(
        exact
            .capture_file(&second)
            .unwrap_err()
            .starts_with("resource limit:")
    );
    assert_eq!(exact.files.len(), 1);
    let real = ControlledToolchain::capture(&["c".into(), "m".into()]).unwrap();
    assert!(real.identity_bytes().unwrap() < MAX_CAPSULE_IDENTITY_BYTES);
}
#[test]
fn controlled_link_input_receipt_rejects_capsule_byte_path_and_selection_mutation() {
    use crate::compiler::{
        expansion_budget::ExpansionBudget,
        source::{self, SourceOptions, SourceSnapshot},
    };
    let mut options = SourceOptions {
        source_snapshot: Some(
            SourceSnapshot::new(
                "/frozen".into(),
                [("main.rs".into(), b"fn main() {}".to_vec())],
            )
            .unwrap(),
        ),
        prelude: false,
        ..SourceOptions::default()
    };
    options.native_link.toolchain = Some(ControlledToolchain::capture(&[]).unwrap());
    let unit = source::prepare_root_unit(
        Path::new("/frozen/main.rs"),
        &options,
        &ExpansionBudget::default(),
    )
    .unwrap();
    assert!(unit.inputs.matches_current_source(&options));
    let mut changed = options.clone();
    changed.native_link.toolchain = None;
    assert!(!unit.inputs.matches_current_source(&changed));
    let mut changed = options.clone();
    changed
        .native_link
        .toolchain
        .as_mut()
        .unwrap()
        .files
        .get_mut(Path::new("/usr/bin/ld"))
        .unwrap()[0] ^= 1;
    assert!(!unit.inputs.matches_current_source(&changed));
    let mut changed = options.clone();
    let capsule = changed.native_link.toolchain.as_mut().unwrap();
    let bytes = capsule.files.remove(Path::new("/usr/bin/ld")).unwrap();
    capsule.files.insert("/usr/bin/other-ld".into(), bytes);
    assert!(!unit.inputs.matches_current_source(&changed));
    let mut changed = options.clone();
    changed.native_link.toolchain.as_mut().unwrap().library_dir =
        "/not-the-captured-selection".into();
    assert!(!unit.inputs.matches_current_source(&changed));
    let mut changed = options.clone();
    changed
        .native_link
        .toolchain
        .as_mut()
        .unwrap()
        .libraries
        .push("c".into());
    assert!(!unit.inputs.matches_current_source(&changed));
    let mut changed = options.clone();
    changed
        .native_link
        .toolchain
        .as_mut()
        .unwrap()
        .files
        .insert("/oversize".into(), vec![0; 16 * 1024 * 1024]);
    assert!(!unit.inputs.matches_current_source(&changed));
    assert!(unit.inputs.matches_current_source(&options));
}
#[test]
fn controlled_link_capture_is_immutable_and_rejects_untracked_library_and_paths() {
    let directory = Temp::new();
    let path = directory.0.join("captured.o");
    let bytes = fs::read("/usr/bin/ld").unwrap();
    fs::write(&path, &bytes).unwrap();
    let mut captured = ControlledToolchain::capture(&[]).unwrap();
    captured.capture_file(&path).unwrap();
    fs::write(&path, b"changed after capture").unwrap();
    fs::remove_file(&path).unwrap();
    assert_eq!(captured.files[&path], bytes);
    assert!(
        ControlledToolchain::capture(&["pthread".into()])
            .unwrap_err()
            .contains("unsupported")
    );
    assert!(
        captured
            .capture_file(Path::new("/usr/../etc/passwd"))
            .unwrap_err()
            .contains("noncanonical")
    );
    assert!(
        captured
            .capture_file(Path::new("relative.o"))
            .unwrap_err()
            .contains("noncanonical")
    );
    for size in 0..64 {
        assert!(elf_dependencies(&vec![0; size]).is_err());
    }
    assert!(elf_dependencies(b"!<thin>\n").is_err());
}
#[test]
fn controlled_link_boundary_denies_host_files_mutation_descriptors_network_and_processes() {
    let directory = Temp::new();
    let root = directory.0.join("root");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("out")).unwrap();
    fs::write(root.join("input"), b"frozen").unwrap();
    let secret = directory.0.join("secret");
    fs::write(&secret, b"not an input").unwrap();
    let leaked = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&secret)
        .unwrap();
    unsafe extern "C" {
        fn fcntl(fd: i32, command: i32, ...) -> i32;
    }
    // Deliberately leak a non-CLOEXEC host descriptor. The boundary, not
    // cooperation by the invoking process, must close it before tool exec.
    assert_eq!(unsafe { fcntl(leaked.as_raw_fd(), 2, 0) }, 0);
    std::os::unix::fs::symlink(&secret, root.join("escape")).unwrap();
    let alias = directory.0.join("outside-alias");
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    let missing = directory.0.join("missing-host-path");
    let source = directory.0.join("probe.c");
    fs::write(&source, format!(r#"
static long call(long n,long a,long b,long c){{long r;__asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c):"rcx","r11","memory");return r;}}
static long call6(long n,long a,long b,long c,long d,long e,long f){{register long x __asm__("r10")=d;register long y __asm__("r8")=e;register long z __asm__("r9")=f;long r;__asm__ volatile("syscall":"=a"(r):"a"(n),"D"(a),"S"(b),"d"(c),"r"(x),"r"(y),"r"(z):"rcx","r11","memory");return r;}}
static const unsigned char readonly_stat[144]={{0}};
static void die(long code){{call(60,code,0,0);__builtin_unreachable();}}
void _start(void){{
 char byte;
 if(call(2,(long){secret:?},0,0)!=-13)die(1);
 if(call(2,(long){secret:?},1,0)!=-13)die(15);
 if(call(2,(long){secret:?},0x200000,0)!=-13)die(29);
 if(call(2,(long){missing:?},0,0)!=-13)die(32);
 if(call(2,(long){alias:?},0,0)!=-13)die(33);
 if(call(257,-100,(long){secret:?},0)!=-13)die(34);
 if(call(257,-100,(long){missing:?},0)!=-13)die(35);
 if(call(257,123,(long)"input",0)!=-13)die(36);
 if(call(2,(long)"../secret",0,0)!=-13)die(37);
 if(call(2,(long)"out/../input",1,0)!=-13)die(38);
 if(call(59,(long){secret:?},0,0)!=-13)die(39);
 if(call(59,(long){missing:?},0,0)!=-13)die(40);
 if(call(2,(long)"escape",0,0)!=-40)die(16);
 if(call(2,(long)"input",1,0)!=-13)die(2);
 if(call(2,(long)"input",2,0)!=-13)die(41);
 if(call(2,(long)"input",0x200,0)!=-13)die(42);
 if(call(2,(long)"input",0x40,0600)!=-13)die(43);
 if(call(2,(long)"input",0x400,0)!=-13)die(44);
 if(call(2,(long)"out",0x410002,0600)!=-13)die(45);
 if(call(2,(long)"out/async",0x2041,0600)!=-13)die(46);
 if(call(2,(long)"input",0x4000,0)!=-13)die(66);
 if(call(76,(long)"input",0,0)!=-1)die(17);
 if(call(82,(long)"input",(long)"out/stolen",0)!=-1)die(18);
 if(call(86,(long)"input",(long)"out/linked",0)!=-1)die(19);
 if(call(87,(long)"input",0,0)!=-1)die(20);
 char metadata[256];
 if(call(4,(long){secret:?},(long)metadata,0)!=-1)die(21);
 if(call(6,(long){secret:?},(long)metadata,0)!=-1)die(22);
 if(call(21,(long){secret:?},0,0)!=-1)die(23);
 if(call(89,(long)"escape",(long)metadata,256)!=-1)die(24);
 if(call(90,(long){secret:?},0600,0)!=-1)die(25);
 if(call(80,(long)"/",0,0)!=-1)die(26);
 if(call(16,1,0,0)!=-1)die(27);
 if(call(133,(long)"out/fifo",0010600,0)!=-1)die(28);
 if(call(0,{leaked},(long)&byte,1)!=-9)die(4);
 if(call(1,{leaked},(long)"x",1)!=-9)die(30);
 if(call(77,{leaked},0,0)!=-9)die(31);
 long fd=call(2,(long)"input",0,0);if(fd<0||call(0,fd,(long)&byte,1)!=1||byte!='f')die(3);
 if(call(72,fd,4,0x4000)!=-1)die(83);
 if(call(72,fd,4,0x2000)!=-1)die(84);
 if(call(72,fd,4,0)!=-1)die(85);
 if(call(72,fd,3,0)<0)die(86);
 union {{unsigned long aligned;unsigned char bytes[144];}} statbuf,duplicate;
 if(call(5,fd,(long)statbuf.bytes,0)!=0)die(47);
 if(*(unsigned long*)(statbuf.bytes+48)!=6||*(unsigned long*)(statbuf.bytes+16)!=1)die(48);
 if(*(unsigned int*)(statbuf.bytes+24)!=0100444)die(49);
 for(long i=28;i<48;i++)if(statbuf.bytes[i])die(50);
 for(long i=72;i<144;i++)if(statbuf.bytes[i])die(51);
 long dup=call(32,fd,0,0);if(dup<0||call(5,dup,(long)duplicate.bytes,0)!=0)die(52);
 for(long i=0;i<144;i++)if(statbuf.bytes[i]!=duplicate.bytes[i])die(53);
 if(call(5,fd,(long)readonly_stat,0)!=-14)die(54);
 if(call(5,fd,0,0)!=-14)die(55);
 if(call(5,999999,(long)statbuf.bytes,0)!=-9)die(56);
 if(call(78,fd,(long)statbuf.bytes,144)!=-1)die(57);
 if(call(217,fd,(long)statbuf.bytes,144)!=-1)die(58);
 if(call(8,fd,0,3)!=-1||call(8,fd,0,4)!=-1)die(59);
 if(call(2,(long)".",0x10000,0)!=-13)die(60);
 long region=call6(9,0,8192,3,0x22,-1,0);if(region<0)die(61);
 if(call(10,region+4096,4096,0)!=0)die(62);
 char *edge=(char*)(region+4090);edge[0]='i';edge[1]='n';edge[2]='p';edge[3]='u';edge[4]='t';edge[5]=0;
 if(call(2,region+4096,0,0)!=-14)die(63);
 long edgefd=call(2,(long)edge,0,0);if(edgefd<0)die(64);
 if(call(28,region,4096,100)!=-1||call(28,region,4096,101)!=-1)die(67);
 if(call(28,region,4096,9999)!=-1)die(68);
 if(call(27,region,4096,(long)&byte)!=-1)die(69);
 if(call(28,region,4096,0)!=0)die(70);
 if(call(11,region,8192,0)!=0)die(65);
 if(call(57,0,0,0)!=-1)die(5);
 if(call(435,0,0,0)!=-1)die(6);
 if(call(41,2,1,0)!=-1)die(7);
 if(call(101,0,0,0)!=-1)die(8);
 if(call(272,0x10000000,0,0)!=-1)die(9);
 if(call(161,(long)"/",0,0)!=-1)die(10);
 if(call(0x40000000+39,0,0,0)!=-1)die(11);
 if(call(302,1,0,0)!=-1)die(13);
 // Raw syscalls deliberately bypass libc/vDSO. Valid writable output buffers
 // and invalid arguments must both receive uniform EPERM without writes.
 static const long observations[]={{39,63,96,97,98,99,100,102,104,107,108,110,115,118,120,121,124,125,186,201,204,228,229,309}};
 volatile unsigned char observation[512];
 for(long i=0;i<512;i++)observation[i]=0xa5;
 for(long i=0;i<(long)(sizeof observations/sizeof observations[0]);i++){{
  long n=observations[i];
  if(call(n,(long)observation,(long)observation,(long)observation)!=-1)die(71);
  if(call(n,0,0,0)!=-1)die(72);
 }}
 if(call(97,3,(long)observation,0)!=-1)die(82);
 if(call(98,0,(long)observation,0)!=-1)die(73);
 if(call(204,0,512,(long)observation)!=-1)die(74);
 if(call(228,0,(long)observation,0)!=-1)die(75);
 if(call(229,0,(long)observation,0)!=-1)die(76);
 if(call6(302,0,3,0,(long)observation,0,0)!=-1)die(77);
 if(call6(302,0,3,(long)observation,0,0,0)!=-1)die(78);
 // No kernel entropy is an admitted input either. AT_RANDOM is a separate
 // auxv limitation, not a reason to allow direct getrandom observations.
 if(call(318,(long)observation,512,0)!=-1)die(80);
 if(call(318,(long)observation,512,1)!=-1)die(81);
 for(long i=0;i<512;i++)if(observation[i]!=0xa5)die(79);
 if(call(72,1,8,1)!=-1)die(14);
 fd=call(2,(long)"out/allowed",65,0600);if(fd<0||call(1,fd,(long)"ok",2)!=2)die(12);
 call(1,1,(long)"confined\n",9);die(0);
}}
"#, secret = secret.to_str().unwrap(), missing = missing.to_str().unwrap(), alias = alias.to_str().unwrap(), leaked = leaked.as_raw_fd())).unwrap();
    let compiled = Command::new("cc")
        .args([
            "-nostdlib",
            "-static",
            "-fno-stack-protector",
            "-no-pie",
            "-O2",
        ])
        .arg(&source)
        .arg("-o")
        .arg(root.join("probe"))
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let mut command = Command::new(root.join("probe"));
    command.env_clear();
    let output = super::super::link_sandbox_run::output(&mut command, &root)
        .unwrap_or_else(|error| panic!("{}", super::super::link_sandbox::setup_error(&error)));
    assert!(
        output.status.success(),
        "boundary probe {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"confined\n");
    assert_eq!(fs::read(root.join("out/allowed")).unwrap(), b"ok");
    assert_eq!(fs::read(root.join("input")).unwrap(), b"frozen");
    assert_eq!(fs::read(secret).unwrap(), b"not an input");
}
#[test]
fn controlled_link_fails_closed_without_capsule_and_on_invalid_native_inputs() {
    use crate::compiler::{
        c_abi::elf::{self, LinkOptions},
        ir::PlanBudget,
    };
    let budget = PlanBudget::default();
    // Object emission is independent of host toolchain availability.
    let functions = vec![(
        "main".into(),
        crate::compiler::backend::Object {
            code: vec![0xc3],
            calls: vec![],
            strings: vec![],
            exports: vec![],
            vtables: vec![],
            receipt: None,
        },
    )];
    assert!(
        elf::link(&functions, &LinkOptions::default(), &budget)
            .unwrap_err()
            .contains("captured controlled toolchain")
    );
    let directory = Temp::new();
    let toolchain = ControlledToolchain::capture(&[]).unwrap();
    let error = toolchain
        .execute(
            &directory.0,
            &[b"!<thin>\n/tmp/host.o".to_vec()],
            b"invalid own object",
            &[],
        )
        .unwrap_err();
    assert!(error.contains("regular ar archive snapshot"), "{error}");
}

#[test]
fn controlled_link_rejects_plugin_only_lto_instead_of_discarding_native_constructor() {
    use crate::compiler::{
        c_abi::elf::{self, LinkOptions},
        ir::PlanBudget,
    };
    let directory = Temp::new();
    let source = directory.0.join("constructor.c");
    let object = directory.0.join("constructor.o");
    fs::write(&source, "extern void side_effect(void); __attribute__((constructor)) static void initialize(void){side_effect();}\n").unwrap();
    let compiled = Command::new("cc")
        .args(["-flto", "-fno-fat-lto-objects", "-c"])
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let functions = vec![
        (
            "main".into(),
            crate::compiler::backend::Object {
                code: vec![0xc3],
                calls: vec![],
                strings: vec![],
                exports: vec![],
                vtables: vec![],
                receipt: None,
            },
        ),
        (
            "side_effect".into(),
            crate::compiler::backend::Object {
                code: vec![0xc3],
                calls: vec![],
                strings: vec![],
                exports: vec![],
                vtables: vec![],
                receipt: None,
            },
        ),
    ];
    let options = LinkOptions {
        objects: vec![fs::read(object).unwrap()],
        toolchain: Some(ControlledToolchain::capture(&[]).unwrap()),
        ..LinkOptions::default()
    };
    let error = elf::link(&functions, &options, &PlanBudget::default()).unwrap_err();
    assert!(
        error.contains("plugin") || error.contains("native input must"),
        "{error}"
    );
}
