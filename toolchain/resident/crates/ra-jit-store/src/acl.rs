//! Whose a file or a directory of the store is on Windows, and who else can write to it.
//!
//! Windows has no modes: who may do what to a file is its access control list, which it
//! mostly inherits from the directory it was made in. A directory under the user's profile
//! lets in the user, the system and the administrators; one made at the top of a drive
//! lets every user of the machine add files. So the owner and each entry of the list are
//! read here, and anything that lets someone else write is a reason to refuse the store.
//!
//! The system and the administrators are taken as the user is: they can read and write
//! the user's files whatever the list says, and a file made by an elevated process belongs
//! to the administrators and not to the user who ran it.

use std::{ffi::c_void, fs::File, io, os::windows::io::AsRawHandle as _, ptr};

use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, LocalFree},
    Security::{
        ACL,
        Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetTokenInformation, IsWellKnownSid,
        OWNER_SECURITY_INFORMATION, PSID, TOKEN_QUERY, TOKEN_USER, TokenUser,
        WinBuiltinAdministratorsSid, WinCreatorOwnerRightsSid, WinLocalSystemSid,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

/// The kinds of entry that let someone in and that keep someone out. Any other kind lets
/// in on a condition this does not evaluate, and is taken to let in.
const ALLOWED: u8 = 0;
const DENIED: u8 = 1;
/// An entry with this is for what is made in a directory and not for the directory.
const INHERIT_ONLY: u8 = 0x8;

/// What changes a file or what is in a directory, or who may: writing and appending (which
/// in a directory are adding a file and adding a directory), deleting it or what is in it,
/// changing its list or its owner, and the rights that stand for all of those.
const WRITES: u32 =
    0x2 | 0x4 | 0x40 | 0x0001_0000 | 0x0004_0000 | 0x0008_0000 | 0x4000_0000 | 0x1000_0000;

/// An entry that lets in or keeps out, as it is laid out: the identifier starts where
/// `sid_start` is and is as long as it says itself.
#[repr(C)]
struct Entry {
    kind: u8,
    flags: u8,
    size: u16,
    mask: u32,
    sid_start: u32,
}

/// A handle that is closed when this is dropped.
struct Token(HANDLE);

impl Drop for Token {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `OpenProcessToken` and is closed once.
        unsafe { CloseHandle(self.0) };
    }
}

/// What `GetSecurityInfo` allocated, freed when this is dropped.
struct Descriptor(*mut c_void);

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `GetSecurityInfo`, which says to free it so.
        unsafe { LocalFree(self.0) };
    }
}

/// The identifier of the user this process runs as, in the buffer that holds it.
fn user() -> io::Result<Vec<u64>> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: the process handle is this process's, and `token` is where a handle goes.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Token(token);
    let mut len = 0;
    // SAFETY: with no buffer the call only says how long one has to be.
    unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut len) };
    // Words and not bytes, for the structure at the start to be aligned.
    let mut buffer = vec![0u64; (len as usize).div_ceil(8).max(1)];
    let room = (buffer.len() * 8) as u32;
    // SAFETY: `buffer` is `room` bytes that are this function's to write.
    let got = unsafe {
        GetTokenInformation(token.0, TokenUser, buffer.as_mut_ptr().cast(), room, &mut len)
    };
    if got == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(buffer)
}

/// Whether `sid` is the user, the system or the administrators.
///
/// # Safety
///
/// `sid` and `user` have to point at valid identifiers.
unsafe fn trusted(sid: PSID, user: PSID) -> bool {
    // SAFETY: both are valid identifiers, by what the caller says.
    unsafe {
        EqualSid(sid, user) != 0
            || IsWellKnownSid(sid, WinLocalSystemSid) != 0
            || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
    }
}

/// Why what `file` is could have been written by someone other than the user this process
/// runs as, the system or an administrator, if it could. What cannot be found out is a
/// reason too: a store is used only when it is known to be the user's.
pub(super) fn others_write(file: &File) -> Option<&'static str> {
    let Ok(user) = user() else {
        return Some("has an owner that could not be compared with the user");
    };
    // SAFETY: `user` holds what `GetTokenInformation` wrote, which starts with a `TOKEN_USER`.
    let user = unsafe { (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let (mut owner, mut list): (PSID, *mut ACL) = (ptr::null_mut(), ptr::null_mut());
    let mut descriptor = ptr::null_mut();
    let asked = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    // SAFETY: the handle is of an open file, and the three pointers are where the owner,
    // the list and the descriptor that holds both are put.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            asked,
            &mut owner,
            ptr::null_mut(),
            &mut list,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Some("has an access control list that could not be read");
    }
    let _descriptor = Descriptor(descriptor);
    // SAFETY: `owner` is in the descriptor, which is alive, and `user` in its buffer.
    if owner.is_null() || !unsafe { trusted(owner, user) } {
        return Some("belongs to another user");
    }
    // No list at all lets everyone do everything.
    if list.is_null() {
        return Some("can be written to by other users");
    }
    // SAFETY: `list` is in the descriptor, which is alive.
    let entries = unsafe { (*list).AceCount };
    for at in 0..u32::from(entries) {
        let mut entry: *mut c_void = ptr::null_mut();
        // SAFETY: `list` is a valid list and `at` is below how many entries it has.
        if unsafe { GetAce(list, at, &mut entry) } == 0 {
            return Some("has an access control list that could not be read");
        }
        // SAFETY: every entry starts with its kind, flags and size, then a mask and an
        // identifier for the kinds read here; `read_unaligned` asks for no alignment.
        let Entry { kind, flags, size, mask, .. } =
            unsafe { entry.cast::<Entry>().read_unaligned() };
        if kind == DENIED || flags & INHERIT_ONLY != 0 || mask & WRITES == 0 {
            continue;
        }
        if kind != ALLOWED || usize::from(size) < size_of::<Entry>() {
            return Some("has an access control list with an entry of a kind that is not read");
        }
        // SAFETY: the identifier of an entry that lets in starts after the mask.
        let sid: PSID = unsafe { entry.byte_add(std::mem::offset_of!(Entry, sid_start)) };
        // SAFETY: `sid` is in the list, `user` in its buffer; the owner's own rights are
        // those of the owner, who was found to be trusted.
        let known =
            unsafe { trusted(sid, user) || IsWellKnownSid(sid, WinCreatorOwnerRightsSid) != 0 };
        if !known {
            return Some("can be written to by other users");
        }
    }
    None
}
