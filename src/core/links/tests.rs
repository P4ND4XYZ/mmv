use super::*;
use std::path::PathBuf;

fn document() -> PathBuf {
    if cfg!(windows) {
        r"C:\docs\sample.md".into()
    } else {
        "/docs/sample.md".into()
    }
}

fn links(values: &[(&str, &str)]) -> Vec<LinkCandidate> {
    candidates(
        &document(),
        values
            .iter()
            .map(|(name, url)| (name.to_string(), url.to_string(), None)),
    )
}

#[test]
fn dedup_requires_both_name_and_resolved_destination_and_keeps_first_identity() {
    let values = links(&[
        ("Design", "./design.md#one"),
        ("Design", "design.md#one"),
        ("Design", "design.md#two"),
        ("Other name", "design.md#one"),
        ("Section", "#one"),
        ("Section", "sample.md#one"),
        ("Web", "https://example.com"),
        ("unsafe", "javascript:alert(1)"),
    ]);
    assert_eq!(
        values.iter().map(|c| c.id.0).collect::<Vec<_>>(),
        [0, 2, 3, 4, 6]
    );
    assert_eq!(
        values[0].target,
        LinkTarget::File {
            path: document().with_file_name("design.md"),
            fragment: Some("one".into())
        }
    );
    assert_eq!(values[0].kind, LinkKind::LocalMarkdownHeading);
    assert_eq!(values[3].kind, LinkKind::Heading);
    assert_eq!(values[4].kind, LinkKind::Web);
}

#[test]
fn first_duplicate_retains_its_source_range() {
    let values = candidates(
        &document(),
        [
            ("Design".into(), "./design.md".into(), Some(3..22)),
            ("Design".into(), "design.md".into(), Some(90..109)),
            ("Other".into(), "other.md".into(), None),
        ],
    );
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].source_range, Some(3..22));
    assert_eq!(values[0].id, CandidateId(0));
}

#[test]
fn fuzzy_is_case_insensitive_and_empty_query_is_uniform() {
    let mut matcher = LinkMatcher::default();
    assert_eq!(
        matcher.fuzzy("", "日本語").unwrap(),
        matcher.fuzzy("", "").unwrap()
    );
    assert_eq!(matcher.fuzzy("不存在", "design"), None);
    let (score, ranges) = matcher.fuzzy("日本dsg", "日本語design").unwrap();
    assert_eq!(ranges, [0..6, 9..10, 11..12, 13..14]);
    assert_eq!(
        matcher.fuzzy("日本DSG", "日本語design"),
        Some((score, ranges))
    );
}

#[test]
fn literal_queries_do_not_enable_fzf_operators_or_word_reordering() {
    let mut matcher = LinkMatcher::default();
    for query in ["^", "$", "!", "'", "|", "\\", "f g", "^a$b!c'd|e\\f g"] {
        assert!(
            matcher.fuzzy(query, "^a$b!c'd|e\\f g").is_some(),
            "{query:?}"
        );
        assert!(matcher.fuzzy(query, "abcdefg").is_none(), "{query:?}");
        assert!(matcher.fuzzy(query, "g f").is_none(), "{query:?}");
    }
    assert!(matcher.fuzzy("不存在", "abcdefg").is_none());
}

#[test]
fn japanese_combining_and_emoji_indices_convert_to_valid_utf8_byte_highlights() {
    let text = "e\u{301}👩‍💻日本語design";
    let mut matcher = LinkMatcher::default();
    for (query, expected) in [
        ("日本", vec![14..20]),
        ("e\u{301}", vec![0..3]),
        ("👩‍💻", vec![3..14]),
        ("dsg", vec![23..24, 25..26, 27..28]),
    ] {
        let (_, ranges) = matcher.fuzzy(query, text).unwrap();
        assert_eq!(ranges, expected, "{query:?}");
        for range in ranges {
            assert!(text.get(range).is_some());
        }
    }
}
