//! Safe file creation is OS-specific; lock ownership and lifetime are common.
#[cfg(target_os = "linux")]
mod unix;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
use unix as platform;
#[cfg(windows)]
use windows as platform;
#[cfg(test)]
mod tests;

use super::paths::Namespace;
use std::{fs::File, io};

pub(super) struct SecureFile {
    pub file: File,
    // Pin ancestors until the file is closed. Do not allow rename/delete races.
    pub _directories: Vec<File>,
}
pub(super) fn open(namespace: &Namespace, role: &str) -> io::Result<SecureFile> {
    if !matches!(role, "live" | "startup") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown instance lock role",
        ));
    }
    platform::open(&namespace.directory, &format!("{role}.lock"))
}
