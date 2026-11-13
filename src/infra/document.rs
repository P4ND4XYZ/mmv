use crate::app::{DocumentKey, LoadedDocument, ReadPhase, ReadRequest, ReadResponse, ReadResult};

pub fn read_document(request: ReadRequest) -> ReadResponse {
    let key = resolve_key(&request).map_err(|error| report_read_error(&request, error));
    if request.phase == ReadPhase::Resolve {
        return ReadResponse::Resolved { request, key };
    }
    let content = key.and_then(|key| {
        let source = std::fs::read_to_string(&request.path)
            .map_err(|error| report_read_error(&request, error))?;
        Ok(LoadedDocument {
            key,
            source: source
                .strip_prefix('\u{feff}')
                .unwrap_or(&source)
                .to_owned(),
        })
    });
    ReadResponse::Loaded(ReadResult { request, content })
}

fn resolve_key(request: &ReadRequest) -> std::io::Result<DocumentKey> {
    if let Some(tree) = &request.tree {
        let parent = request.path.parent().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid tree file path")
        })?;
        super::file_tree::check_directory_path(&tree.root, parent)?;
        if !request.path.strip_prefix(&tree.root).is_ok_and(|relative| {
            relative
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
        }) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid tree file path",
            ));
        }
        let metadata = std::fs::symlink_metadata(&request.path)?;
        if !metadata.is_file() || super::file_tree::reparse(&metadata) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "tree target is not a regular non-reparse file",
            ));
        }
    }
    let metadata = std::fs::metadata(&request.path)?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "target is not a regular file",
        ));
    }
    Ok(DocumentKey(std::fs::canonicalize(&request.path)?))
}

fn report_read_error(request: &ReadRequest, error: std::io::Error) -> String {
    super::logging::log_document_failure(&request.path, &error.to_string());
    format!(
        "{}: {}",
        read_error_message(&error, request.tree.is_some()),
        request.path.display()
    )
}

fn read_error_message(error: &std::io::Error, tree: bool) -> &'static str {
    use std::io::ErrorKind;
    if matches!(error.raw_os_error(), Some(32 | 33)) {
        return "File access denied or locked";
    }
    match error.kind() {
        ErrorKind::NotFound => "File not found",
        ErrorKind::PermissionDenied | ErrorKind::WouldBlock => "File access denied or locked",
        ErrorKind::InvalidData => "File is not valid UTF-8",
        ErrorKind::InvalidInput if tree => "Tree target is not a regular accessible file",
        _ => "Unable to read file",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;

    fn loaded(response: ReadResponse) -> ReadResult {
        match response {
            ReadResponse::Loaded(result) => result,
            _ => panic!("expected loaded result"),
        }
    }

    fn failed(response: ReadResponse) -> String {
        match response {
            ReadResponse::Loaded(result) => result.content.err().unwrap(),
            ReadResponse::Resolved { key, .. } => key.err().unwrap(),
        }
    }

    fn tree_read(app: &mut App, path: std::path::PathBuf, root: std::path::PathBuf) -> ReadRequest {
        match app.operate(
            crate::app::DocumentOperation::TreeOpen { path, root },
            crate::app::OperationOrigin::Keyboard,
            120.,
            None,
        ) {
            crate::app::OperationResult::Pending(request) => request,
            _ => panic!("tree open requires preflight"),
        }
    }

    #[test]
    fn tree_same_identity_alias_does_not_read_changed_invalid_utf8() {
        let root = std::env::temp_dir().join(format!(
            "mmv-tree-identity-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("日本語.MD");
        std::fs::write(&path, "# Original 日本語").unwrap();
        let mut app = App::default();
        let request = app.open(path.clone());
        app.complete(read_document(request)).unwrap();
        let generation = app.document().unwrap().generation;
        let position = app.position_request().cloned();
        std::fs::write(&path, [0xff]).unwrap();
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        let mut aliases = vec![
            canonical_root.join("日本語.MD"),
            canonical_root.join(".").join("日本語.MD"),
        ];
        #[cfg(windows)]
        aliases.push(canonical_root.join("日本語.md"));
        for alias in aliases {
            // Preserve the alias spelling through the read boundary.
            let request = tree_read(&mut app, alias, canonical_root.clone());
            let completion = app.complete(read_document(request)).unwrap();
            assert!(!completion.adopted);
            assert_eq!(completion.outcome, Ok(()));
            assert_eq!(app.document().unwrap().generation, generation);
            assert_eq!(app.document().unwrap().source.as_str(), "# Original 日本語");
            assert_eq!(app.position_request(), position.as_ref());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tree_preflight_rejects_deleted_replaced_and_escaping_files() {
        let root = std::env::temp_dir().join(format!(
            "mmv-tree-preflight-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let path = root.join("stale.md");
        let mut app = App::default();
        for directory in [false, true] {
            if directory {
                std::fs::create_dir(&path).unwrap();
            }
            let request = tree_read(&mut app, path.clone(), root.clone());
            let error = failed(read_document(request));
            assert!(error.starts_with(if directory {
                "Tree target is not a regular accessible file"
            } else {
                "File not found"
            }));
            if directory {
                std::fs::remove_dir(&path).unwrap();
            }
        }
        for path in [
            root.join("../outside.md"),
            root.with_file_name("outside.md"),
        ] {
            let request = tree_read(&mut app, path, root.clone());
            assert!(
                failed(read_document(request))
                    .starts_with("Tree target is not a regular accessible file")
            );
        }
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn external_alias_open_resolves_without_reading_but_explicit_reload_reads() {
        use crate::app::{DocumentOperation, OperationOrigin, OperationResult, ReadResolution};
        let root = std::env::temp_dir().join(format!(
            "mmv-external-unread-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("日本語.MD");
        std::fs::write(&path, "original").unwrap();
        let mut app = App::default();
        let request = app.open(path.clone());
        app.complete(read_document(request)).unwrap();
        let generation = app.document().unwrap().generation;
        std::fs::write(&path, [0xff]).unwrap();
        let OperationResult::Pending(request) = app.operate(
            DocumentOperation::ExternalOpen {
                path: std::fs::canonicalize(&root)
                    .unwrap()
                    .join(".")
                    .join("日本語.MD"),
                line: std::num::NonZeroU64::new(30),
            },
            OperationOrigin::External,
            0.,
            None,
        ) else {
            panic!();
        };
        let ReadResponse::Resolved { request, key } = read_document(request) else {
            panic!("must only resolve identity");
        };
        let Some(ReadResolution::Complete(completion)) = app.resolve_at(request, key, 0.) else {
            panic!("must not read changed invalid UTF-8");
        };
        assert_eq!(completion.outcome, Ok(()));
        assert!(!completion.adopted);
        assert_eq!(app.document().unwrap().generation, generation);
        assert_eq!(app.document().unwrap().source.as_str(), "original");
        let OperationResult::Pending(request) = app.operate(
            DocumentOperation::Reload,
            OperationOrigin::Keyboard,
            0.,
            None,
        ) else {
            panic!();
        };
        assert!(failed(read_document(request)).starts_with("File is not valid UTF-8"));
        std::fs::remove_file(&path).unwrap();
        let OperationResult::Pending(request) = app.operate(
            DocumentOperation::ExternalOpen { path, line: None },
            OperationOrigin::External,
            0.,
            None,
        ) else {
            panic!();
        };
        assert!(
            app.complete(read_document(request))
                .unwrap()
                .outcome
                .is_err()
        );
        assert_eq!(app.document().unwrap().generation, generation);
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn localized_os_errors_never_reach_gui_messages() {
        let invalid_path = std::io::Error::new(std::io::ErrorKind::InvalidInput, "不正なパス");
        assert_eq!(
            read_error_message(&invalid_path, false),
            "Unable to read file"
        );
        assert_eq!(
            read_error_message(&invalid_path, true),
            "Tree target is not a regular accessible file"
        );
        for (error, expected) in [
            (
                std::io::Error::new(std::io::ErrorKind::NotFound, "見つかりません"),
                "File not found",
            ),
            (
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "アクセス拒否"),
                "File access denied or locked",
            ),
            (
                std::io::Error::new(std::io::ErrorKind::InvalidData, "無効な文字"),
                "File is not valid UTF-8",
            ),
            (
                std::io::Error::other("不明なOSエラー"),
                "Unable to read file",
            ),
            (
                std::io::Error::from_raw_os_error(32),
                "File access denied or locked",
            ),
        ] {
            assert_eq!(read_error_message(&error, true), expected);
        }
    }

    #[test]
    fn reads_utf8_bom_and_reports_invalid_utf8_and_missing_files() {
        let path = std::env::temp_dir().join(format!(
            "mmv-read-{}-{:?}.md",
            std::process::id(),
            std::thread::current().id()
        ));
        let mut app = App::default();
        for source in ["# 日本語\r\n本文", "\u{feff}# 日本語\r\n本文"] {
            std::fs::write(&path, source).unwrap();
            let result = loaded(read_document(app.open(path.clone())));
            assert_eq!(result.content.unwrap().source, "# 日本語\r\n本文");
        }
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        let invalid = loaded(read_document(app.open(path.clone())));
        std::fs::remove_file(&path).unwrap();
        assert!(
            invalid
                .content
                .err()
                .unwrap()
                .contains(&path.display().to_string())
        );
        let missing = loaded(read_document(app.open(path)));
        assert!(missing.content.is_err());
    }

    #[test]
    fn real_file_path_aliases_have_the_same_document_key() {
        // Keep cwd unchanged: other tests can run concurrently.
        let relative_dir = std::path::PathBuf::from("target").join(format!(
            "mmv-identity-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(relative_dir.join("child")).unwrap();
        let relative = relative_dir.join("MixedCase 日本語.md");
        std::fs::write(&relative, "body").unwrap();
        let absolute = std::env::current_dir().unwrap().join(&relative);
        let mut aliases = vec![
            relative.clone(),
            absolute.clone(),
            relative_dir.join(".").join("MixedCase 日本語.md"),
            relative_dir
                .join("child")
                .join("..")
                .join("MixedCase 日本語.md"),
        ];
        #[cfg(windows)]
        {
            aliases.push(std::path::PathBuf::from(
                absolute.to_str().unwrap().to_uppercase(),
            ));
            aliases.push(std::path::PathBuf::from(
                absolute.to_str().unwrap().replace('\\', "/"),
            ));
            aliases.push(std::fs::canonicalize(&relative).unwrap());
        }
        let mut app = App::default();
        let mut expected = None;
        for alias in aliases {
            let result = read_document(app.open(alias));
            assert!(app.complete(result).unwrap().adopted);
            let key = &app.document().unwrap().key;
            if let Some(expected) = &expected {
                assert_eq!(key, expected);
            } else {
                expected = Some(key.clone());
            }
        }
        std::fs::remove_file(relative).unwrap();
        std::fs::remove_dir(relative_dir.join("child")).unwrap();
        std::fs::remove_dir(relative_dir).unwrap();
    }
}
