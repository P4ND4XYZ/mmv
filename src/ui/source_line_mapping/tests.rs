use std::sync::Arc;

use gpui_kit::{
    AppContext as _, TestAppContext,
    base::text::{TextViewState, editor_host::parse_markdown},
    component::text::MarkdownExtensions,
};

use super::*;

fn fixture(cx: &mut TestAppContext, source: &str) -> (SourceLines, Vec<BlockNode>, RenderedText) {
    let blocks = parse_markdown(source, Arc::new(MarkdownExtensions::default())).unwrap();
    cx.update(gpui_kit::init);
    let text = cx.new(|cx| TextViewState::markdown(source, cx));
    cx.run_until_parked();
    let snapshot = text.read_with(cx, |text, _| text.rendered_text());
    assert_eq!(snapshot.source(), source);
    (SourceLines::new(source), blocks, snapshot)
}

fn mapped(fixture: &(SourceLines, Vec<BlockNode>, RenderedText), line: u64) -> MappedLineTarget {
    let result = map_source_line(
        &fixture.0,
        &fixture.1,
        &fixture.2,
        NonZeroU64::new(line).unwrap(),
    )
    .unwrap();
    assert_eq!(result.snapshot(), &fixture.2);
    result.target().clone()
}

#[gpui_kit::test]
fn indented_code_trimmed_trailing_blank_preserves_other_text_and_real_body(
    cx: &mut TestAppContext,
) {
    let fixture = self::fixture(cx, "before\n\n    a\n    \n\nafter");
    assert_eq!(fixture.2.as_str(), "before\na\nafter\n");
    assert_eq!(mapped(&fixture, 1), MappedLineTarget::RenderedPosition(0));
    assert_eq!(mapped(&fixture, 6), MappedLineTarget::RenderedPosition(9));
    for line in [3, 4] {
        assert_eq!(
            mapped(&fixture, line),
            MappedLineTarget::CodePosition {
                source_span: 8..18,
                offset: 0,
            }
        );
    }
    assert_eq!(mapped(&fixture, 5), MappedLineTarget::RenderedPosition(9));
}

#[gpui_kit::test]
fn default_math_code_fences_preserve_body_offsets_and_surrounding_text(cx: &mut TestAppContext) {
    let fixture = self::fixture(cx, "before\n\n$$\nx + y\n$$\n\nafter");
    assert_eq!(fixture.2.as_str(), "before\nx + y\nafter\n");
    assert_eq!(mapped(&fixture, 1), MappedLineTarget::RenderedPosition(0));
    assert_eq!(mapped(&fixture, 7), MappedLineTarget::RenderedPosition(13));
    for line in [3, 4, 5] {
        assert_eq!(
            mapped(&fixture, line),
            MappedLineTarget::CodePosition {
                source_span: 8..19,
                offset: 0,
            }
        );
    }
}

#[gpui_kit::test]
fn html_unmapped_subtree_is_local_and_still_checks_rendered_coherence(cx: &mut TestAppContext) {
    let fixture = self::fixture(cx, "before\n\n<div>hello</div>\n\nafter");
    assert_eq!(fixture.2.as_str(), "before\nhello\n\nafter\n");
    assert_eq!(mapped(&fixture, 1), MappedLineTarget::RenderedPosition(0));
    assert_eq!(mapped(&fixture, 5), MappedLineTarget::RenderedPosition(14));
    assert_eq!(
        map_source_line(
            &fixture.0,
            &fixture.1,
            &fixture.2,
            NonZeroU64::new(3).unwrap()
        )
        .unwrap_err(),
        SourceLineMapError::OpaqueMedia
    );
    let changed = parse_markdown("before\n\n<div>other</div>\n\nafter", Arc::default()).unwrap();
    assert_eq!(
        map_source_line(
            &fixture.0,
            &changed,
            &fixture.2,
            NonZeroU64::new(1).unwrap()
        )
        .unwrap_err(),
        SourceLineMapError::InvalidStructure
    );
}

#[gpui_kit::test]
fn repeated_text_is_projected_by_source_identity_not_string_search(cx: &mut TestAppContext) {
    let fixture = self::fixture(cx, "# same\n\nsame\n\n> - same");
    assert_eq!(mapped(&fixture, 1), MappedLineTarget::RenderedPosition(0));
    assert_eq!(mapped(&fixture, 3), MappedLineTarget::RenderedPosition(5));
    assert_eq!(mapped(&fixture, 5), MappedLineTarget::RenderedPosition(10));
}

#[gpui_kit::test]
fn unicode_crlf_and_inline_markup_select_the_first_body_character(cx: &mut TestAppContext) {
    let fixture = self::fixture(cx, "# **日本**\r\n\r\n> - [次](next.md)");
    assert_eq!(mapped(&fixture, 1), MappedLineTarget::RenderedPosition(0));
    assert_eq!(mapped(&fixture, 3), MappedLineTarget::RenderedPosition(7));
    assert_eq!(mapped(&fixture, 2), MappedLineTarget::RenderedPosition(7));
}

#[gpui_kit::test]
fn crlf_code_blank_lines_and_final_empty_line_keep_code_offsets(cx: &mut TestAppContext) {
    let fixture = self::fixture(cx, "```rust\r\n日本\r\n\r\nsame\r\n\r\n```");
    let BlockNode::CodeBlock(code) = &fixture.1[0] else {
        panic!("code block");
    };
    assert_eq!(code.code().as_ref(), "日本\r\n\r\nsame\r\n");
    // Offsets index code(), whose CRLF bytes are preserved by stable 0.9.
    for (line, offset) in [(1, 0), (2, 0), (3, 8), (4, 10), (5, 16), (6, 16)] {
        assert_eq!(
            mapped(&fixture, line),
            MappedLineTarget::CodePosition {
                source_span: 0..30,
                offset
            }
        );
    }
}

#[gpui_kit::test]
fn empty_code_is_an_actual_empty_region_not_an_invented_character(cx: &mut TestAppContext) {
    for source in ["```\n```", "```\n\n```", "```"] {
        let fixture = self::fixture(cx, source);
        for line in 1..=fixture.0.logical_count() as u64 {
            assert_eq!(
                mapped(&fixture, line),
                MappedLineTarget::EmptyCode {
                    source_span: 0..source.len()
                }
            );
        }
    }
}

#[gpui_kit::test]
fn indented_code_does_not_lose_its_first_body_line(cx: &mut TestAppContext) {
    let source = "    same\n\n    same";
    let fixture = self::fixture(cx, source);
    for (line, offset) in [(1, 0), (2, 5), (3, 6)] {
        assert_eq!(
            mapped(&fixture, line),
            MappedLineTarget::CodePosition {
                source_span: 0..source.len(),
                offset
            }
        );
    }
    let source = "    ```\n    x";
    let fixture = self::fixture(cx, source);
    assert_eq!(
        mapped(&fixture, 1),
        MappedLineTarget::CodePosition {
            source_span: 0..source.len(),
            offset: 0
        }
    );
    assert_eq!(
        mapped(&fixture, 2),
        MappedLineTarget::CodePosition {
            source_span: 0..source.len(),
            offset: 4
        }
    );
}

#[gpui_kit::test]
fn nested_code_uses_its_leaf_span_and_container_preserved_line_order(cx: &mut TestAppContext) {
    let source = "> - ```\n>   echo\n>   \n>   echo\n>   ```";
    let fixture = self::fixture(cx, source);
    for (line, offset) in [(1, 0), (2, 0), (3, 5), (4, 6), (5, 6)] {
        assert_eq!(
            mapped(&fixture, line),
            MappedLineTarget::CodePosition {
                source_span: 4..source.len(),
                offset
            }
        );
    }
}

#[gpui_kit::test]
fn unclosed_fence_does_not_invent_a_closing_line(cx: &mut TestAppContext) {
    let source = "~~~\nsame\nsame\n\n";
    let fixture = self::fixture(cx, source);
    for (line, offset) in [(1, 0), (2, 0), (3, 5), (4, 10)] {
        assert_eq!(
            mapped(&fixture, line),
            MappedLineTarget::CodePosition {
                source_span: 0..source.len(),
                offset
            }
        );
    }
    assert_eq!(mapped(&fixture, 5), MappedLineTarget::PastEnd);
}

#[gpui_kit::test]
fn an_indented_fence_shaped_last_body_line_is_not_a_closing_fence(cx: &mut TestAppContext) {
    let source = "```\nx\n    ```";
    let fixture = self::fixture(cx, source);
    assert_eq!(
        mapped(&fixture, 3),
        MappedLineTarget::CodePosition {
            source_span: 0..source.len(),
            offset: 2,
        }
    );
}

#[gpui_kit::test]
fn nearest_candidates_are_body_lines_not_fences_and_later_body_wins_ties(cx: &mut TestAppContext) {
    let fixture = self::fixture(cx, "a\n\n```\nb\n```");
    assert_eq!(mapped(&fixture, 2), MappedLineTarget::RenderedPosition(0));
    let source = "a\n\n\n```\nb\n```";
    let fixture = self::fixture(cx, source);
    assert_eq!(
        mapped(&fixture, 3),
        MappedLineTarget::CodePosition {
            source_span: 4..source.len(),
            offset: 0
        }
    );
}

#[gpui_kit::test]
fn table_first_text_cell_and_delimiter_are_not_separator_targets(cx: &mut TestAppContext) {
    let fixture = self::fixture(cx, "| | head |\n| --- | --- |\n| | row |\n\n```\nx\n```");
    assert_eq!(mapped(&fixture, 1), MappedLineTarget::RenderedPosition(1));
    assert_eq!(mapped(&fixture, 2), MappedLineTarget::RenderedPosition(7));
    assert_eq!(mapped(&fixture, 3), MappedLineTarget::RenderedPosition(7));
    assert!(matches!(
        mapped(&fixture, 6),
        MappedLineTarget::CodePosition { offset: 0, .. }
    ));
}

#[gpui_kit::test]
fn table_media_owns_only_its_row_not_the_structural_delimiter(cx: &mut TestAppContext) {
    let fixture = self::fixture(cx, "| head |\n| --- |\n| ![image](image.png) |");
    assert_eq!(mapped(&fixture, 2), MappedLineTarget::RenderedPosition(0));
    assert_eq!(
        map_source_line(
            &fixture.0,
            &fixture.1,
            &fixture.2,
            NonZeroU64::new(3).unwrap()
        )
        .unwrap_err(),
        SourceLineMapError::OpaqueMedia
    );
}

#[gpui_kit::test]
fn mixed_inline_media_does_not_hide_real_text_or_borrow_another_source_line(
    cx: &mut TestAppContext,
) {
    let fixture = self::fixture(cx, "before\n![image](image.png)\nafter");
    assert_eq!(mapped(&fixture, 1), MappedLineTarget::RenderedPosition(0));
    assert_eq!(
        map_source_line(
            &fixture.0,
            &fixture.1,
            &fixture.2,
            NonZeroU64::new(2).unwrap()
        )
        .unwrap_err(),
        SourceLineMapError::OpaqueMedia
    );
    // Stable Markdown soft breaks become spaces, without changing byte identity.
    assert_eq!(fixture.2.as_str(), "before  after\n");
    assert_eq!(mapped(&fixture, 3), MappedLineTarget::RenderedPosition(8));
    let fixture = self::fixture(cx, "![image](image.png) **after**");
    assert_eq!(mapped(&fixture, 1), MappedLineTarget::RenderedPosition(1));
}

#[gpui_kit::test]
fn opaque_owned_lines_fail_without_jumping_to_another_block(cx: &mut TestAppContext) {
    let fixture = self::fixture(cx, "before\n\n![image](image.png)\n\nafter");
    assert_eq!(
        map_source_line(
            &fixture.0,
            &fixture.1,
            &fixture.2,
            NonZeroU64::new(3).unwrap()
        )
        .unwrap_err(),
        SourceLineMapError::OpaqueMedia
    );
}

#[gpui_kit::test]
fn no_content_and_past_end_are_distinct_from_mapping_failure(cx: &mut TestAppContext) {
    for source in ["", " \r\n\t\n", "\u{3000}"] {
        let fixture = self::fixture(cx, source);
        assert_eq!(mapped(&fixture, 1), MappedLineTarget::Noop);
        assert_eq!(mapped(&fixture, u64::MAX), MappedLineTarget::PastEnd);
    }
    let fixture = self::fixture(cx, "a\n");
    assert_eq!(mapped(&fixture, 2), MappedLineTarget::PastEnd);
    let fixture = self::fixture(cx, "---");
    assert_eq!(
        map_source_line(
            &fixture.0,
            &fixture.1,
            &fixture.2,
            NonZeroU64::new(1).unwrap()
        )
        .unwrap_err(),
        SourceLineMapError::UnmappedText
    );
}

#[gpui_kit::test]
fn stale_source_and_incoherent_ast_are_rejected(cx: &mut TestAppContext) {
    let fixture = self::fixture(cx, "two");
    assert_eq!(
        map_source_line(
            &SourceLines::new("one"),
            &fixture.1,
            &fixture.2,
            NonZeroU64::new(1).unwrap()
        )
        .unwrap_err(),
        SourceLineMapError::StaleSource
    );
    let old_blocks = parse_markdown("one", Arc::default()).unwrap();
    assert_eq!(
        map_source_line(
            &fixture.0,
            &old_blocks,
            &fixture.2,
            NonZeroU64::new(1).unwrap()
        )
        .unwrap_err(),
        SourceLineMapError::InvalidStructure
    );
}

#[gpui_kit::test]
fn replacement_snapshot_rejects_the_old_index_and_preserves_old_target_identity(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let text = cx.new(|cx| TextViewState::markdown("old", cx));
    cx.run_until_parked();
    let old = text.read_with(cx, |text, _| text.rendered_text());
    let lines = SourceLines::new("old");
    let blocks = parse_markdown("old", Arc::default()).unwrap();
    let mapped = map_source_line(&lines, &blocks, &old, NonZeroU64::new(1).unwrap()).unwrap();
    text.update(cx, |text, cx| text.set_text("new", cx));
    cx.run_until_parked();
    let new = text.read_with(cx, |text, _| text.rendered_text());
    assert_ne!(old, new);
    assert_eq!(mapped.snapshot(), &old);
    assert_eq!(
        map_source_line(&lines, &blocks, &new, NonZeroU64::new(1).unwrap()).unwrap_err(),
        SourceLineMapError::StaleSource
    );
}

#[gpui_kit::test]
fn long_repeated_code_after_a_table_selects_the_exact_code_line(cx: &mut TestAppContext) {
    let prefix = "| head |\n| --- |\n| body |\n\n```lua\n";
    let source = format!("{prefix}{}```", "x\n".repeat(1000));
    let fixture = self::fixture(cx, &source);
    assert_eq!(
        mapped(&fixture, 1005),
        MappedLineTarget::CodePosition {
            source_span: 27..source.len(),
            offset: 1998,
        }
    );
}
