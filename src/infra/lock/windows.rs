use super::SecureFile;
use crate::infra::security::{Handle, UserSecurity, invalid};
use std::{
    fs::File,
    io,
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::{Component, Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, GENERIC_READ, GENERIC_WRITE},
    Security::Authorization::SE_FILE_OBJECT,
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
        OPEN_ALWAYS, OPEN_EXISTING, READ_CONTROL,
    },
};

fn name(path: &Path) -> io::Result<Vec<u16>> {
    let mut units: Vec<_> = path.as_os_str().encode_wide().collect();
    if units.contains(&0) {
        return Err(invalid("NUL in instance path"));
    }
    units.push(0);
    Ok(units)
}
fn check(handle: &Handle, directory: bool) -> io::Result<()> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    #[allow(unsafe_code, reason = "read metadata from the opened Windows handle")]
    // SAFETY: handle keeps the kernel handle live and info is a writable output.
    let queried = unsafe { GetFileInformationByHandle(handle.0, &mut info) };
    if queried == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory
        || (!directory && info.nNumberOfLinks != 1)
    {
        return Err(invalid("untrusted instance file type or link"));
    }
    Ok(())
}
pub(super) fn open(directory: &Path, role: &str) -> io::Result<SecureFile> {
    if !directory.is_absolute() {
        return Err(invalid("instance directory must be absolute"));
    }
    let security = UserSecurity::current()?;
    let mut parents = Vec::new();
    let mut current = PathBuf::new();
    for component in directory.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                current.push(component.as_os_str());
                continue;
            }
            Component::Normal(_) => current.push(component.as_os_str()),
            _ => return Err(invalid("invalid instance directory component")),
        }
        let path = name(&current)?;
        let mut created = false;
        #[allow(
            unsafe_code,
            reason = "create a directory with the current-user descriptor"
        )]
        // SAFETY: path is NUL-terminated; the attributes and their descriptor
        // remain live for this synchronous call.
        let created_result = unsafe { CreateDirectoryW(path.as_ptr(), &security.attributes()) };
        if created_result == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
                return Err(error);
            }
        } else {
            created = true;
        }
        // No FILE_SHARE_DELETE: each verified path component remains pinned while
        // descendants are opened and while the final file lock is held.
        #[allow(unsafe_code, reason = "open a pinned directory handle through Win32")]
        // SAFETY: path is a live NUL-terminated string. The optional attributes
        // and template handle are null; Handle::new takes the successful handle.
        let raw = unsafe {
            CreateFileW(
                path.as_ptr(),
                FILE_READ_ATTRIBUTES | READ_CONTROL,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        let handle = Handle::new(raw)?;
        check(&handle, true)?;
        if created || current == directory {
            security.verify(handle.0, SE_FILE_OBJECT)?;
        }
        #[allow(unsafe_code, reason = "transfer the verified directory handle to File")]
        // SAFETY: The handle is valid and owned; into_raw suppresses its close
        // so File becomes the sole owner.
        let parent = unsafe { File::from_raw_handle(handle.into_raw()) };
        parents.push(parent);
    }
    let path = name(&directory.join(role))?;
    #[allow(unsafe_code, reason = "open or create the lock file through Win32")]
    // SAFETY: path is live and NUL-terminated. The attributes and descriptor
    // remain live for the call, and the optional template handle is null.
    let raw = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &security.attributes(),
            OPEN_ALWAYS,
            FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    let handle = Handle::new(raw)?;
    check(&handle, false)?;
    security.verify(handle.0, SE_FILE_OBJECT)?;
    #[allow(unsafe_code, reason = "transfer the verified lock-file handle to File")]
    // SAFETY: The handle is valid and owned; into_raw suppresses its close
    // so File becomes the sole owner.
    let file = unsafe { File::from_raw_handle(handle.into_raw()) };
    Ok(SecureFile {
        file,
        _directories: parents,
    })
}
