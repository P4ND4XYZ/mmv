use super::*;
use crate::core::{
    LinkTarget,
    links::{LinkKind, LinkMatcher},
};
use crate::ui::links::extract_candidates;

fn candidates(source: &str) -> Vec<crate::core::links::LinkCandidate> {
    extract_candidates(&PathBuf::from(r"C:\fixtures\sample.md"), source).unwrap()
}

#[test]
fn upstream_markdown_links_include_references_autolinks_tables_and_nested_containers() {
    let values = candidates(
        "[**Design** `API`](design.md#start)\n\n\
         > - [Japanese 日本語][later]\n\n\
         | link | other |\n| --- | --- |\n| [Table](table.md) | cell |\n\n\
         <https://example.com/a>\n\nhttps://example.com/b\n\n\
         [later]: notes/%E6%97%A5%E6%9C%AC%E8%AA%9E.md#%E4%BD%BF%E3%81%84%E6%96%B9\n",
    );
    assert_eq!(
        values
            .iter()
            .map(|v| v.display_name.as_str())
            .collect::<Vec<_>>(),
        [
            "Design API",
            "Japanese 日本語",
            "Table",
            "https://example.com/a",
            "https://example.com/b"
        ]
    );
    assert_eq!(values[0].kind, LinkKind::LocalMarkdownHeading);
    assert_eq!(
        values[1].target,
        LinkTarget::File {
            path: PathBuf::from(r"C:\fixtures\notes\日本語.md"),
            fragment: Some("使い方".into())
        }
    );
    assert_eq!(values[2].kind, LinkKind::LocalMarkdown);
    assert_eq!(values[3].kind, LinkKind::Web);
}

#[test]
fn source_ranges_are_ast_usage_bytes_for_unicode_crlf_multiline_and_references() {
    let source = "日本語\r\n\r\n[**最初**\r\nのリンク](target.md)\r\n\r\n> [参照][id]\r\n\r\n[before ![画像](img.png) after](mixed.md)\r\n\r\n[id]: other.md\r\n";
    let values = candidates(source);
    let expected = [
        "[**最初**\r\nのリンク](target.md)",
        "[参照][id]",
        "[before ![画像](img.png) after](mixed.md)",
    ];
    assert_eq!(values.len(), expected.len());
    for (candidate, expected) in values.iter().zip(expected) {
        let start = source.find(expected).unwrap();
        assert_eq!(candidate.source_range, Some(start..start + expected.len()));
        assert_eq!(&source[candidate.source_range.clone().unwrap()], expected);
    }
}

#[test]
fn decoded_unicode_and_literal_math_keep_the_displayed_link_name() {
    let values = candidates("[e\u{301}👩‍💻 &amp; $5$](unicode.md)");
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].display_name, "e\u{301}👩‍💻 & $5$");
}

#[test]
fn embedded_images_are_excluded_but_complete_links_wrapping_images_are_included() {
    let values = candidates(
        "![standalone](image.png)\n\n\
         [![Preview](image.png)](preview.md)\n\n\
         [before ![alt](image.png) **after**](mixed.md)\n\n\
         [![Reference image][image]][target]\n\n\
         [image]: image.png\n[target]: reference.md\n\n\
         `[not a link](code.md)`\n\n```markdown\n[also not](code.md)\n```",
    );
    assert_eq!(
        values
            .iter()
            .map(|v| v.display_name.as_str())
            .collect::<Vec<_>>(),
        ["Preview", "before alt after", "Reference image"]
    );
    assert!(
        values
            .iter()
            .all(|v| matches!(v.target, LinkTarget::File { .. }))
    );
}

#[test]
fn html_links_in_roots_paragraphs_and_tables_are_excluded_from_picker() {
    let values = candidates(
        "<div><p><a href=\"block.md\">Block <strong>HTML</strong></a></p>\
         <table><tr><td><a href=\"table.md\">HTML table</a></td></tr></table></div>\n\n\
         <p><a href=\"inline.md#part\"><em>Inline</em> HTML</a></p>\n\n\
         [Markdown](normal.md).\n\n<img src=\"image.png\">",
    );
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].display_name, "Markdown");
}

#[test]
fn extraction_deduplicates_only_equal_names_and_resolved_targets_and_is_deterministic() {
    let source = "[日本語](./guide.md#one) [日本語](guide.md#one) \
                  [日本語](guide.md#two) [Different](guide.md#one) \
                  [日本語](guide.md#%E4%BD%BF%E3%81%84%E6%96%B9)";
    let values = candidates(source);
    assert_eq!(values, candidates(source));
    assert_eq!(values.len(), 4);
    assert_eq!(
        values.iter().map(|v| v.document_order).collect::<Vec<_>>(),
        [0, 2, 3, 4]
    );
    let mut matcher = LinkMatcher::default();
    let filtered = matcher.rank(&values, "Different");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, values[2].id);
    assert_eq!(
        values
            .iter()
            .find(|v| v.id == filtered[0].id)
            .unwrap()
            .display_name,
        "Different"
    );
}

#[test]
fn html_anchor_fragments_are_excluded_without_splitting_markdown_image_links() {
    for source in [
        r#"<p><a href="target.md#part">before<img src="image.png" alt="preview">after</a><a href="target.md#part">before</a></p>"#,
        r#"<p><a href="target.md#part">before</a><a href="target.md#part"><img src="image.png" alt="preview"></a><a href="target.md#part">after</a></p>"#,
    ] {
        assert!(candidates(source).is_empty());
    }
    let source = "[before![preview](image.png)after](target.md#part)";
    let markdown = candidates(source);
    assert_eq!(markdown.len(), 1);
    assert_eq!(markdown[0].display_name, "beforepreviewafter");
    assert_eq!(markdown[0].source_range, Some(0..source.len()));
    assert_eq!(
        LinkMatcher::default().rank(&markdown, "beforeafter").len(),
        1
    );
}

#[test]
fn round2_html_breaks_preserve_distinct_display_names_during_deduplication() {
    use gpui_kit::{
        base::text::editor_host::{BlockNode, parse_markdown},
        component::text::MarkdownExtensions,
    };
    for tag in ["<br>", "<br/>", "<BR class=\"break\">"] {
        let first = format!("[before{tag}after](target.md)");
        let normal = parse_markdown(&first, MarkdownExtensions::default().into()).unwrap();
        let BlockNode::Paragraph(paragraph) = &normal[0] else {
            panic!("upstream must recognize the displayed link");
        };
        assert_eq!(paragraph.children()[0].text(), "before\nafter", "{tag}");
        let values = candidates(&format!("{first} [beforeafter](target.md) {first}"));
        assert_eq!(
            values
                .iter()
                .map(|value| value.display_name.as_str())
                .collect::<Vec<_>>(),
            ["before\nafter", "beforeafter"],
            "{tag}"
        );
        assert_eq!(values[0].target, values[1].target);
        assert_ne!(values[0].id, values[1].id);
        assert_eq!(
            values
                .iter()
                .map(|value| value.document_order)
                .collect::<Vec<_>>(),
            [0, 1]
        );
    }
}

#[test]
fn round2_reference_definitions_inside_footnotes_follow_upstream_recognition_and_first_definition()
{
    use gpui_kit::{
        base::text::editor_host::{BlockNode, parse_markdown},
        component::text::MarkdownExtensions,
    };
    for source in [
        "[a]: target.md\n\n[One][A]",
        "[^x]: [a]: target.md\n\n[One][A]",
        "[^x]: [a]: target.md\n\n[a]: wrong.md\n\n[One][A]",
    ] {
        let normal = parse_markdown(source, MarkdownExtensions::default().into()).unwrap();
        let BlockNode::Paragraph(paragraph) = normal.last().unwrap() else {
            panic!("upstream must recognize the reference link");
        };
        let run = &paragraph.children()[0];
        assert_eq!(run.text(), "One");
        assert!(
            run.mark_runs().iter().any(|(_, mark)| {
                mark.link
                    .as_ref()
                    .is_some_and(|link| link.identifier.as_deref() == Some("a"))
            }),
            "upstream must recognize the definition instead of leaving literal [One][A]"
        );
        let values = candidates(source);
        assert_eq!(values.len(), 1, "{source}");
        assert_eq!(values[0].display_name, "One");
        assert_eq!(
            values[0].target,
            LinkTarget::File {
                path: PathBuf::from(r"C:\fixtures\target.md"),
                fragment: None
            }
        );
    }
}

#[gpui_kit::test]
fn document_analysis_collects_offscreen_links_without_changing_normal_rendering(
    cx: &mut TestAppContext,
) {
    let source = "[Start](start.md)\n\n".to_owned()
        + &"長い本文です。\n\n".repeat(250)
        + "<div><a href=\"last.md#end\">Offscreen HTML</a></div>\n\n[![Preview](image.png)](preview.md)";
    let (window, view) = open(cx, source);
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        view.update(cx, |viewer, cx| {
            viewer.analyze_document(cx).unwrap();
            let analysis = viewer.document.analysis.as_ref().unwrap();
            assert_eq!(
                analysis
                    .candidates
                    .iter()
                    .map(|v| v.display_name.as_str())
                    .collect::<Vec<_>>(),
                ["Start", "Preview"]
            );
            let rendered = viewer.document.text.read(cx).rendered_text();
            assert!(rendered.as_str().contains("Start"));
            assert!(rendered.as_str().contains("Offscreen HTML"));
            assert!(!rendered.as_str().contains("mmv-link-candidate"));
            assert_eq!(
                viewer
                    .document
                    .text
                    .read(cx)
                    .list_state()
                    .logical_scroll_top()
                    .item_ix,
                0
            );
        });
    })
    .unwrap();
}
