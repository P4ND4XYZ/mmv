//! Isolated feasibility probes; these are not production position contracts.
use gpui_kit::{
    AnyWindowHandle, Bounds, Context, Entity, IntoElement, ListOffset, Render, TestAppContext,
    Window, WindowBounds, WindowOptions,
    base::text::editor_host::{BlockNode, parse_markdown},
    component::text::{MarkdownExtensions, TextView, TextViewState},
    point,
    prelude::*,
    px, size,
    test::TestWindowExt as _,
};

struct Probe {
    text: Entity<TextViewState>,
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        TextView::new(&self.text).scrollable(true).size_full()
    }
}

fn open(cx: &mut TestAppContext, source: &str) -> (AnyWindowHandle, Entity<Probe>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(640.), px(360.)),
                ))),
                ..Default::default()
            },
            cx,
            |_, cx| {
                cx.new(|cx| Probe {
                    text: cx.new(|cx| TextViewState::markdown(source, cx)),
                })
            },
        )
        .unwrap()
    })
}

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

fn fixture() -> String {
    // Same 20x10 PNG as the existing image decoding test; no filesystem/network I/O.
    let image = "![pixel](data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABQAAAAKCAYAAAC0VX7mAAAAAXNSR0IArs4c6QAAAARnQU1BAACxjwv8YQUAAAAJcEhZcwAADsMAAA7DAcdvqGQAAAAQSURBVDhPY2AYBaNgFBAGAAMqAAGUVyP4AAAAAElFTkSuQmCC)\n\n";
    "# Start\n\n".to_owned()
        + &"A long wrapped paragraph with 日本語 text. ".repeat(200)
        + "\n\n| Column | Value |\n| --- | --- |\n"
        + &"| table cell | wrapped table contents 日本語 |\n".repeat(30)
        + "\n"
        + image
        + &"Offscreen paragraph.\n\n".repeat(100)
        + "## **日本語** target\n\n"
        + &"Trailing paragraph.\n\n".repeat(30)
}

#[gpui_kit::test]
fn phase_one_pixel_distance_survives_replacement_and_clamps(cx: &mut TestAppContext) {
    let source = fixture();
    let (window, probe) = open(cx, &source);
    frame(cx, window);
    let text = probe.read_with(cx, |probe, _| probe.text.clone());
    // TextView's measure_all pass must include offscreen blocks, not just the viewport.
    text.read_with(cx, |text, _| {
        let list = text.list_state();
        assert!(list.item_count() > 100);
        assert!(list.is_scrolled_to_end().is_some());
        assert!(list.max_offset_for_scrollbar().y > px(3000.));
    });
    cx.update(|cx| {
        text.update(cx, |text, cx| {
            text.list_state()
                .set_offset_from_scrollbar(point(px(0.), px(-2500.)));
            cx.notify();
        })
    });
    frame(cx, window);
    let saved = text.read_with(cx, |text, _| {
        text.list_state().scroll_px_offset_for_scrollbar()
    });
    assert_eq!(saved.y, px(-2500.));
    let original_block =
        text.read_with(cx, |text, _| text.list_state().logical_scroll_top().item_ix);

    // Change block boundaries and all preceding heights. Reusing a logical block
    // index would not preserve this distance in the replacement document.
    cx.update(|cx| {
        probe.update(cx, |probe, cx| {
            probe.text =
                cx.new(|cx| TextViewState::markdown(&"Replacement paragraph.\n\n".repeat(300), cx));
            cx.notify();
        })
    });
    frame(cx, window);
    let replacement = probe.read_with(cx, |probe, _| probe.text.clone());
    cx.update(|cx| {
        replacement.update(cx, |text, cx| {
            text.list_state().set_offset_from_scrollbar(saved);
            cx.notify();
        })
    });
    frame(cx, window);
    replacement.read_with(cx, |text, _| {
        assert_eq!(text.list_state().scroll_px_offset_for_scrollbar(), saved);
        assert_ne!(
            text.list_state().logical_scroll_top().item_ix,
            original_block
        );
    });
    cx.update(|cx| {
        replacement.update(cx, |text, cx| {
            text.set_text(&"Short paragraph.\n\n".repeat(20), cx);
        })
    });
    frame(cx, window);
    cx.update(|cx| {
        replacement.update(cx, |text, cx| {
            text.list_state().set_offset_from_scrollbar(saved);
            cx.notify();
        })
    });
    frame(cx, window);
    replacement.read_with(cx, |text, _| {
        let list = text.list_state();
        let max = list.max_offset_for_scrollbar().y;
        assert!(max > px(0.) && max < px(2500.));
        assert_eq!(list.scroll_px_offset_for_scrollbar().y, -max);
        assert_eq!(list.is_scrolled_to_end(), Some(true));
    });
}

#[gpui_kit::test]
fn phase_one_upstream_heading_maps_to_an_offscreen_display_block(cx: &mut TestAppContext) {
    let source = fixture();
    let blocks = parse_markdown(&source, MarkdownExtensions::default().into()).unwrap();
    let heading_ix = blocks
        .iter()
        .position(|block| matches!(block, BlockNode::Heading { level: 2, .. }))
        .unwrap();
    let span = blocks[heading_ix].span().unwrap();
    assert!(source[span.start..span.end].contains("## **日本語** target"));
    let (window, probe) = open(cx, &source);
    frame(cx, window);
    let text = probe.read_with(cx, |probe, _| probe.text.clone());
    let heading_distance = text.read_with(cx, |text, _| {
        let list = text.list_state();
        assert_eq!(list.item_count(), blocks.len());
        assert!(list.bounds_for_item(1).unwrap().size.height > px(360.));
        assert!(list.bounds_for_item(2).unwrap().size.height > px(360.));
        assert!(list.bounds_for_item(3).unwrap().size.height >= px(10.));
        let rendered = text.rendered_text();
        let range = rendered.range_for_source(span.start..span.end).unwrap();
        assert_eq!(rendered.as_str()[range].trim(), "日本語 target");
        // All preceding block heights must be measured even though only the
        // first blocks have painted. Do not substitute a uniform height hint.
        (0..heading_ix)
            .map(|ix| list.bounds_for_item(ix).unwrap().size.height)
            .sum::<gpui_kit::Pixels>()
    });
    cx.update(|cx| {
        text.update(cx, |text, cx| {
            text.list_state().scroll_to(ListOffset {
                item_ix: heading_ix,
                offset_in_item: px(0.),
            });
            cx.notify();
        })
    });
    frame(cx, window);
    text.read_with(cx, |text, _| {
        let list = text.list_state();
        assert_eq!(list.logical_scroll_top().item_ix, heading_ix);
        let bounds = list.bounds_for_item(heading_ix).unwrap();
        assert_eq!(bounds.top(), list.viewport_bounds().top());
        assert!(bounds.size.height > px(0.));
        assert!(heading_distance > px(3000.));
        assert_eq!(list.scroll_px_offset_for_scrollbar().y, -heading_distance);
    });
}
