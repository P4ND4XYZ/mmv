use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mmv-tree-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn request(path: PathBuf) -> DirectoryRequest {
    DirectoryRequest {
        id: 1,
        generation: 1,
        root: path.clone(),
        path,
    }
}

#[test]
fn real_listing_includes_hidden_and_all_extensions_sorts_without_natural_order() {
    let fixture = Fixture::new();
    for name in ["z-dir", ".git", "A-dir"] {
        fs::create_dir(fixture.0.join(name)).unwrap();
    }
    for name in ["z.txt", ".hidden", "a10.png", "a2.md", "B.bin"] {
        fs::write(fixture.0.join(name), "").unwrap();
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;
        let name: Vec<u16> = fixture
            .0
            .join(".hidden")
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        assert_ne!(
            unsafe {
                windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(
                    name.as_ptr(),
                    windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN,
                )
            },
            0
        );
    }
    let entries = list_directory(&request(fixture.0.clone())).unwrap();
    let names: Vec<_> = entries.iter().map(|e| e.name.to_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            ".git", "A-dir", "z-dir", ".hidden", "a10.png", "a2.md", "B.bin", "z.txt"
        ]
    );
    assert!(!entries[0].expandable);
    assert!(entries[1].expandable && entries[2].expandable);
    assert!(list_directory(&request(fixture.0.join(".git"))).is_err());
    assert!(list_directory(&request(fixture.0.join("missing"))).is_err());
}

#[test]
fn nearest_marker_wins_independent_of_marker_order_and_empty_means_parent() {
    let fixture = Fixture::new();
    let parent = fixture.0.join("near").join("doc");
    fs::create_dir_all(&parent).unwrap();
    fs::write(fixture.0.join(".git"), "").unwrap();
    fs::create_dir(fixture.0.join("near").join(".marksman.toml")).unwrap();
    let document = parent.join("sample.md");
    for markers in [
        vec![".git".into(), ".marksman.toml".into()],
        vec![".marksman.toml".into(), ".git".into()],
    ] {
        assert_eq!(
            find_root(&RootRequest {
                id: 1,
                document: document.clone(),
                markers
            }),
            fixture.0.join("near")
        );
    }
    assert_eq!(
        find_root(&RootRequest {
            id: 2,
            document: document.clone(),
            markers: Vec::new()
        }),
        parent
    );
    assert_eq!(
        find_root(&RootRequest {
            id: 3,
            document,
            markers: vec!["no-such-marker".into()]
        }),
        parent
    );
}

#[test]
fn denied_marker_probe_quietly_falls_back_instead_of_using_farther_marker() {
    let document = PathBuf::from("/repo/near/doc.md");
    let root = find_root_with(&document, &["marker".into()], |path| {
        if path.parent() == document.parent() {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        } else {
            Ok(())
        }
    });
    assert_eq!(root, document.parent().unwrap());
}

#[cfg(windows)]
#[test]
fn marker_search_stops_at_drive_or_unc_share_and_empty_markers_do_not_probe() {
    for (document, expected) in [
        (r"C:\repo\doc.md", vec![r"C:\repo", r"C:\"]),
        (
            r"\\server\share\repo\doc.md",
            vec![r"\\server\share\repo", r"\\server\share\"],
        ),
    ] {
        let mut probes = Vec::new();
        let document = Path::new(document);
        let root = find_root_with(document, &["marker".into()], |path| {
            probes.push(path.parent().unwrap().to_owned());
            Err(io::ErrorKind::NotFound.into())
        });
        assert_eq!(root, document.parent().unwrap());
        assert_eq!(
            probes,
            expected.into_iter().map(PathBuf::from).collect::<Vec<_>>()
        );
    }
    assert_eq!(
        find_root_with(Path::new(r"C:\repo\doc.md"), &[], |_| panic!(
            "empty markers must not probe"
        )),
        PathBuf::from(r"C:\repo")
    );
}

#[test]
fn case_ties_use_original_name() {
    let mut entries = [
        Entry {
            path: "a".into(),
            name: "a".into(),
            directory: false,
            expandable: false,
            reparse: false,
        },
        Entry {
            path: "A".into(),
            name: "A".into(),
            directory: false,
            expandable: false,
            reparse: false,
        },
    ];
    sort_entries(&mut entries);
    assert_eq!(entries[0].name, "A");
}

#[cfg(windows)]
struct Junction(PathBuf);

#[cfg(windows)]
impl Junction {
    fn new(path: PathBuf, target: &Path) -> Self {
        let output = std::process::Command::new("cmd")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&path)
            .arg(target)
            .output()
            .unwrap();
        assert!(output.status.success(), "junction creation: {output:?}");
        Self(path)
    }
}

#[cfg(windows)]
impl Drop for Junction {
    fn drop(&mut self) {
        fs::remove_dir(&self.0).unwrap();
    }
}

#[cfg(windows)]
#[test]
fn tree_open_revalidates_replaced_ancestor_and_leaf_junctions() {
    use crate::app::{App, DocumentOperation, OperationOrigin, OperationResult};
    for replace_ancestor in [false, true] {
        let fixture = Fixture::new();
        let root = fixture.0.join("root");
        let parent = root.join("parent");
        let target = fixture.0.join("target");
        fs::create_dir_all(&parent).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(parent.join("doc.md"), "original").unwrap();
        fs::write(target.join("doc.md"), "must not be read").unwrap();
        let mut app = App::default();
        let request = match app.operate(
            DocumentOperation::TreeOpen {
                path: parent.join("doc.md"),
                root: root.clone(),
            },
            OperationOrigin::Keyboard,
            0.,
            None,
        ) {
            OperationResult::Pending(request) => request,
            _ => panic!(),
        };
        let replaced = if replace_ancestor {
            parent
        } else {
            parent.join("doc.md")
        };
        fs::rename(&replaced, fixture.0.join("original")).unwrap();
        let junction = Junction::new(replaced, &target);
        let result = crate::infra::read_document(request);
        assert!(
            result
                .content
                .err()
                .unwrap()
                .starts_with("Tree target is not a regular accessible file")
        );
        assert!(app.document().is_none());
        drop(junction);
    }
}

#[cfg(windows)]
#[test]
fn ancestor_junction_replacement_rejects_descendant_listing_before_adoption() {
    use crate::app::file_tree::{Action, FileTree};
    for refresh in [false, true] {
        let fixture = Fixture::new();
        let root = fixture.0.join("root");
        let nested = root.join("parent/nested");
        let target = fixture.0.join("target");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("retained.txt"), "original").unwrap();
        fs::create_dir_all(target.join("nested")).unwrap();
        fs::write(target.join("nested/external-secret.txt"), "external").unwrap();
        let mut tree = FileTree::default();
        let probe = tree
            .document_changed(root.join("doc.md"), true, &[])
            .remove(0);
        let initial = tree.complete(execute(probe)).remove(0);
        assert!(tree.complete(execute(initial)).is_empty());
        tree.act(Action::Next);
        let parent = tree.act(Action::Right).remove(0);
        tree.complete(execute(parent));
        tree.act(Action::Right);
        assert_eq!(tree.cursor.as_ref(), Some(&nested));
        if refresh {
            let listing = tree.act(Action::Right).remove(0);
            tree.complete(execute(listing));
            tree.act(Action::Right);
        }
        let previous = tree.rows();
        let cursor = tree.cursor.clone();
        fs::rename(root.join("parent"), fixture.0.join("original-parent")).unwrap();
        let junction = Junction::new(root.join("parent"), &target);
        // Replacement is complete before requesting the descendant, rather than
        // a race between this request's metadata check and enumeration.
        let listing = if refresh {
            tree.act(Action::Refresh).into_iter().find(|request| matches!(request, Request::Directory(request) if request.path == nested)).unwrap()
        } else {
            tree.act(Action::Right).remove(0)
        };
        let completion = execute(listing);
        let denied = matches!(&completion, Completion::Directory(_, Err(())));
        tree.complete(completion);
        drop(junction);
        assert!(
            denied,
            "an ancestor junction must fail before listing its target"
        );
        assert_eq!(
            tree.rows(),
            previous,
            "failure must retain the existing listing"
        );
        assert_eq!(tree.cursor, cursor);
        assert!(tree.node(&nested).unwrap().error);
        assert!(!tree.rows().iter().any(|(path, _)| {
            path.file_name()
                .is_some_and(|name| name == "external-secret.txt")
        }));
    }
}

#[cfg(windows)]
#[test]
fn replaced_tree_root_rejects_both_root_and_descendant_listings() {
    let fixture = Fixture::new();
    let root = fixture.0.join("root");
    let target = fixture.0.join("target");
    fs::create_dir_all(root.join("nested")).unwrap();
    fs::create_dir_all(target.join("nested")).unwrap();
    fs::write(target.join("nested/external-secret.txt"), "external").unwrap();
    fs::rename(&root, fixture.0.join("original-root")).unwrap();
    let junction = Junction::new(root.clone(), &target);
    let root_result = list_directory(&request(root.clone()));
    let mut descendant = request(root.join("nested"));
    descendant.root = root;
    let descendant_result = list_directory(&descendant);
    drop(junction);
    assert!(root_result.is_err());
    assert!(
        descendant_result.is_err(),
        "checking only the leaf would follow the replaced root"
    );
}

#[cfg(windows)]
#[test]
fn reparse_ancestors_above_designated_root_are_not_blanket_rejected() {
    let fixture = Fixture::new();
    let target = fixture.0.join("target");
    let alias = fixture.0.join("alias");
    fs::create_dir_all(target.join("normal-root/nested")).unwrap();
    fs::write(target.join("normal-root/doc.md"), "body").unwrap();
    fs::write(target.join("normal-root/nested/allowed.txt"), "in root").unwrap();
    let junction = Junction::new(alias.clone(), &target);
    let root = alias.join("normal-root");
    let mut descendant = request(root.join("nested"));
    descendant.root = root.clone();
    let entries = list_directory(&descendant).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "allowed.txt");
    let document = fs::canonicalize(root.join("doc.md")).unwrap();
    let expected = fs::canonicalize(target.join("normal-root")).unwrap();
    assert_eq!(
        find_root(&RootRequest {
            id: 1,
            document,
            markers: Vec::new()
        }),
        expected
    );
    drop(junction);
}

#[test]
fn listing_checks_git_and_path_containment_from_the_designated_root() {
    let fixture = Fixture::new();
    let root = fixture.0.join("root");
    fs::create_dir_all(root.join(".git/nested")).unwrap();
    fs::create_dir(fixture.0.join("outside")).unwrap();
    for path in [
        root.join(".git"),
        root.join(".git/nested"),
        root.join("../outside"),
        fixture.0.join("outside"),
    ] {
        let mut listing = request(path);
        listing.root = root.clone();
        assert!(
            list_directory(&listing).is_err(),
            "unexpectedly listed {}",
            listing.path.display()
        );
    }
    assert!(
        list_directory(&request(root.join(".git"))).is_err(),
        ".git as root is also not expandable"
    );
    assert!(
        list_directory(&request(root.join(".git/nested"))).is_ok(),
        "ancestors above the designated root are not tree expansion"
    );
}

#[cfg(windows)]
#[test]
fn windows_reparse_attributes_and_real_junction_are_not_expandable() {
    use std::os::windows::fs::MetadataExt as _;
    let fixture = Fixture::new();
    let target = fixture.0.join("target");
    let junction = fixture.0.join("junction");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("secret.md"), "not enumerated").unwrap();
    fs::create_dir(&junction).unwrap();
    assert!(
        list_directory(&request(fixture.0.clone()))
            .unwrap()
            .iter()
            .find(|entry| entry.name == "junction")
            .unwrap()
            .expandable
    );
    fs::remove_dir(&junction).unwrap();
    // Replace an already-enumerated ordinary directory before its expansion.
    let output = std::process::Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&target)
        .output()
        .unwrap();
    assert!(output.status.success(), "junction creation: {:?}", output);
    let entries = list_directory(&request(fixture.0.clone())).unwrap();
    let link = entries.iter().find(|e| e.name == "junction").unwrap();
    assert!(!link.expandable);
    let metadata = fs::symlink_metadata(&junction).unwrap();
    assert_ne!(
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT,
        0
    );
    assert!(reparse(&metadata));
    // Classification depends on the reparse attribute, not a particular tag or
    // FileType::is_symlink implementation (including non-link reparse points).
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_REPARSE_POINT,
    };
    assert!(reparse_attributes(FILE_ATTRIBUTE_REPARSE_POINT));
    assert!(reparse_attributes(
        FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY
    ));
    assert!(!reparse_attributes(
        FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_HIDDEN
    ));
    assert!(list_directory(&request(junction.clone())).is_err());
    // Remove the junction itself before recursive fixture cleanup.
    fs::remove_dir(junction).unwrap();
}
