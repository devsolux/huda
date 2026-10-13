//! Windows profile and pipe security. No permission-repair or administrator fallback.
use crate::error::{require, Error, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{LocalFree, HANDLE};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    GetNamedSecurityInfoW, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    GetAce, GetTokenInformation, IsValidAcl, IsValidSid, TokenUser, ACCESS_ALLOWED_ACE, ACE_HEADER,
    ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{CreateDirectoryW, FILE_ATTRIBUTE_REPARSE_POINT};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

pub(crate) fn wide(value: &OsStr) -> Result<Vec<u16>> {
    let mut out: Vec<_> = value.encode_wide().collect();
    require(
        !out.contains(&0),
        "INVALID_INPUT",
        "Embedded NUL in an OS path",
    )?;
    out.push(0);
    Ok(out)
}

struct LocalAllocation(*mut std::ffi::c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: these pointers are allocated by Windows APIs documented to use LocalFree.
        unsafe {
            LocalFree(self.0);
        }
    }
}

/// Owns the allocation referenced by SECURITY_ATTRIBUTES for the entire native call.
pub(crate) struct Descriptor(LocalAllocation);
impl Descriptor {
    pub(crate) fn private(inherit_files: bool) -> Result<Self> {
        let sid = current_sid()?;
        let inherit = if inherit_files { "OICI" } else { "" };
        let sddl = wide(OsStr::new(&format!("O:{sid}D:P(A;{inherit};GA;;;{sid})")))?;
        let mut descriptor = null_mut();
        // SAFETY: terminated SDDL input and valid output pointer; revision 1 is SDDL_REVISION_1.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self(LocalAllocation(descriptor)))
    }
    pub(crate) fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0 .0,
            bInheritHandle: 0,
        }
    }
}

/// The SID remains valid for the duration of the API call; output is copied before freeing it.
unsafe fn sid_string(sid: *mut std::ffi::c_void) -> Result<String> {
    // SAFETY: callers provide a SID inside a live token/security-descriptor allocation.
    require(
        !sid.is_null() && unsafe { IsValidSid(sid) } != 0,
        "PERMISSION_DENIED",
        "Invalid Windows identity",
    )?;
    let mut text = null_mut();
    // SAFETY: valid SID and writable output pointer.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let _allocation = LocalAllocation(text.cast());
    let mut len = 0;
    // SAFETY: ConvertSidToStringSidW returns a terminated UTF-16 allocation.
    unsafe {
        while *text.add(len) != 0 {
            len += 1;
        }
    }
    // SAFETY: len was measured inside the live terminated allocation.
    String::from_utf16(unsafe { std::slice::from_raw_parts(text, len) })
        .map_err(|_| Error::new("PERMISSION_DENIED", "Invalid Windows SID encoding"))
}

fn process_sid(process: HANDLE) -> Result<String> {
    let mut token = null_mut();
    // SAFETY: process is a live/pseudo process handle; token is a writable output.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: successful OpenProcessToken transfers an owned handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut size = 0;
    // SAFETY: null/zero buffer is the documented size-query pattern.
    unsafe {
        GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut size);
    }
    require(
        size as usize >= std::mem::size_of::<TOKEN_USER>() && size <= 65536,
        "PERMISSION_DENIED",
        "Windows token size is invalid",
    )?;
    // usize storage supplies the alignment required by TOKEN_USER and its embedded pointers.
    let mut storage = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    // SAFETY: allocation is sufficiently large and aligned, and lives through SID copying.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            storage.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: the successful API call initialized TOKEN_USER and its SID in storage.
    unsafe { sid_string((*storage.as_ptr().cast::<TOKEN_USER>()).User.Sid) }
}

pub(crate) fn current_sid() -> Result<String> {
    // SAFETY: GetCurrentProcess has no preconditions; its pseudo handle must not be closed.
    process_sid(unsafe { GetCurrentProcess() })
}

pub(crate) fn verify_peer(pid: u32) -> Result<()> {
    // SAFETY: numeric PID came from the connected kernel pipe, not from request data.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    require(
        !handle.is_null(),
        "PERMISSION_DENIED",
        "IPC peer identity is unavailable",
    )?;
    // SAFETY: successful OpenProcess returns an owned handle.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    require(
        process_sid(handle.as_raw_handle())? == current_sid()?,
        "PERMISSION_DENIED",
        "IPC peer belongs to a different Windows user",
    )
}

pub(crate) fn create_private_directory(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let descriptor = Descriptor::private(true)?;
    let attributes = descriptor.attributes();
    let name = wide(path.as_os_str())?;
    // SAFETY: inputs remain alive and the new leaf gets its DACL atomically, before any data.
    if unsafe { CreateDirectoryW(name.as_ptr(), &attributes) } == 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(error.into());
        }
    }
    check_private_path(path)
}

pub(crate) fn check_private_path(path: &Path) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path)?;
    require(
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        "PERMISSION_DENIED",
        "Managed paths must not be Windows reparse points",
    )?;
    let name = wide(path.as_os_str())?;
    let mut owner = null_mut();
    let mut dacl: *mut ACL = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: valid terminated path and output pointers; allocation owned below on success.
    let status = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(std::io::Error::from_raw_os_error(status as i32).into());
    }
    let _allocation = LocalAllocation(descriptor);
    let current = current_sid()?;
    // SAFETY: owner and ACL point into the live security descriptor.
    require(
        unsafe { sid_string(owner)? } == current && !dacl.is_null(),
        "PERMISSION_DENIED",
        "Managed path must have a user-owned restrictive DACL",
    )?;
    // SAFETY: descriptor owns dacl; IsValidAcl checks its structure before enumeration.
    require(
        unsafe { IsValidAcl(dacl) } != 0,
        "PERMISSION_DENIED",
        "Invalid DACL",
    )?;
    // Fail closed on unfamiliar ACEs instead of guessing their effective rights.
    // 0/1 are the SDK ACCESS_ALLOWED_ACE_TYPE / ACCESS_DENIED_ACE_TYPE values.
    for i in 0..unsafe { (*dacl).AceCount } as u32 {
        let mut ace = null_mut();
        // SAFETY: index is bounded by a validated ACL; output points inside the allocation.
        require(
            unsafe { GetAce(dacl, i, &mut ace) } != 0,
            "PERMISSION_DENIED",
            "Cannot read DACL entry",
        )?;
        // SAFETY: GetAce returned the entry header.
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        if header.AceType == 1 {
            continue;
        } // Deny entries can only restrict access.
        require(
            header.AceType == 0
                && header.AceSize as usize >= std::mem::size_of::<ACCESS_ALLOWED_ACE>(),
            "PERMISSION_DENIED",
            "Unsupported DACL entry; no automatic permission repair",
        )?;
        // SAFETY: validated allowed-ACE layout; SID begins at SidStart and lives in the ACL.
        let sid = unsafe {
            sid_string(std::ptr::addr_of_mut!((*ace.cast::<ACCESS_ALLOWED_ACE>()).SidStart).cast())?
        };
        require(
            sid == current || sid == "S-1-5-18" || sid == "S-1-5-32-544",
            "PERMISSION_DENIED",
            "Managed path grants access beyond its owner, SYSTEM and Administrators",
        )?;
    }
    Ok(())
}
