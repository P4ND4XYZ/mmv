use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

use crate::app::file_tree::{Completion, DirectoryRequest, Entry, Request, RootRequest};

pub fn execute(request: Request) -> Completion {
    match request {
        Request::Root(request) => {
            let root = find_root(&request);
            Completion::Root(request, root)
        }
        Request::Directory(request) => {
            let result = list_directory(&request).map_err(|error| {
                super::logging::log_document_failure(&request.path, &error.to_string());
            });
            Completion::Directory(request, result)
        }
    }
}

fn find_root(request: &RootRequest) -> PathBuf {
    find_root_with(&request.document, &request.markers, |path| {
        fs::symlink_metadata(path).map(|_| ())
    })
}

fn find_root_with(
    document: &Path,
    markers: &[String],
    mut metadata: impl FnMut(&Path) -> io::Result<()>,
) -> PathBuf {
    let parent = document.parent().unwrap_or(document);
    for ancestor in parent.ancestors() {
        for marker in markers {
            match metadata(&ancestor.join(marker)) {
                Ok(()) => return ancestor.to_owned(),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                // A denied probe is not "no marker". Quietly use the document
                // parent, independently of whether its listing is accessible.
                Err(_) => return parent.to_owned(),
            }
        }
        if markers.is_empty() {
            break;
        }
    }
    parent.to_owned()
}

pub(super) fn reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        reparse_attributes(metadata.file_attributes())
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(windows)]
fn reparse_attributes(attributes: u32) -> bool {
    attributes & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn expandable(metadata: &fs::Metadata, name: &std::ffi::OsStr) -> bool {
    metadata.is_dir() && !reparse(metadata) && !name.to_string_lossy().eq_ignore_ascii_case(".git")
}

pub(super) fn check_directory_path(root: &Path, path: &Path) -> io::Result<()> {
    let relative = path.strip_prefix(root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory is outside the tree root",
        )
    })?;
    let mut current = root.to_owned();
    // Check root first, then each component before inspecting its child. A
    // symlink_metadata call on the leaf alone follows replaced ancestors.
    // Ancestors above the designated root are deliberately not inspected.
    check_directory(&current)?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid tree directory path",
            ));
        };
        current.push(name);
        check_directory(&current)?;
    }
    // These checks do not make traversal atomic against later replacements.
    Ok(())
}

fn check_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !expandable(&metadata, path.file_name().unwrap_or(path.as_os_str())) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory is not expandable",
        ));
    }
    Ok(())
}

fn list_directory(request: &DirectoryRequest) -> io::Result<Vec<Entry>> {
    check_directory_path(&request.root, &request.path)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&request.path)? {
        let entry = entry?;
        // DirEntry::metadata does not traverse symbolic links.
        let metadata = entry.metadata()?;
        let name = entry.file_name();
        entries.push(Entry {
            path: entry.path(),
            directory: metadata.is_dir(),
            expandable: expandable(&metadata, &name),
            reparse: reparse(&metadata),
            name,
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| {
                a.name
                    .to_string_lossy()
                    .to_lowercase()
                    .cmp(&b.name.to_string_lossy().to_lowercase())
            })
            .then_with(|| a.name.cmp(&b.name))
    });
}

#[cfg(test)]
mod tests;
