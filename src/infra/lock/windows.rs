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
    if unsafe { GetFileInformationByHandle(handle.0, &mut info) } == 0 {
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
        if unsafe { CreateDirectoryW(path.as_ptr(), &security.attributes()) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
                return Err(error);
            }
        } else {
            created = true;
        }
        // No FILE_SHARE_DELETE: each verified path component remains pinned while
        // descendants are opened and while the final file lock is held.
        let handle = Handle::new(unsafe {
            CreateFileW(
                path.as_ptr(),
                FILE_READ_ATTRIBUTES | READ_CONTROL,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        })?;
        check(&handle, true)?;
        if created || current == directory {
            security.verify(handle.0, SE_FILE_OBJECT)?;
        }
        parents.push(unsafe { File::from_raw_handle(handle.into_raw()) });
    }
    let path = name(&directory.join(role))?;
    let handle = Handle::new(unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &security.attributes(),
            OPEN_ALWAYS,
            FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    })?;
    check(&handle, false)?;
    security.verify(handle.0, SE_FILE_OBJECT)?;
    Ok(SecureFile {
        file: unsafe { File::from_raw_handle(handle.into_raw()) },
        _directories: parents,
    })
}
