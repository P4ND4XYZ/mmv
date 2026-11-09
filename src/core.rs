pub mod links;

use std::{
    collections::HashSet,
    ops::Range,
    path::{Path, PathBuf},
};

/// Prototype condition: case-sensitive, non-overlapping literal UTF-8 search.
pub fn find_matches(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    text.match_indices(query)
        .map(|(start, found)| start..start + found.len())
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum ImageLocation {
    File(PathBuf),
    Url(String),
}

/// The document path must be absolute; relative URLs are resolved against it,
/// not against the process working directory.
pub fn resolve_image(document: &Path, source: &str) -> Result<ImageLocation, String> {
    // A drive-letter path is not a URL scheme on Windows.
    if Path::new(source).is_absolute() {
        return Ok(ImageLocation::File(source.into()));
    }
    let base = url::Url::from_file_path(document)
        .map_err(|_| "document path must be absolute".to_string())?;
    let resolved = base.join(source).map_err(|error| error.to_string())?;
    match resolved.scheme() {
        "file" => resolved
            .to_file_path()
            .map(ImageLocation::File)
            .map_err(|_| "invalid file URL".to_string()),
        "http" | "https" => Ok(ImageLocation::Url(resolved.into())),
        scheme => Err(format!("unsupported image scheme: {scheme}")),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LinkTarget {
    File {
        path: PathBuf,
        fragment: Option<String>,
    },
    Web(String),
    Heading(String),
}

/// Keep the decoded fragment separate from the filesystem path.
pub fn resolve_link(document: &Path, source: &str) -> Result<LinkTarget, String> {
    if source.is_empty() || source.starts_with('#') {
        return decode_fragment(source.strip_prefix('#').unwrap_or(source))
            .map(LinkTarget::Heading);
    }
    // A drive-letter path is not a URL scheme on Windows.
    let (path, fragment) = source
        .split_once('#')
        .map_or((source, None), |(path, id)| (path, Some(id)));
    if Path::new(path).is_absolute() {
        return Ok(LinkTarget::File {
            path: path.into(),
            fragment: fragment.map(decode_fragment).transpose()?,
        });
    }
    let base = url::Url::from_file_path(document)
        .map_err(|_| "document path must be absolute".to_string())?;
    let resolved = base.join(source).map_err(|error| error.to_string())?;
    match resolved.scheme() {
        "file" => Ok(LinkTarget::File {
            path: resolved
                .to_file_path()
                .map_err(|_| "invalid file URL".to_string())?,
            fragment: resolved.fragment().map(decode_fragment).transpose()?,
        }),
        "http" | "https" => Ok(LinkTarget::Web(resolved.into())),
        scheme => Err(format!("unsupported link scheme: {scheme}")),
    }
}

fn decode_fragment(fragment: &str) -> Result<String, String> {
    let mut decoded = Vec::with_capacity(fragment.len());
    let mut bytes = fragment.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes.next().and_then(|c| (c as char).to_digit(16));
            let low = bytes.next().and_then(|c| (c as char).to_digit(16));
            let (Some(high), Some(low)) = (high, low) else {
                return Err("invalid fragment encoding".into());
            };
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(byte);
        }
    }
    String::from_utf8(decoded).map_err(|_| "invalid UTF-8 fragment".into())
}

/// Input is upstream's plain heading text, not Markdown source. One allocator
/// per document reserves every emitted ID, including numbered IDs.
#[derive(Default)]
pub struct HeadingIds(HashSet<String>);

impl HeadingIds {
    pub fn next(&mut self, text: &str) -> String {
        let base: String = text.trim().to_lowercase().chars().filter_map(|c| {
            if c == ' ' {
                Some('-')
            } else if c.is_alphanumeric() || matches!(c, '-' | '_' | '\u{0300}'..='\u{036f}' | '\u{1ab0}'..='\u{1aff}' | '\u{1dc0}'..='\u{1dff}' | '\u{3099}'..='\u{309a}' | '\u{fe20}'..='\u{fe2f}') {
                Some(c)
            } else {
                None
            }
        }).collect();
        let mut id = base.clone();
        let mut suffix = 0;
        while !self.0.insert(id.clone()) {
            suffix += 1;
            id = format!("{base}-{suffix}");
        }
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn japanese_matches_use_distinct_utf8_byte_ranges() {
        assert_eq!(find_matches("日本語と日本語", "日本語"), vec![0..9, 12..21]);
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // Expected values are vectors of match ranges, not indices.
    fn literal_search_conditions_are_explicit() {
        for (text, query, expected) in [
            ("aaa", "aa", vec![0..2]),
            ("a.b A.B", "a.b", vec![0..3]),
            ("a\nb", "a\nb", vec![0..3]),
            ("abc", "", vec![]),
            ("abc", "z", vec![]),
        ] {
            assert_eq!(find_matches(text, query), expected, "query={query:?}");
        }
    }

    fn document() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(r"C:\documents\日本語\sample.md")
        } else {
            PathBuf::from("/documents/日本語/sample.md")
        }
    }

    #[test]
    fn local_links_resolve_from_the_displayed_document_and_keep_decoded_fragments() {
        for (source, expected) in [
            (
                "notes/日本語%20memo.md",
                document().parent().unwrap().join("notes/日本語 memo.md"),
            ),
            (
                "../other.md#section",
                document()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("other.md"),
            ),
        ] {
            assert_eq!(
                resolve_link(&document(), source).unwrap(),
                LinkTarget::File {
                    path: expected,
                    fragment: source.split_once('#').map(|(_, id)| id.into())
                }
            );
        }
        let path = document().with_file_name("other.md");
        let mut file_url = url::Url::from_file_path(&path).unwrap();
        file_url.set_fragment(Some("section"));
        let absolute_with_fragment = format!("{}#section", path.display());
        for source in [
            path.to_str().unwrap(),
            &absolute_with_fragment,
            file_url.as_str(),
        ] {
            assert_eq!(
                resolve_link(&document(), source).unwrap(),
                LinkTarget::File {
                    path: path.clone(),
                    fragment: source.contains('#').then(|| "section".into())
                }
            );
        }
    }

    #[test]
    fn web_and_heading_links_do_not_become_local_files() {
        let web = "https://example.com/page?q=value#section";
        assert_eq!(
            resolve_link(&document(), web).unwrap(),
            LinkTarget::Web(web.into())
        );
        assert_eq!(
            resolve_link(&document(), "#見出し").unwrap(),
            LinkTarget::Heading("見出し".into())
        );
        for source in [
            "javascript:alert(1)",
            "mailto:user@example.com",
            "https://[",
        ] {
            assert!(resolve_link(&document(), source).is_err(), "{source}");
        }
    }

    #[test]
    fn fragments_decode_utf8_once_without_treating_plus_as_space() {
        for (source, id) in [
            ("#%E4%BD%BF%E3%81%84%E6%96%B9", "使い方"),
            ("#使い方", "使い方"),
            ("#a+b%2520", "a+b%20"),
            ("#", ""),
        ] {
            assert_eq!(
                resolve_link(&document(), source).unwrap(),
                LinkTarget::Heading(id.into())
            );
            let source = format!("other.md{source}");
            assert_eq!(
                resolve_link(&document(), &source).unwrap(),
                LinkTarget::File {
                    path: document().with_file_name("other.md"),
                    fragment: Some(id.into()),
                }
            );
        }
        for source in ["#%", "#%FF", "other.md#%GG"] {
            assert!(resolve_link(&document(), source).is_err(), "{source}");
        }
    }

    #[test]
    fn heading_ids_are_document_wide_and_avoid_numbered_id_collisions() {
        let mut ids = HeadingIds::default();
        for (text, expected) in [
            (" Getting Started ", "getting-started"),
            ("使い方", "使い方"),
            ("使い方-1", "使い方-1"),
            ("使い方", "使い方-2"),
            ("使い方-1", "使い方-1-1"),
            ("使い方", "使い方-3"),
            (" API\t &  CLI! 日本語。「」😀 ", "api---cli-日本語"),
            ("Cafe\u{0301}", "cafe\u{0301}"),
            ("!!!", ""),
            ("?", "-1"),
        ] {
            assert_eq!(ids.next(text), expected, "{text:?}");
        }
    }

    #[test]
    fn image_paths_are_document_relative_and_url_decoded() {
        assert_eq!(
            resolve_image(&document(), "images/local%20image.svg").unwrap(),
            ImageLocation::File(document().parent().unwrap().join("images/local image.svg"))
        );
        assert_eq!(
            resolve_image(&document(), "../other.svg").unwrap(),
            ImageLocation::File(
                document()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("other.svg")
            )
        );
    }

    #[test]
    fn absolute_file_and_web_image_sources_are_preserved() {
        let path = document().with_extension("svg");
        let url = url::Url::from_file_path(&path).unwrap();
        assert_eq!(
            resolve_image(&document(), url.as_str()).unwrap(),
            ImageLocation::File(path.clone())
        );
        assert_eq!(
            resolve_image(&document(), path.to_str().unwrap()).unwrap(),
            ImageLocation::File(path)
        );
        assert_eq!(
            resolve_image(&document(), "https://example.com/image.png").unwrap(),
            ImageLocation::Url("https://example.com/image.png".into())
        );
        assert!(resolve_image(&document(), "javascript:alert(1)").is_err());
    }
}
