use super::*;
use crate::infra::instance::{InstanceGuard, gui_is_alive};

struct Fixture(Namespace);
impl Fixture {
    fn new() -> Self {
        Self(Namespace::isolated())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0.directory);
    }
}

#[test]
fn secure_creation_stale_file_and_hardlink_are_distinct() {
    let fixture = Fixture::new();
    let n = &fixture.0;
    let file = open(n, "live").unwrap();
    let original = file.file.metadata().unwrap().len();
    drop(file);
    assert!(!gui_is_alive(n).unwrap());
    assert_eq!(
        std::fs::metadata(n.directory.join("live.lock"))
            .unwrap()
            .len(),
        original
    );
    let alias = n.directory.join("alias");
    std::fs::hard_link(n.directory.join("live.lock"), &alias).unwrap();
    assert!(
        open(n, "live").is_err(),
        "hardlinked lock must not be trusted"
    );
    std::fs::remove_file(alias).unwrap();
    assert!(InstanceGuard::claim_gui(n).unwrap().is_some());
}

#[cfg(windows)]
#[test]
fn windows_security_and_noninheritance_and_pinned_ancestor() {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::{GetHandleInformation, HANDLE_FLAG_INHERIT},
        Security::Authorization::SE_FILE_OBJECT,
    };
    let fixture = Fixture::new();
    let file = open(&fixture.0, "live").unwrap();
    let user = crate::infra::security::UserSecurity::current().unwrap();
    for handle in std::iter::once(&file.file).chain(&file._directories) {
        let mut flags = 0;
        #[allow(
            unsafe_code,
            reason = "check Windows handle inheritance flags through Win32"
        )]
        // SAFETY: The borrowed File keeps its handle open, and flags is writable.
        let result = unsafe { GetHandleInformation(handle.as_raw_handle(), &mut flags) };
        assert_ne!(result, 0);
        assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
    }
    user.verify(file.file.as_raw_handle(), SE_FILE_OBJECT)
        .unwrap();
    user.verify(
        file._directories.last().unwrap().as_raw_handle(),
        SE_FILE_OBJECT,
    )
    .unwrap();
    assert!(
        std::fs::rename(
            &fixture.0.directory,
            fixture.0.directory.with_extension("moved")
        )
        .is_err()
    );
}

#[cfg(windows)]
#[test]
fn reparse_directory_is_rejected_without_creating_a_lock_in_target() {
    let fixture = Fixture::new();
    let n = &fixture.0;
    let target = n.directory.with_extension("target");
    std::fs::create_dir(&target).unwrap();
    let output = std::process::Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&n.directory)
        .arg(&target)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "junction creation failed: {:?}",
        output.status
    );
    assert!(gui_is_alive(n).is_err());
    assert!(!target.join("live.lock").exists());
    std::fs::remove_dir(&n.directory).unwrap();
    std::fs::remove_dir(target).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn linux_native_handles_are_cloexec_and_ancestor_symlinks_are_rejected() {
    use std::os::{fd::AsRawFd, unix::fs::symlink};
    let fixture = Fixture::new();
    let file = open(&fixture.0, "live").unwrap();
    for handle in std::iter::once(&file.file).chain(&file._directories) {
        #[allow(unsafe_code, reason = "inspect close-on-exec flags through libc")]
        // SAFETY: The borrowed File keeps its descriptor live; F_GETFD does not
        // take a variadic argument.
        let flags = unsafe { libc::fcntl(handle.as_raw_fd(), libc::F_GETFD) };
        assert!(
            flags >= 0,
            "fcntl failed: {}",
            std::io::Error::last_os_error()
        );
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
    }
    let alias_path = fixture.0.directory.with_extension("symlink");
    symlink(&fixture.0.directory, &alias_path).unwrap();
    let mut alias = fixture.0.clone();
    alias.directory = alias_path.clone();
    assert!(
        open(&alias, "live").is_err(),
        "native O_NOFOLLOW must reject directory symlinks"
    );
    std::fs::remove_file(alias_path).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn linux_modes_links_and_permissions_fail_closed() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let fixture = Fixture::new();
    let n = &fixture.0;
    drop(open(n, "live").unwrap());
    assert_eq!(
        std::fs::metadata(&n.directory).unwrap().mode() & 0o7777,
        0o700
    );
    let path = n.directory.join("live.lock");
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o600);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(gui_is_alive(n).is_err());
    std::fs::remove_file(&path).unwrap();
    symlink("missing-target", &path).unwrap();
    assert!(gui_is_alive(n).is_err());
    assert!(!n.directory.join("missing-target").exists());
    std::fs::remove_file(path).unwrap();
    std::fs::set_permissions(&n.directory, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(gui_is_alive(n).is_err());
}
