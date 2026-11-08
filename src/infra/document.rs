use crate::app::{DocumentKey, LoadedDocument, ReadRequest, ReadResult};

pub fn read_document(request: ReadRequest) -> ReadResult {
    let content = (|| {
        let source = std::fs::read_to_string(&request.path)?;
        let key = DocumentKey(std::fs::canonicalize(&request.path)?);
        Ok::<_, std::io::Error>(LoadedDocument {
            key,
            source: source
                .strip_prefix('\u{feff}')
                .unwrap_or(&source)
                .to_owned(),
        })
    })()
    .map_err(|error| format!("{}: {error}", request.path.display()));
    ReadResult { request, content }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;

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
            let loaded = read_document(app.open(path.clone()));
            assert_eq!(loaded.content.unwrap().source, "# 日本語\r\n本文");
        }
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        let invalid = read_document(app.open(path.clone()));
        std::fs::remove_file(&path).unwrap();
        assert!(
            invalid
                .content
                .err()
                .unwrap()
                .contains(&path.display().to_string())
        );
        let missing = read_document(app.open(path));
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
