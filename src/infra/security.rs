//! Windows handles and current-user-only object security.
#[cfg(test)]
#[path = "security_tests.rs"]
mod tests;
use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        ACCESS_ALLOWED_ACE, ACL,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SE_OBJECT_TYPE,
        },
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetTokenInformation,
        OWNER_SECURITY_INFORMATION, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

pub(super) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub(super) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(super) struct Handle(pub HANDLE);
// Owned kernel handles can move between workers. Mutex ownership is separately
// confined to its acquiring thread by InstanceGuard.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Handle {
    /// Transfer sole ownership to an I/O driver (which also closes on failure).
    pub fn into_raw(self) -> HANDLE {
        let raw = self.0;
        std::mem::forget(self);
        raw
    }

    pub fn new(raw: HANDLE) -> io::Result<Self> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(raw))
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct LocalAllocation(*mut core::ffi::c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

pub(super) struct UserSecurity {
    token_user: Vec<usize>,
    descriptor: LocalAllocation,
    pub sid_text: String,
}
impl UserSecurity {
    pub fn current() -> io::Result<Self> {
        unsafe {
            let mut raw = ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) == 0 {
                return Err(io::Error::last_os_error());
            }
            let token = Handle::new(raw)?;
            let mut size = 0;
            GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut size);
            let mut token_user = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
            if GetTokenInformation(
                token.0,
                TokenUser,
                token_user.as_mut_ptr().cast(),
                size,
                &mut size,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let sid = (*(token_user.as_ptr().cast::<TOKEN_USER>())).User.Sid;
            let mut text = ptr::null_mut();
            if ConvertSidToStringSidW(sid, &mut text) == 0 {
                return Err(io::Error::last_os_error());
            }
            let allocation = LocalAllocation(text.cast());
            let mut len = 0;
            while *text.add(len) != 0 {
                len += 1;
            }
            let sid_text = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
            drop(allocation);
            // Explicit owner and protected DACL: neither Everyone nor inherited ACEs.
            let sddl = wide(&format!("O:{sid_text}D:P(A;;GA;;;{sid_text})"));
            let mut descriptor = ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                token_user,
                descriptor: LocalAllocation(descriptor),
                sid_text,
            })
        }
    }
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor.0,
            bInheritHandle: 0,
        }
    }
    /// Reject pre-created objects/servers with a different owner or permissive ACL.
    /// Same-user processes remain inside this trust boundary.
    pub fn verify(&self, handle: HANDLE, kind: SE_OBJECT_TYPE) -> io::Result<()> {
        unsafe {
            let mut owner = ptr::null_mut();
            let mut acl: *mut ACL = ptr::null_mut();
            let mut sd = ptr::null_mut();
            let code = GetSecurityInfo(
                handle,
                kind,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                ptr::null_mut(),
                &mut acl,
                ptr::null_mut(),
                &mut sd,
            );
            if code != 0 {
                return Err(io::Error::from_raw_os_error(code as i32));
            }
            let _sd = LocalAllocation(sd);
            let sid = (*(self.token_user.as_ptr().cast::<TOKEN_USER>())).User.Sid;
            if owner.is_null() || EqualSid(owner, sid) == 0 || acl.is_null() || (*acl).AceCount != 1
            {
                return Err(invalid("untrusted object owner or DACL"));
            }
            let mut ace = ptr::null_mut();
            if GetAce(acl, 0, &mut ace) == 0 {
                return Err(io::Error::last_os_error());
            }
            let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
            // Generic-all can be mapped by the object manager to object-specific all.
            let all = match kind {
                windows_sys::Win32::Security::Authorization::SE_FILE_OBJECT => 0x001f01ff,
                _ => 0x001f0001, // MUTEX_ALL_ACCESS
            };
            if allowed.Header.AceType != 0
                || allowed.Header.AceFlags != 0
                || (allowed.Mask != 0x10000000 && allowed.Mask != all)
                || EqualSid((&allowed.SidStart as *const u32).cast_mut().cast(), sid) == 0
            {
                return Err(invalid("untrusted object access list"));
            }
            Ok(())
        }
    }
}

/// Stable across processes and sessions for this Windows user. Test suffixes must
/// be unique and never use the production object names.
#[derive(Clone, Debug)]
pub struct Namespace {
    pub(super) stem: String,
}
impl Namespace {
    pub fn current_user() -> io::Result<Self> {
        Ok(Self {
            stem: format!("mmv-v1-{}", UserSecurity::current()?.sid_text),
        })
    }
    #[cfg(test)]
    pub(super) fn isolated() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self {
            stem: format!(
                "mmv-test-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ),
        }
    }
    pub(super) fn pipe(&self) -> Vec<u16> {
        wide(&format!(r"\\.\pipe\{}", self.stem))
    }
    pub(super) fn mutex(&self, role: &str) -> Vec<u16> {
        wide(&format!(r"Global\{}-{role}", self.stem))
    }
}
