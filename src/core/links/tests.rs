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
fn first_duplicate_retains_its_source_range_after_reranking() {
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
    let ranked = LinkMatcher::default().rank(&values, "dsg");
    assert_eq!(ranked[0].id, values[0].id);
    assert_eq!(values[0].source_range, Some(3..22));
}

#[test]
fn empty_query_and_equal_scores_follow_document_order_even_if_input_is_reordered() {
    let mut values = links(&[("design", "#one"), ("design", "#two"), ("other", "#three")]);
    values.reverse();
    let mut matcher = LinkMatcher::default();
    assert_eq!(
        matcher
            .rank(&values, "")
            .iter()
            .map(|m| m.id.0)
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    let ranked = matcher.rank(&values, "dsg");
    assert_eq!(ranked.iter().map(|m| m.id.0).collect::<Vec<_>>(), [0, 1]);
    assert_eq!(ranked[0].score, ranked[1].score);
    assert_eq!(ranked[0].name_ranges, [0..1, 2..3, 4..5]);
}

#[test]
fn ranking_uses_best_field_score_and_preserves_ids_after_filtering() {
    let values = links(&[
        ("distant slow gap", "#else"),
        ("DSG", "#else"),
        ("Other", "design.md"),
    ]);
    let mut matcher = LinkMatcher::default();
    let ranked = matcher.rank(&values, "dsg");
    assert_eq!(ranked[0].id, values[1].id);
    assert!(ranked.windows(2).all(|pair| pair[0].score >= pair[1].score));
    let destination = ranked.iter().find(|m| m.id == values[2].id).unwrap();
    assert!(destination.name_ranges.is_empty());
    assert!(!destination.destination_ranges.is_empty());
    assert_eq!(matcher.rank(&values, "DSG"), ranked);
}

#[test]
fn literal_queries_do_not_enable_fzf_operators_or_word_reordering() {
    let values = links(&[
        ("^a$b!c'd|e\\f g", "#target"),
        ("abcdefg", "#target"),
        ("g f", "#target"),
    ]);
    let mut matcher = LinkMatcher::default();
    for query in ["^", "$", "!", "'", "|", "\\", "f g", "^a$b!c'd|e\\f g"] {
        let found = matcher.rank(&values, query);
        assert_eq!(found.len(), 1, "{query:?}");
        assert_eq!(found[0].id, values[0].id);
    }
    assert!(matcher.rank(&values, "不存在").is_empty());
}

#[test]
fn japanese_combining_and_emoji_indices_convert_to_valid_utf8_byte_highlights() {
    let values = links(&[("e\u{301}👩‍💻日本語design", "#e\u{301}👩‍💻日本語design")]);
    let mut matcher = LinkMatcher::default();
    for (query, expected) in [
        ("日本", std::iter::once(14..20).collect::<Vec<_>>()),
        ("e\u{301}", std::iter::once(0..3).collect::<Vec<_>>()),
        ("👩‍💻", std::iter::once(3..14).collect::<Vec<_>>()),
        ("dsg", vec![23..24, 25..26, 27..28]),
    ] {
        let result = matcher.rank(&values, query);
        assert_eq!(result.len(), 1, "{query:?}");
        assert_eq!(result[0].name_ranges, expected, "{query:?}");
        for range in &result[0].name_ranges {
            assert!(values[0].display_name.get(range.clone()).is_some());
        }
        assert_eq!(
            result[0].destination_ranges,
            result[0]
                .name_ranges
                .iter()
                .map(|r| r.start + 1..r.end + 1)
                .collect::<Vec<_>>()
        );
    }
}
