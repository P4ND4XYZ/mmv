use crate::app::{ReadRequest, ReadResult};

pub fn read_document(request: ReadRequest) -> ReadResult {
    let content = std::fs::read_to_string(&request.path)
        .map(|source| {
            source
                .strip_prefix('\u{feff}')
                .unwrap_or(&source)
                .to_owned()
        })
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
        std::fs::write(&path, "\u{feff}# 日本語\r\n本文").unwrap();
        let loaded = read_document(app.open(path.clone()));
        assert_eq!(loaded.content.unwrap(), "# 日本語\r\n本文");
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        let invalid = read_document(app.open(path.clone()));
        std::fs::remove_file(&path).unwrap();
        assert!(
            invalid
                .content
                .unwrap_err()
                .contains(&path.display().to_string())
        );
        let missing = read_document(app.open(path));
        assert!(missing.content.is_err());
    }
}
