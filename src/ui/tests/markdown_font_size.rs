//! Real Lua -> host queue -> TextView layout, on GPUI's headless platform.
use super::*;
use crate::lua::markdown::Request;
use gpui_kit::{ListOffset, px};

#[gpui_kit::test]
fn font_size_changes_layout_preserves_selection_and_survives_replacement(cx: &mut TestAppContext) {
    let (window, view) = open(cx, "# Heading\n\nBody with `inline code` and words.\n\n| Column | Other |\n|---|---|\n| Cell | Cell |\n\n```rust\nlet value = 42;\n```\n".into());
    pdf::frame(cx, window);
    let before = view.update(cx, |v, cx| {
        let text = v.document.as_markdown().unwrap().text.clone();
        text.update(cx, |text, cx| text.select_all(cx));
        let heights = (0..text.read(cx).list_state().item_count())
            .map(|ix| {
                text.read(cx)
                    .list_state()
                    .bounds_for_item(ix)
                    .unwrap()
                    .size
                    .height
            })
            .collect::<Vec<_>>();
        (text.read(cx).selected_text(), heights)
    });
    view.update(cx, |v,cx| {
        v.lua.lua().load("local m=require('mmv').markdown; assert(m.get_font_size()==16); assert(m.set_font_size(32)); assert(m.get_font_size()==16)").exec().unwrap();
        v.pump_lua(cx);
        assert_eq!(v.markdown_font_size,32.0);
    });
    pdf::frame(cx, window);
    view.read_with(cx, |v, cx| {
        let text = v.document.as_markdown().unwrap().text.read(cx);
        assert_eq!(text.selected_text(), before.0);
        for (ix, height) in before.1.iter().enumerate() {
            assert!(
                text.list_state().bounds_for_item(ix).unwrap().size.height > *height,
                "node {ix}"
            );
        }
        v.lua
            .lua()
            .load("assert(require('mmv').markdown.get_font_size()==32)")
            .exec()
            .unwrap();
    });
    view.update(cx, |v, cx| {
        let request = v.app.open(PathBuf::from(r"C:\fixtures\second.md"));
        v.complete_fixture(success(request, "replacement".into()), cx);
        assert_eq!(v.markdown_font_size, 32.0);
        v.apply_markdown_font_size(Request::Reset, cx);
        assert_eq!(v.markdown_font_size, 16.0);
    });
    pdf::frame(cx, window);
}

#[gpui_kit::test]
fn font_reflow_retains_scroll_state_and_clamps_after_shrink(cx: &mut TestAppContext) {
    let source = (0..70)
        .map(|n| {
            format!(
                "Paragraph {n}: {}\n\n",
                "words and more readable text ".repeat(15)
            )
        })
        .collect::<String>();
    let (window, view) = open(cx, source);
    pdf::frame(cx, window);
    view.update(cx, |v, cx| {
        let text = v.document.as_markdown().unwrap().text.clone();
        text.read(cx).list_state().scroll_to(ListOffset {
            item_ix: 20,
            offset_in_item: px(80.),
        });
        text.update(cx, |_, cx| cx.notify());
    });
    pdf::frame(cx, window);
    let before = view.read_with(cx, |v, cx| {
        v.document
            .as_markdown()
            .unwrap()
            .text
            .read(cx)
            .list_state()
            .logical_scroll_top()
    });
    view.update(cx, |v, cx| {
        v.apply_markdown_font_size(Request::Set(24.), cx);
        v.apply_markdown_font_size(Request::Adjust(4.), cx);
    });
    pdf::frame(cx, window);
    view.read_with(cx, |v, cx| {
        let list = v.document.as_markdown().unwrap().text.read(cx).list_state();
        assert_eq!(list.logical_scroll_top(), before);
        assert_eq!(v.markdown_font_size, 28.);
    });
    view.update(cx, |v, cx| v.document.scroll_edge(true, cx));
    pdf::frame(cx, window);
    view.update(cx, |v, cx| v.apply_markdown_font_size(Request::Set(8.), cx));
    pdf::frame(cx, window);
    view.read_with(cx, |v, cx| {
        let list = v.document.as_markdown().unwrap().text.read(cx).list_state();
        assert!(
            list.logical_scroll_top().item_ix > 0,
            "must not return to top"
        );
        assert!(
            list.is_scrolled_to_end().unwrap_or(false),
            "shrink clamps to valid end"
        );
    });
}

#[gpui_kit::test]
fn font_changes_preserve_search_and_newer_navigation_wins(cx: &mut TestAppContext) {
    let source = format!(
        "# Start\n\n{}\n\n# Destination\n\n{}",
        "needle paragraph words\n\n".repeat(100),
        "tail\n\n".repeat(40)
    );
    let (window, view) = open(cx, source);
    pdf::frame(cx, window);
    pdf::press(cx, window, "/");
    pdf::input(cx, window, "needle");
    pdf::press(cx, window, "enter");
    let before = view.read_with(cx, |v, _| (v.app.matches().to_vec(), v.app.current_match()));
    view.update(cx, |v, cx| {
        v.apply_markdown_font_size(Request::Set(48.), cx);
        v.request_operation(
            crate::app::DocumentOperation::Heading {
                path: None,
                id: "destination".into(),
            },
            crate::app::OperationOrigin::Link,
            None,
            cx,
        );
    });
    pdf::frame(cx, window);
    view.read_with(cx, |v, cx| {
        assert_eq!(v.app.matches(), before.0.as_slice());
        assert_eq!(v.app.current_match(), before.1);
        assert!(
            v.document
                .as_markdown()
                .unwrap()
                .text
                .read(cx)
                .list_state()
                .logical_scroll_top()
                .item_ix
                >= 100,
            "newer heading navigation wins"
        );
    });
    view.update(cx, |v, cx| {
        v.document.scroll_edge(true, cx);
    });
    pdf::frame(cx, window);
    view.update(cx, |v, cx| {
        v.apply_markdown_font_size(Request::Set(8.), cx);
        v.apply_markdown_font_size(Request::Adjust(1.), cx);
        v.apply_markdown_font_size(Request::Adjust(-1.), cx);
    });
    pdf::frame(cx, window);
    view.read_with(cx, |v, cx| {
        let list = v.document.as_markdown().unwrap().text.read(cx).list_state();
        assert!(list.logical_scroll_top().item_ix > 100);
        assert!(v.document.distance(cx).is_finite());
        assert_eq!(v.markdown_font_size, 8.);
    });
}

#[gpui_kit::test]
fn font_size_survives_real_pdf_roundtrip_and_history(cx: &mut TestAppContext) {
    let files = pdf::Files::new();
    let path = files.pdf("font-roundtrip.pdf");
    let markdown = files.write("font-return.md", b"# Return\n\nbody");
    let (window, view) = open(cx, "initial".into());
    pdf::frame(cx, window);
    let _registry = cx.update(|cx| crate::runtime::connect_document_reads(&view, cx));
    view.update(cx, |v, cx| {
        v.apply_markdown_font_size(Request::Set(23.5), cx)
    });
    pdf::request(cx, &view, &markdown, "font-markdown");
    pdf::frame(cx, window);
    files.write("font-return.md", b"# Reloaded\n\nupdated body");
    pdf::press(cx, window, "f5");
    view.read_with(cx, |v, cx| {
        assert_eq!(v.markdown_font_size, 23.5);
        assert!(
            v.document
                .as_markdown()
                .unwrap()
                .text
                .read(cx)
                .rendered_text()
                .source()
                .contains("Reloaded")
        );
    });
    pdf::request(cx, &view, &path, "font-pdf");
    pdf::frame(cx, window);
    view.read_with(cx,|v,_| {
        assert!(v.document.as_markdown().is_none());
        v.lua.lua().load("local m=require('mmv').markdown; assert(m.get_font_size()==nil); assert(m.set_font_size(40)==false)").exec().unwrap();
    });
    view.update(cx, |v, cx| {
        v.request_operation(
            crate::app::DocumentOperation::Back,
            crate::app::OperationOrigin::External,
            None,
            cx,
        );
    });
    pdf::frame(cx, window);
    view.read_with(cx, |v, _| {
        assert!(v.document.as_markdown().is_some());
        assert_eq!(v.markdown_font_size, 23.5);
        v.lua
            .lua()
            .load("assert(require('mmv').markdown.get_font_size()==23.5)")
            .exec()
            .unwrap();
    });
}

#[gpui_kit::test]
fn queued_font_requests_survive_actual_markdown_and_pdf_replacement(cx: &mut TestAppContext) {
    let files = pdf::Files::new();
    let markdown = files.write("queued.md", b"replacement body");
    let pdf_path = files.pdf("queued.pdf");
    let (window, view) = open(cx, "initial".into());
    pdf::frame(cx, window);
    let reads = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = reads.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone())
        })
    });
    for (path, script, expected) in [
        (
            &markdown,
            "local m=require('mmv').markdown; assert(m.set_font_size(20)); assert(m.adjust_font_size(2)); assert(m.get_font_size()==16)",
            22.,
        ),
        (
            &pdf_path,
            "local m=require('mmv').markdown; assert(m.set_font_size(24)); assert(m.adjust_font_size(3)); assert(m.get_font_size()==22)",
            27.,
        ),
    ] {
        let resolve = pdf::request(cx, &view, path, "queued");
        view.update(cx, |v, cx| {
            v.complete(crate::runtime::test_read(resolve), cx)
        });
        let load = reads.borrow().last().unwrap().clone();
        let prepared = crate::runtime::test_read(load);
        view.update(cx, |v, cx| {
            v.lua.lua().load(script).exec().unwrap();
            // Adoption publishes the replacement before draining the accepted requests.
            v.complete(prepared, cx);
            assert_eq!(v.markdown_font_size, expected);
        });
        pdf::frame(cx, window);
    }
    view.read_with(cx, |v, _| {
        assert!(v.document.as_markdown().is_none());
        v.lua
            .lua()
            .load("assert(require('mmv').markdown.get_font_size()==nil)")
            .exec()
            .unwrap();
    });
}
