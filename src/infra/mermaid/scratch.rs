//! Short-lived Mermaid files only; this is not a persistent diagram cache.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const ROOT: &str = "my-markdown-viewer-mermaid-v1";
const LEASE: &str = "owner.lease";
const MARKER: &[u8] = b"my-markdown-viewer Mermaid scratch v1\n";

fn user_root(base: &Path, sid: &str) -> PathBuf {
    base.join(format!("{ROOT}-{sid}"))
}

fn current_root() -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        // TEMP may be shared between accounts. Use the same trusted Windows SID
        // source as IPC, never an environment-supplied user name or a shared root.
        let user = crate::infra::security::UserSecurity::current()?;
        Ok(user_root(&std::env::temp_dir(), &user.sid_text))
    }
    #[cfg(not(windows))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Mermaid scratch requires a Windows user SID",
        ))
    }
}

pub(super) struct Scratch {
    path: PathBuf,
    // Keep ownership through deletion, including when unwinding after a failure.
    _lease: File,
}

impl Scratch {
    pub(super) fn new() -> io::Result<Self> {
        Self::in_root(&current_root()?)
    }

    fn in_root(root: &Path) -> io::Result<Self> {
        match fs::create_dir(root) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error),
        }
        if !ordinary_directory(root)? {
            return Err(io::Error::other(
                "Mermaid temporary root is not an ordinary directory",
            ));
        }
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = root.join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    // Until the lease is initialized there are no generated files.
                    // An interrupted empty directory is deliberately left alone.
                    let initialized = (|| {
                        let mut lease = open_lease(&path.join(LEASE), true)?;
                        lease.write_all(MARKER)?;
                        Ok(Self {
                            path: path.clone(),
                            _lease: lease,
                        })
                    })();
                    if initialized.is_err() {
                        let _ = fs::remove_file(path.join(LEASE));
                        let _ = fs::remove_dir(&path);
                    }
                    return initialized;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = remove_scratch(&self.path) {
            eprintln!(
                "removing Mermaid temporary directory {}: {error}",
                self.path.display()
            );
        }
    }
}

fn open_lease(path: &Path, create: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(create);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        // Permit deletion while held, but not another reader/writer. Besides
        // protecting a live render, this prevents a second cleaner from retaining
        // an old handle across deletion and later acting on a reused path.
        options.share_mode(0x0000_0004); // FILE_SHARE_DELETE
    }
    let file = options.open(path)?;
    file.try_lock().map_err(io::Error::other)?;
    Ok(file)
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        // Includes junctions and other reparse points, not just symbolic links.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn ordinary_directory(path: &Path) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(metadata.is_dir() && !is_link(&metadata))
}

pub(super) fn cleanup() {
    match current_root() {
        Ok(root) => cleanup_in(&root),
        Err(error) => eprintln!("locating current-user Mermaid temporary files: {error}"),
    }
}

fn cleanup_in(root: &Path) {
    let entries = (|| {
        if !ordinary_directory(root)? {
            return Ok(None);
        }
        fs::read_dir(root).map(Some)
    })();
    match entries {
        Ok(Some(entries)) => {
            for entry in entries {
                let result = entry.and_then(|entry| cleanup_entry(&entry.path()));
                if let Err(error) = result {
                    eprintln!("cleaning Mermaid temporary files: {error}");
                }
            }
        }
        Ok(None) => (),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => eprintln!("scanning Mermaid temporary files: {error}"),
    }
}

fn cleanup_entry(path: &Path) -> io::Result<()> {
    if !ordinary_directory(path)? {
        return Ok(());
    }
    let marker = path.join(LEASE);
    let metadata = match fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || is_link(&metadata) || metadata.len() != MARKER.len() as u64 {
        return Ok(());
    }
    // A failed exclusive open/lock means live, inaccessible or concurrently
    // cleaned. None of these is evidence that it is safe to delete the directory.
    let Ok(mut lease) = open_lease(&marker, false) else {
        return Ok(());
    };
    let mut content = [0; MARKER.len()];
    lease.read_exact(&mut content)?;
    if content != MARKER {
        return Ok(());
    }
    remove_scratch(path)
}

fn remove_scratch(path: &Path) -> io::Result<()> {
    if !ordinary_directory(path)? {
        return Err(io::Error::other(
            "Mermaid scratch is not an ordinary directory",
        ));
    }
    // Keep the marker if a generated file cannot be removed, so a later startup
    // can retry. std's remove_dir_all removes links, never their targets.
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_name() == LEASE {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        #[cfg(windows)]
        let directory = {
            use std::os::windows::fs::MetadataExt as _;
            // A directory junction is not Metadata::is_dir(), but must still be
            // unlinked as a directory. remove_dir_all does not follow it.
            metadata.file_attributes() & 0x10 != 0
        };
        #[cfg(not(windows))]
        let directory = metadata.is_dir();
        if directory {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    // The lease is still held, including across unlinking the marker.
    fs::remove_file(path.join(LEASE))?;
    fs::remove_dir(path)
}

#[cfg(test)]
mod tests;
