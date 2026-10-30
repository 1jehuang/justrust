//! Closed GNU/glibc linker input capsule. Discovery never executes a tool.
//! Only bytes retained in this value are materialized in the linker's root.
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    process::Command,
};

// The complete capsule's option-identity framing fits beneath this bound,
// leaving 4 MiB for other fields in the existing 16 MiB source option identity.
// Acquisition retains the bytes actually read, not an atomic host snapshot.
const MAX_CAPSULE_IDENTITY_BYTES: usize = 12 * 1024 * 1024;
const MAX_EXECUTABLE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ControlledToolchain {
    files: BTreeMap<PathBuf, Vec<u8>>,
    library_dir: PathBuf,
    libraries: Vec<String>,
}
impl ControlledToolchain {
    /// Change this framing whenever the execution boundary or fixed argv changes.
    pub(crate) fn policy(&self) -> &'static [u8] {
        b"justrust-controlled-gnu-ld-glibc-x86-64-relative-argv-v8"
    }
    pub(crate) fn library_directory(&self) -> &Path {
        &self.library_dir
    }
    pub(crate) fn libraries(&self) -> &[String] {
        &self.libraries
    }
    /// Capture the deliberately narrow Linux x86-64 GNU/glibc toolchain.
    /// No PATH, driver specs, plugins, linker scripts or environment discovery.
    pub fn capture(libraries: &[String]) -> Result<Self, String> {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return Err("unsupported: controlled native linking requires Linux x86-64".into());
        }
        if libraries.len() > 4096 {
            return Err("resource limit: native library selection exceeds 4096 entries".into());
        }
        for name in libraries {
            if !matches!(name.as_str(), "c" | "m") {
                return Err(format!(
                    "unsupported: controlled native library `{name}` (only c and m are captured)"
                ));
            }
        }
        let library_dir = if Path::new("/usr/lib/x86_64-linux-gnu/crt1.o").is_file() {
            PathBuf::from("/usr/lib/x86_64-linux-gnu")
        } else {
            PathBuf::from("/usr/lib")
        };
        let mut this = Self {
            files: BTreeMap::new(),
            library_dir,
            libraries: libraries.to_vec(),
        };
        this.capture_file(Path::new("/usr/bin/ld"))?;
        for name in [
            "crt1.o",
            "crti.o",
            "crtn.o",
            "libc.so.6",
            "libc_nonshared.a",
        ] {
            this.capture_file(&this.library_dir.join(name))?;
        }
        if libraries.iter().any(|name| name == "m") {
            this.capture_file(&this.library_dir.join("libm.so.6"))?;
        }
        // crt1 and libc name the system ELF interpreter. Keep its conventional
        // executable path in the capsule even on distributions using symlinks.
        this.capture_file(Path::new("/lib64/ld-linux-x86-64.so.2"))?;
        Ok(this)
    }
    pub fn inputs(&self) -> impl Iterator<Item = (&Path, &[u8])> {
        self.files
            .iter()
            .map(|(path, bytes)| (path.as_path(), bytes.as_slice()))
    }
    fn identity_bytes(&self) -> Option<usize> {
        // Matches SourceInputs' capsule field framing: presence flag, policy,
        // selected directory, library count/names, file count and path/bytes.
        let mut size = 57usize
            .checked_add(self.policy().len())?
            .checked_add(self.library_dir.as_os_str().as_encoded_bytes().len())?;
        for name in &self.libraries {
            size = size.checked_add(8)?.checked_add(name.len())?;
        }
        for (path, bytes) in &self.files {
            size = size
                .checked_add(16)?
                .checked_add(path.as_os_str().as_encoded_bytes().len())?
                .checked_add(bytes.len())?;
        }
        Some(size)
    }
    fn capture_file(&mut self, path: &Path) -> Result<(), String> {
        if !path.is_absolute()
            || path.components().any(|part| {
                !matches!(
                    part,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )
            })
        {
            return Err("unsupported: noncanonical native toolchain path".into());
        }
        if self.files.contains_key(path) {
            return Ok(());
        }
        if self.files.len() >= 128 {
            return Err("resource limit: native toolchain closure exceeds 128 files".into());
        }
        let remaining = self
            .identity_bytes()
            .and_then(|size| size.checked_add(16))
            .and_then(|size| size.checked_add(path.as_os_str().as_encoded_bytes().len()))
            .and_then(|size| MAX_CAPSULE_IDENTITY_BYTES.checked_sub(size))
            .ok_or("resource limit: native toolchain capsule identity exceeds 12 MiB")?;
        let bytes = acquire_regular(path, remaining, "native toolchain capsule input")?;
        let dependencies = elf_dependencies(&bytes)?;
        self.files.insert(path.to_owned(), bytes);
        for dependency in dependencies {
            let dep = if dependency.starts_with('/') {
                PathBuf::from(dependency)
            } else {
                if dependency.contains('/') || dependency == "." || dependency == ".." {
                    return Err("unsupported: noncanonical native dependency".into());
                }
                let candidates = [
                    self.library_dir.join(&dependency),
                    Path::new("/lib/x86_64-linux-gnu").join(&dependency),
                    Path::new("/usr/lib64").join(&dependency),
                    Path::new("/lib64").join(&dependency),
                ];
                candidates
                    .into_iter()
                    .find(|path| path.is_file())
                    .ok_or_else(|| {
                        format!("unsupported: missing native tool dependency `{dependency}`")
                    })?
            };
            self.capture_file(&dep)?;
        }
        Ok(())
    }
    pub(super) fn execute(
        &self,
        root: &Path,
        objects: &[Vec<u8>],
        own: &[u8],
        libraries: &[String],
    ) -> Result<Vec<u8>, String> {
        if self.libraries != libraries {
            return Err("unsupported: native libraries differ from captured toolchain".into());
        }
        for (path, bytes) in &self.files {
            let path = root.join(
                path.strip_prefix("/")
                    .map_err(|_| "invalid toolchain path")?,
            );
            std::fs::create_dir_all(path.parent().ok_or("invalid toolchain path")?)
                .map_err(infrastructure)?;
            std::fs::write(&path, bytes).map_err(infrastructure)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o555))
                    .map_err(infrastructure)?;
            }
        }
        std::fs::create_dir(root.join("inputs")).map_err(infrastructure)?;
        std::fs::create_dir(root.join("out")).map_err(infrastructure)?;
        std::fs::write(root.join("inputs/native.o"), own).map_err(infrastructure)?;
        for (index, bytes) in objects.iter().enumerate() {
            // Thin archives contain filesystem paths, not captured member bytes.
            // Linker scripts and LTO/plugin driver inputs are not admitted.
            if !bytes.starts_with(b"\x7fELF") && !bytes.starts_with(b"!<arch>\n") {
                return Err("unsupported: native input must be an ELF object or regular ar archive snapshot".into());
            }
            std::fs::write(root.join(format!("inputs/{index}.o")), bytes)
                .map_err(infrastructure)?;
        }
        let mut command = self.command(root, objects.len(), libraries)?;
        let output = super::link_sandbox_run::output(&mut command, root).map_err(|e| {
            format!(
                "infrastructure: controlled native linker boundary: {}",
                super::link_sandbox::setup_error(&e)
            )
        })?;
        if !output.status.success() {
            return Err(format!(
                "native link failed ({}):\n{}{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        acquire_output(
            &root.join("out/program"),
            MAX_EXECUTABLE_BYTES,
            "native executable",
        )
    }

    pub(super) fn command(
        &self,
        root: &Path,
        objects: usize,
        libraries: &[String],
    ) -> Result<Command, String> {
        use std::os::unix::process::CommandExt;
        // Only the kernel's executable lookup uses the private absolute root.
        // Tool-visible argument strings use the captured namespace instead.
        let relative = |path: &Path| path.strip_prefix("/").unwrap().to_path_buf();
        let loader = root.join("lib64/ld-linux-x86-64.so.2");
        let library_paths = self
            .files
            .keys()
            .filter_map(|p| p.parent())
            .map(relative)
            .collect::<std::collections::BTreeSet<_>>();
        let library_paths = std::env::join_paths(library_paths)
            .map_err(|e| format!("infrastructure: native library paths: {e}"))?;
        let mut command = Command::new(&loader);
        command
            .arg0("lib64/ld-linux-x86-64.so.2")
            .env_clear()
            .current_dir(root)
            .args(["--inhibit-cache", "--library-path"])
            .arg(library_paths)
            .arg(relative(Path::new("/usr/bin/ld")))
            .args([
                "-m",
                "elf_x86_64",
                "--build-id=none",
                "--fatal-warnings",
                "--eh-frame-hdr",
                "-z",
                "relro",
                "-z",
                "noexecstack",
                "-o",
            ])
            .arg("out/program")
            .args(["--dynamic-linker", "/lib64/ld-linux-x86-64.so.2"])
            .arg(relative(&self.library_dir.join("crt1.o")))
            .arg(relative(&self.library_dir.join("crti.o")))
            .arg("inputs/native.o");
        for index in 0..objects {
            command.arg(format!("inputs/{index}.o"));
        }
        if libraries.iter().any(|name| name == "m") {
            command.arg(relative(&self.library_dir.join("libm.so.6")));
        }
        command
            .arg(relative(&self.library_dir.join("libc.so.6")))
            .arg(relative(&self.library_dir.join("libc_nonshared.a")))
            .arg("lib64/ld-linux-x86-64.so.2")
            .arg(relative(&self.library_dir.join("crtn.o")));
        Ok(command)
    }
}

#[derive(Debug)]
enum AcquisitionError {
    Io(io::Error),
    NotRegular,
    Limit,
}
fn open_regular(path: &Path, limit: usize) -> Result<File, AcquisitionError> {
    open_regular_mode(path, limit, false)
}
fn open_regular_mode(path: &Path, limit: usize, nofollow: bool) -> Result<File, AcquisitionError> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A FIFO substituted before open must reject, not block waiting for a
        // writer. Regular files ignore O_NONBLOCK. Validate the opened handle,
        // never metadata from a separate path lookup.
        options.custom_flags(0x800 | if nofollow { 0x20000 } else { 0 });
    }
    let file = options.open(path).map_err(AcquisitionError::Io)?;
    let metadata = file.metadata().map_err(AcquisitionError::Io)?;
    if !metadata.is_file() {
        return Err(AcquisitionError::NotRegular);
    }
    if metadata.len() > limit as u64 {
        return Err(AcquisitionError::Limit);
    }
    Ok(file)
}
fn bounded_read(reader: impl Read, limit: usize) -> Result<Vec<u8>, AcquisitionError> {
    let probe = limit.checked_add(1).ok_or(AcquisitionError::Limit)?;
    let mut bytes = Vec::new();
    reader
        .take(probe as u64)
        .read_to_end(&mut bytes)
        .map_err(AcquisitionError::Io)?;
    if bytes.len() > limit {
        return Err(AcquisitionError::Limit);
    }
    Ok(bytes)
}
fn acquire_regular(path: &Path, limit: usize, context: &str) -> Result<Vec<u8>, String> {
    open_regular(path, limit)
        .and_then(|file| bounded_read(file, limit))
        .map_err(|error| acquisition_error(error, path, limit, context))
}
// The linker is confined, but the parent reading its result is not. Never
// follow a tool-created output symlink into an uncaptured host file.
fn acquire_output(path: &Path, limit: usize, context: &str) -> Result<Vec<u8>, String> {
    open_regular_mode(path, limit, true)
        .and_then(|file| bounded_read(file, limit))
        .map_err(|error| acquisition_error(error, path, limit, context))
}
fn acquisition_error(error: AcquisitionError, path: &Path, limit: usize, context: &str) -> String {
    match error {
        AcquisitionError::Io(error) => {
            format!("infrastructure: read {context} {}: {error}", path.display())
        }
        AcquisitionError::NotRegular => format!(
            "unsupported: {context} must be a regular file: {}",
            path.display()
        ),
        AcquisitionError::Limit => format!(
            "resource limit: {context} exceeds {limit} byte acquisition limit: {}",
            path.display()
        ),
    }
}
fn infrastructure(error: std::io::Error) -> String {
    format!("infrastructure: controlled native link: {error}")
}
fn read<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], String> {
    bytes
        .get(offset..offset.checked_add(N).ok_or("ELF offset overflow")?)
        .ok_or("unsupported: truncated native tool ELF")?
        .try_into()
        .map_err(|_| "unsupported: malformed native tool ELF".into())
}
fn usize64(bytes: &[u8], offset: usize) -> Result<usize, String> {
    usize::try_from(u64::from_le_bytes(read(bytes, offset)?))
        .map_err(|_| "unsupported: ELF offset too large".into())
}
fn string(bytes: &[u8], offset: usize) -> Result<String, String> {
    let tail = bytes
        .get(offset..)
        .ok_or("unsupported: invalid ELF string offset")?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or("unsupported: unterminated ELF dependency")?;
    std::str::from_utf8(&tail[..length])
        .map(str::to_owned)
        .map_err(|_| "unsupported: non-UTF8 ELF dependency".into())
}
fn elf_dependencies(bytes: &[u8]) -> Result<Vec<String>, String> {
    if bytes.starts_with(b"!<arch>\n") {
        return Ok(Vec::new());
    }
    if bytes.get(..7) != Some(b"\x7fELF\x02\x01\x01") || u16::from_le_bytes(read(bytes, 18)?) != 62
    {
        return Err(
            "unsupported: native toolchain requires ELF64 little-endian x86-64 or regular ar"
                .into(),
        );
    }
    let mut dependencies = Vec::new();
    let ph = usize64(bytes, 32)?;
    let ph_size = usize::from(u16::from_le_bytes(read(bytes, 54)?));
    let ph_count = usize::from(u16::from_le_bytes(read(bytes, 56)?));
    if ph_count > 1024 || ph_size < 56 && ph_count != 0 {
        return Err("unsupported: native ELF program headers".into());
    }
    for index in 0..ph_count {
        let header = ph
            .checked_add(index.checked_mul(ph_size).ok_or("ELF header overflow")?)
            .ok_or("ELF header overflow")?;
        if u32::from_le_bytes(read(bytes, header)?) == 3 {
            dependencies.push(string(bytes, usize64(bytes, header + 8)?)?);
        }
    }
    let sh = usize64(bytes, 40)?;
    let sh_size = usize::from(u16::from_le_bytes(read(bytes, 58)?));
    let sh_count = usize::from(u16::from_le_bytes(read(bytes, 60)?));
    if sh_count == 0 || sh_size < 64 {
        return Err("unsupported: sectionless native tool ELF".into());
    }
    for index in 0..sh_count {
        let header = sh
            .checked_add(index.checked_mul(sh_size).ok_or("ELF header overflow")?)
            .ok_or("ELF header overflow")?;
        if u32::from_le_bytes(read(bytes, header + 4)?) != 6 {
            continue;
        }
        let offset = usize64(bytes, header + 24)?;
        let size = usize64(bytes, header + 32)?;
        let linked = u32::from_le_bytes(read(bytes, header + 40)?) as usize;
        if linked >= sh_count || size % 16 != 0 {
            return Err("unsupported: malformed ELF dynamic table".into());
        }
        let strings_header = sh
            .checked_add(linked.checked_mul(sh_size).ok_or("ELF header overflow")?)
            .ok_or("ELF header overflow")?;
        let start = usize64(bytes, strings_header + 24)?;
        let length = usize64(bytes, strings_header + 32)?;
        let strings = bytes
            .get(start..start.checked_add(length).ok_or("ELF string overflow")?)
            .ok_or("unsupported: truncated ELF strings")?;
        let dynamic = bytes
            .get(offset..offset.checked_add(size).ok_or("ELF dynamic overflow")?)
            .ok_or("unsupported: truncated ELF dynamic table")?;
        for entry in dynamic.as_chunks::<16>().0 {
            let tag = i64::from_le_bytes(read(entry, 0)?);
            match tag {
                0 => break,
                1 => dependencies.push(string(strings, usize64(entry, 8)?)?),
                15 | 29 => {
                    return Err(
                        "unsupported: native toolchain RPATH/RUNPATH is not captured".into(),
                    );
                }
                _ => {}
            }
        }
    }
    Ok(dependencies)
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
#[path = "controlled_link_tests.rs"]
mod tests;
