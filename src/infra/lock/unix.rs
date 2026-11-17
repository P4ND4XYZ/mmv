//! Linux descriptor-relative operations using the target ABI's libc definitions.
use super::SecureFile;
use std::{
    ffi::CString,
    fs::File,
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path},
};

use libc::{O_CLOEXEC, O_CREAT, O_DIRECTORY, O_EXCL, O_NOFOLLOW, O_RDONLY, O_RDWR};
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "untrusted instance owner, permissions or link",
    )
}
fn name(value: &std::ffi::OsStr) -> io::Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| invalid())
}
fn open_at(
    parent: &File,
    name: &CString,
    flags: libc::c_int,
    mode: libc::mode_t,
) -> io::Result<File> {
    #[allow(
        unsafe_code,
        reason = "open relative to a live directory descriptor through libc"
    )]
    // SAFETY: parent keeps the directory FD live, name is NUL-terminated, and
    // mode has the target ABI type required when the flags include O_CREAT.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | O_NOFOLLOW | O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        #[allow(unsafe_code, reason = "transfer the newly opened descriptor to File")]
        // SAFETY: A nonnegative result from openat is a new owned descriptor;
        // File takes sole responsibility for closing it.
        let file = unsafe { File::from_raw_fd(fd) };
        Ok(file)
    }
}
pub(super) fn open(directory: &Path, role: &str) -> io::Result<SecureFile> {
    if !directory.is_absolute() {
        return Err(invalid());
    }
    #[allow(unsafe_code, reason = "query the effective user ID through libc")]
    // SAFETY: geteuid takes no arguments and has no caller-side preconditions.
    let uid = unsafe { libc::geteuid() };
    let mut parents = vec![File::open("/")?];
    let components: Vec<_> = directory.components().collect();
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::RootDir => continue,
            Component::Normal(part) => {
                let part = name(part)?;
                let parent = parents.last().unwrap();
                let next = match open_at(parent, &part, O_RDONLY | O_DIRECTORY, 0) {
                    Ok(file) => file,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        #[allow(
                            unsafe_code,
                            reason = "create a directory relative to the live parent FD"
                        )]
                        // SAFETY: parent keeps its descriptor live and part is
                        // a live NUL-terminated component name.
                        let created =
                            unsafe { libc::mkdirat(parent.as_raw_fd(), part.as_ptr(), 0o700) };
                        if created != 0 {
                            let error = io::Error::last_os_error();
                            if error.kind() != io::ErrorKind::AlreadyExists {
                                return Err(error);
                            }
                        }
                        open_at(parent, &part, O_RDONLY | O_DIRECTORY, 0)?
                    }
                    Err(error) => return Err(error),
                };
                let meta = next.metadata()?;
                let mode = meta.mode() & 0o7777;
                let final_dir = index + 1 == components.len();
                // Root-owned sticky temp directories cannot rename somebody else's
                // protected child; needed only for isolated tests under /tmp.
                let sticky_root = meta.uid() == 0 && mode & 0o1000 != 0;
                if !meta.is_dir()
                    || (meta.uid() != uid && meta.uid() != 0)
                    || (mode & 0o022 != 0 && !sticky_root)
                    || (final_dir && (meta.uid() != uid || mode != 0o700))
                {
                    return Err(invalid());
                }
                parents.push(next);
            }
            _ => return Err(invalid()),
        }
    }
    let parent = parents.last().unwrap();
    let part = name(std::ffi::OsStr::new(role))?;
    let file = match open_at(parent, &part, O_RDWR | O_CREAT | O_EXCL, 0o600) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            open_at(parent, &part, O_RDWR, 0)?
        }
        Err(error) => return Err(error),
    };
    let meta = file.metadata()?;
    if !meta.is_file() || meta.uid() != uid || meta.mode() & 0o7777 != 0o600 || meta.nlink() != 1 {
        return Err(invalid());
    }
    Ok(SecureFile {
        file,
        _directories: parents,
    })
}
