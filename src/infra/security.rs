//! Windows handles and current-user-only object security.
#[cfg(test)]
#[path = "security_tests.rs"]
mod tests;
use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SE_OBJECT_TYPE,
        },
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetSecurityDescriptorControl,
        GetTokenInformation, OWNER_SECURITY_INFORMATION, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES,
        TOKEN_QUERY, TOKEN_USER, TokenUser,
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
#[allow(
    unsafe_code,
    reason = "an owned Windows kernel handle can move across threads"
)]
// SAFETY: These owned kernel handles are not thread-affine. Moving the wrapper
// transfers sole ownership; CloseHandle may run on the receiving thread.
unsafe impl Send for Handle {}
#[allow(
    unsafe_code,
    reason = "sharing the wrapper only permits borrowing its handle"
)]
// SAFETY: Sharing the wrapper does not modify or close the handle. Drop and
// into_raw require ownership, so a live shared borrow keeps the handle open.
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
        #[allow(unsafe_code, reason = "release the owned Windows kernel handle")]
        // SAFETY: The wrapper owns this CloseHandle-compatible handle. into_raw
        // suppresses Drop when transferring ownership.
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct LocalAllocation(*mut core::ffi::c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        #[allow(unsafe_code, reason = "release the Windows API allocation")]
        // SAFETY: This pointer comes from a successful Win32 call that transfers
        // a LocalFree allocation to the caller. This wrapper frees it once.
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
        let mut raw = ptr::null_mut();
        #[allow(unsafe_code, reason = "open the current process token through Win32")]
        // SAFETY: GetCurrentProcess supplies a valid pseudo-handle for this call,
        // and raw is a writable HANDLE out parameter.
        let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) };
        if opened == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Handle::new(raw)?;
        let mut size = 0;
        #[allow(unsafe_code, reason = "query the required Win32 token buffer size")]
        // SAFETY: The live token has TOKEN_QUERY access. A null buffer of length
        // zero requests the required size through the writable out parameter.
        unsafe {
            GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut size);
        }
        let mut token_user = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        #[allow(unsafe_code, reason = "fill the aligned token buffer through Win32")]
        // SAFETY: The live token has TOKEN_QUERY access. The buffer is writable
        // for at least size bytes, aligned for TOKEN_USER, and separate from the size output.
        let queried = unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                token_user.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        };
        if queried == 0 {
            return Err(io::Error::last_os_error());
        }
        #[allow(unsafe_code, reason = "read the TOKEN_USER initialized by Win32")]
        // SAFETY: Successful GetTokenInformation initialized the aligned
        // TOKEN_USER and its SID in this buffer, which remains live.
        let sid = unsafe { (*token_user.as_ptr().cast::<TOKEN_USER>()).User.Sid };
        let mut text = ptr::null_mut();
        #[allow(unsafe_code, reason = "convert the token SID to a Win32 string")]
        // SAFETY: sid points into the live token_user buffer and text is a
        // writable out parameter for the allocated string.
        let converted = unsafe { ConvertSidToStringSidW(sid, &mut text) };
        if converted == 0 {
            return Err(io::Error::last_os_error());
        }
        let allocation = LocalAllocation(text.cast());
        #[allow(unsafe_code, reason = "borrow the UTF-16 string returned by Win32")]
        // SAFETY: The successful conversion supplies a NUL-terminated UTF-16
        // string. Its allocation stays live through the scan and string copy.
        let units = unsafe {
            let mut len = 0;
            while *text.add(len) != 0 {
                len += 1;
            }
            std::slice::from_raw_parts(text, len)
        };
        let sid_text = String::from_utf16_lossy(units);
        drop(allocation);
        // Explicit owner and protected DACL: neither Everyone nor inherited ACEs.
        let sddl = wide(&format!("O:{sid_text}D:P(A;;GA;;;{sid_text})"));
        let mut descriptor = ptr::null_mut();
        #[allow(
            unsafe_code,
            reason = "construct a Win32 security descriptor from SDDL"
        )]
        // SAFETY: The SDDL buffer is live and NUL-terminated, descriptor is
        // writable, and the optional size output may be null.
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        if converted == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            token_user,
            descriptor: LocalAllocation(descriptor),
            sid_text,
        })
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
    /// Callers keep the borrowed handle live for the duration of the check.
    pub fn verify(&self, handle: HANDLE, kind: SE_OBJECT_TYPE) -> io::Result<()> {
        let mut owner = ptr::null_mut();
        let mut acl: *mut ACL = ptr::null_mut();
        let mut sd = ptr::null_mut();
        #[allow(
            unsafe_code,
            reason = "query Win32 security information for the borrowed handle"
        )]
        // SAFETY: Callers keep the handle live. The requested outputs are writable,
        // and the unrequested group and SACL outputs may be null.
        let code = unsafe {
            GetSecurityInfo(
                handle,
                kind,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                ptr::null_mut(),
                &mut acl,
                ptr::null_mut(),
                &mut sd,
            )
        };
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
        let _sd = LocalAllocation(sd);
        let mut control = 0;
        let mut revision = 0;
        #[allow(unsafe_code, reason = "read control bits from the Win32 descriptor")]
        // SAFETY: sd points to the live descriptor returned by GetSecurityInfo,
        // and both out parameters are writable.
        let queried = unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) };
        if queried == 0 {
            return Err(io::Error::last_os_error());
        }
        if control & SE_DACL_PROTECTED == 0 {
            return Err(invalid("unprotected instance access list"));
        }
        #[allow(
            unsafe_code,
            reason = "read the stored token SID for the owner and ACE checks"
        )]
        // SAFETY: The unchanged, aligned token buffer still contains the
        // TOKEN_USER and SID initialized by GetTokenInformation.
        let sid = unsafe { (*self.token_user.as_ptr().cast::<TOKEN_USER>()).User.Sid };
        #[allow(
            unsafe_code,
            reason = "inspect the descriptor owner and ACL returned by Win32"
        )]
        // SAFETY: sid is live in the token buffer. Non-null owner and ACL
        // pointers refer to initialized data in the live descriptor.
        let trusted_owner_and_acl = unsafe {
            !owner.is_null() && EqualSid(owner, sid) != 0 && !acl.is_null() && (*acl).AceCount == 1
        };
        if !trusted_owner_and_acl {
            return Err(invalid("untrusted object owner or DACL"));
        }
        let mut ace = ptr::null_mut();
        #[allow(unsafe_code, reason = "locate the sole ACE through Win32")]
        // SAFETY: acl is non-null, has one entry, and is kept live by _sd;
        // ace is a writable out parameter.
        let got_ace = unsafe { GetAce(acl, 0, &mut ace) };
        if got_ace == 0 {
            return Err(io::Error::last_os_error());
        }
        // Generic-all can be mapped by the object manager to object-specific all.
        let all = match kind {
            windows_sys::Win32::Security::Authorization::SE_FILE_OBJECT => 0x001f01ff,
            _ => 0x001f0001, // MUTEX_ALL_ACCESS
        };
        #[allow(
            unsafe_code,
            reason = "inspect the ACE and compare its SID through Win32"
        )]
        // SAFETY: GetAce supplies an aligned ACE in the live descriptor. Read
        // its common header first; the access-allowed type has a mask and an
        // inline SID beginning at SidStart. Both SIDs remain live for EqualSid.
        let trusted_ace = unsafe {
            let header = &*ace.cast::<ACE_HEADER>();
            header.AceType == 0 && header.AceFlags == 0 && {
                let allowed = ace.cast::<ACCESS_ALLOWED_ACE>();
                let mask = (*allowed).Mask;
                (mask == 0x10000000 || mask == all)
                    && EqualSid((&raw mut (*allowed).SidStart).cast(), sid) != 0
            }
        };
        if !trusted_ace {
            return Err(invalid("untrusted object access list"));
        }
        Ok(())
    }
}
