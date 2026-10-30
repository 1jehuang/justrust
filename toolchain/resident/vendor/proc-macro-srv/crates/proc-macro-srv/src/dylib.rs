//! Handles dynamic library loading for proc macro

mod proc_macros;

use paths::{Utf8Path, Utf8PathBuf};
use rustc_proc_macro::bridge;
use std::{fs, io, path::Path, time::SystemTime};
use stdx::tempfile::NamedTempFile;

use crate::{
    PanicMessage, ProcMacroClientHandle, ProcMacroKind, ProcMacroSrvSpan, TrackedEnv,
    dylib::proc_macros::ProcMacros, token_stream::TokenStream,
};

pub(crate) struct Expander {
    inner: ProcMacroLibrary,
    modified_time: SystemTime,
    _file: NamedTempFile,
}

impl Expander {
    pub(crate) fn new(lib: &Utf8Path) -> io::Result<Expander> {
        // Some libraries for dynamic loading require canonicalized path even when it is
        // already absolute
        let lib = lib.canonicalize_utf8()?;
        let modified_time = fs::metadata(&lib).and_then(|it| it.modified())?;

        let file = ensure_file_with_lock_free_access(lib);
        let library = ProcMacroLibrary::open(file.path())?;

        Ok(Expander { inner: library, modified_time, _file: file })
    }

    pub(crate) fn expand<'a, S: ProcMacroSrvSpan + 'a>(
        &self,
        macro_name: &str,
        macro_body: TokenStream<S>,
        attribute: Option<TokenStream<S>>,
        def_site: S,
        call_site: S,
        mixed_site: S,
        tracked_env: &'a mut TrackedEnv,
        callback: Option<ProcMacroClientHandle<'a>>,
    ) -> Result<TokenStream<S>, PanicMessage>
    where
        <S::Server<'a> as bridge::server::Server>::TokenStream: Default,
    {
        self.inner.proc_macros.expand(
            macro_name,
            macro_body,
            attribute,
            def_site,
            call_site,
            mixed_site,
            tracked_env,
            callback,
        )
    }

    pub(crate) fn list_macros(&self) -> impl Iterator<Item = (&str, ProcMacroKind)> {
        self.inner.proc_macros.list_macros()
    }

    pub(crate) fn modified_time(&self) -> SystemTime {
        self.modified_time
    }
}

struct ProcMacroLibrary {
    proc_macros: ProcMacros,
}

/// One entry of the list ra-jit writes into a proc-macro crate it links as a shared library,
/// as `crates/ra-jit/src/macro_list.rs` declares it.
#[repr(C)]
struct Macro {
    kind: u32,
    name: &'static str,
    /// The protocol has no place for a derive's helper attributes: the client reads them
    /// from the source.
    #[allow(dead_code)]
    helpers: &'static [&'static str],
    client: bridge::client::Client,
}

/// The name of the list in the library's symbol table.
const SYMBOL: &std::ffi::CStr = c"RA_JIT_PROC_MACROS";

impl ProcMacroLibrary {
    /// Loads the library and reads ra-jit's list of its macros. The library is never
    /// unloaded, as rustc's loader leaks the libraries it loads: the clients and the names
    /// point into it.
    #[cfg(unix)]
    fn open(path: &Path) -> io::Result<Self> {
        use std::ffi::{CStr, CString};
        use std::os::unix::ffi::OsStrExt;

        let shown = path.display();
        let error = |what: &str| {
            // SAFETY: `dlerror` gives the last error of this thread, or null.
            let said = unsafe { libc::dlerror() };
            let said = match said.is_null() {
                true => "unknown error".into(),
                // SAFETY: a non-null result of `dlerror` is a C string.
                false => unsafe { CStr::from_ptr(said) }.to_string_lossy(),
            };
            io::Error::other(format!("{what} {shown}: {said}"))
        };
        let c_path = CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
        // SAFETY: loading a library runs its initializers, which a proc-macro library is
        // trusted to have, as rustc trusts it.
        let handle = unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            return Err(error("cannot load"));
        }
        // SAFETY: the handle is a loaded library's.
        let list = unsafe { libc::dlsym(handle, SYMBOL.as_ptr()) };
        if list.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{shown} has no list of proc macros: ra-jit did not link it"),
            ));
        }
        // SAFETY: `list` is what `dlsym` found.
        unsafe { Self::from_list(&shown.to_string(), list) }
    }

    /// The library behind `list`, the address of ra-jit's `static RA_JIT_PROC_MACROS: &[Macro]`.
    ///
    /// # Safety
    /// `list` is that static, in a library that stays loaded for as long as the process runs.
    unsafe fn from_list(shown: &str, list: *mut std::ffi::c_void) -> io::Result<Self> {
        // SAFETY: ra-jit exports the list as a `static RA_JIT_PROC_MACROS: &[Macro]`, and the
        // library stays loaded for as long as the process runs.
        let list: &'static [Macro] = unsafe { *list.cast::<&'static [Macro]>() };
        let mut macros = Vec::with_capacity(list.len());
        for mac in list {
            let kind = match mac.kind {
                0 => ProcMacroKind::CustomDerive,
                1 => ProcMacroKind::Attr,
                2 => ProcMacroKind::Bang,
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("{shown}: macro `{}` has unknown kind {other}", mac.name),
                    ));
                }
            };
            macros.push((mac.client, kind, mac.name));
        }

        Ok(ProcMacroLibrary { proc_macros: ProcMacros::new(macros) })
    }

    /// The same on Windows, with `LoadLibraryW` and `GetProcAddress`: the library is a DLL
    /// ra-jit linked, which exports the list by name.
    #[cfg(windows)]
    fn open(path: &Path) -> io::Result<Self> {
        use std::ffi::c_void;
        use std::os::windows::ffi::OsStrExt;

        unsafe extern "system" {
            fn LoadLibraryW(name: *const u16) -> *mut c_void;
            fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        }
        let shown = path.display();
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: loading a library runs its initializers, which a proc-macro library is
        // trusted to have, as rustc trusts it. The name ends with a zero.
        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::other(format!(
                "cannot load {shown}: {}",
                io::Error::last_os_error()
            )));
        }
        // SAFETY: the handle is a loaded library's; the name ends with a zero.
        let list = unsafe { GetProcAddress(handle, SYMBOL.as_ptr().cast()) };
        if list.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{shown} has no list of proc macros: ra-jit did not link it"),
            ));
        }
        // SAFETY: `list` is what `GetProcAddress` found.
        unsafe { Self::from_list(&shown.to_string(), list) }
    }

    #[cfg(not(any(unix, windows)))]
    fn open(path: &Path) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "cannot load {}: ra-jit's proc-macro libraries load on unix only",
                path.display()
            ),
        ))
    }
}

/// Copy the dylib to temp directory to prevent locking in Windows
#[cfg(windows)]
fn ensure_file_with_lock_free_access(path: Utf8PathBuf) -> NamedTempFile {
    if std::env::var("RA_DONT_COPY_PROC_MACRO_DLL").is_ok() {
        return NamedTempFile::from_path(path.into_std_path_buf());
    }

    (|| {
        let file_name = path.file_stem().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("File path is invalid: {path}"))
        })?;

        NamedTempFile::new_from_existing(
            &format!("proc-macro-srv-{file_name}.dll"),
            path.as_std_path(),
        )
    })()
    .unwrap_or_else(|_err| NamedTempFile::from_path(path.into_std_path_buf()))
}

#[cfg(unix)]
fn ensure_file_with_lock_free_access(path: Utf8PathBuf) -> NamedTempFile {
    NamedTempFile::from_path(path.into_std_path_buf())
}
