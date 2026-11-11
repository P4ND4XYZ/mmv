use super::Viewer;
use crate::app::{
    DocumentOperation, OperationOrigin, OperationResult, PositionTarget, ReadRequest,
};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, TestAppContext, point, px, test::TestWindowExt as _,
};
use std::{cell::RefCell, rc::Rc};

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

fn fixture() -> String {
    "[移動](#%E4%BD%BF%E3%81%84%E6%96%B9-2)\n\n".to_owned()
        + &"A wrapped paragraph with 日本語 text. ".repeat(160)
        + "\n\n# **使い方**\n\n"
        + &"Paragraph.\n\n".repeat(20)
        + "## 使い方-1\n\n"
        + &"Paragraph.\n\n".repeat(20)
        + "### [使い方](https://example.com)\n\n"
        + &"Paragraph.\n\n".repeat(20)
        + "## *Getting* `Started`!\n\n"
        + &"Trailing paragraph.\n\n".repeat(40)
}

fn link(cx: &mut TestAppContext, window: AnyWindowHandle, view: &Entity<Viewer>, url: &str) {
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |viewer, cx| viewer.open_link(url, window, cx));
    })
    .unwrap();
}

fn scroll(cx: &mut TestAppContext, view: &Entity<Viewer>, distance: f32) {
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.document.text.update(cx, |text, cx| {
                text.list_state()
                    .set_offset_from_scrollbar(point(px(0.), px(-distance)));
                cx.notify();
            });
        })
    });
}

fn assert_heading(cx: &mut TestAppContext, view: &Entity<Viewer>, label: &str) -> f32 {
    cx.update(|cx| view.update(cx, |viewer, cx| viewer.analyze_document(cx).unwrap()));
    view.read_with(cx, |viewer, cx| {
        let text = viewer.document.text.read(cx);
        let list = text.list_state();
        let ix = list.logical_scroll_top().item_ix;
        let span = viewer.document.analysis.as_ref().unwrap().blocks[ix]
            .clone()
            .unwrap();
        let snapshot = text.rendered_text();
        let range = snapshot.range_for_source(span).unwrap();
        assert_eq!(snapshot.as_str()[range].trim(), label);
        assert_eq!(
            list.bounds_for_item(ix).unwrap().top(),
            list.viewport_bounds().top()
        );
        assert!(viewer.app.position_request().is_none());
        viewer.document.distance(cx)
    })
}

#[gpui_kit::test]
fn clicked_japanese_fragment_uses_rendered_decoration_and_document_wide_ids_once(
    cx: &mut TestAppContext,
) {
    let (window, view) = super::open(cx, fixture());
    frame(cx, window);
    let generation = view.read_with(cx, |viewer, _| viewer.app.document().unwrap().generation);
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    let bounds = visual.debug_bounds("document-body").unwrap();
    visual.simulate_click(bounds.origin + point(px(20.), px(20.)), Default::default());
    frame(cx, window);
    let distance = assert_heading(cx, &view, "使い方");
    assert!(distance > 1500.);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.app.document().unwrap().generation, generation);
        assert_eq!(
            viewer
                .document
                .text
                .read(cx)
                .list_state()
                .logical_scroll_top()
                .item_ix,
            44
        );
        assert!(viewer.message.is_none());
    });
    scroll(cx, &view, 240.);
    frame(cx, window);
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 240.)
    });
    link(cx, window, &view, "#getting-started");
    frame(cx, window);
    assert_heading(cx, &view, "Getting Started!");
    link(cx, window, &view, "#使い方-1");
    frame(cx, window);
    assert_heading(cx, &view, "使い方-1");
}

#[gpui_kit::test]
fn other_document_fragment_is_decoded_and_missing_headings_go_to_top(cx: &mut TestAppContext) {
    let (window, view) = super::open(cx, fixture());
    frame(cx, window);
    let requests = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = requests.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone())
        })
    });
    link(
        cx,
        window,
        &view,
        "notes/日本語%20memo.md#%E4%BD%BF%E3%81%84%E6%96%B9-2",
    );
    let request = requests.borrow_mut().pop().unwrap();
    assert_eq!(request.origin, OperationOrigin::Link);
    assert_eq!(request.path.file_name().unwrap(), "日本語 memo.md");
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.complete(super::success(request, fixture()), cx);
            assert_eq!(
                viewer.app.position_request().unwrap().target,
                PositionTarget::Heading("使い方-2".into())
            );
        })
    });
    frame(cx, window);
    assert_heading(cx, &view, "使い方");
    for url in ["#不明", "next.md#%E4%B8%8D%E6%98%8E"] {
        link(cx, window, &view, url);
        if let Some(request) = requests.borrow_mut().pop() {
            cx.update(|cx| {
                view.update(cx, |viewer, cx| {
                    viewer.complete(super::success(request, fixture()), cx)
                })
            });
        }
        frame(cx, window);
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.document.distance(cx), 0.);
            assert!(viewer.app.position_request().is_none());
            assert_eq!(viewer.message.as_deref(), Some("Heading not found: 不明"));
        });
    }
}

#[gpui_kit::test]
fn consecutive_links_record_applied_heading_distance_for_history(cx: &mut TestAppContext) {
    let (window, view) = super::open(cx, fixture());
    frame(cx, window);
    // No frame between the two links: the second must not save the original offset.
    link(cx, window, &view, "#使い方-2");
    link(cx, window, &view, "#getting-started");
    frame(cx, window);
    let destination = assert_heading(cx, &view, "Getting Started!");
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let OperationResult::Pending(request) = viewer.request_operation(
                DocumentOperation::Back,
                OperationOrigin::External,
                None,
                cx,
            ) else {
                panic!("history read")
            };
            viewer.complete(super::success(request, fixture()), cx);
        })
    });
    frame(cx, window);
    assert!(assert_heading(cx, &view, "使い方") < destination);
}

#[gpui_kit::test]
fn heading_before_first_layout_survives_reload_and_queued_link(cx: &mut TestAppContext) {
    let (window, view) = super::open(cx, fixture());
    frame(cx, window);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let OperationResult::Pending(request) = viewer.request_operation(
                DocumentOperation::Heading {
                    path: Some(viewer.path.with_file_name("other.md")),
                    id: "使い方-2".into(),
                },
                OperationOrigin::Link,
                None,
                cx,
            ) else {
                panic!("heading read")
            };
            viewer.complete(super::success(request, fixture()), cx);
            let OperationResult::Pending(reload) = viewer.request_operation(
                DocumentOperation::Reload,
                OperationOrigin::Automatic,
                None,
                cx,
            ) else {
                panic!("reload")
            };
            viewer.complete(super::success(reload, fixture()), cx);
            assert_eq!(
                viewer.app.position_request().unwrap().target,
                PositionTarget::Heading("使い方-2".into())
            );
        })
    });
    link(cx, window, &view, "#getting-started");
    frame(cx, window);
    frame(cx, window);
    assert_heading(cx, &view, "Getting Started!");
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let OperationResult::Pending(request) = viewer.request_operation(
                DocumentOperation::Back,
                OperationOrigin::External,
                None,
                cx,
            ) else {
                panic!("history")
            };
            viewer.complete(super::success(request, fixture()), cx);
        })
    });
    frame(cx, window);
    assert_heading(cx, &view, "使い方");
}

#[gpui_kit::test]
fn nested_heading_uses_the_projected_line_and_shares_document_ids(cx: &mut TestAppContext) {
    let source = "# 使い方\n\n".to_owned()
        + &"> Wrapped quoted paragraph.\n>\n".repeat(40)
        + "> ## **使い方**\n>\n"
        + &"> Quoted tail.\n>\n".repeat(40)
        + "\n# End\n\n"
        + &"Trailing paragraph.\n\n".repeat(30);
    let (window, view) = super::open(cx, source.clone());
    frame(cx, window);
    link(cx, window, &view, "#使い方-1");
    frame(cx, window);
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        let list = viewer.document.text.read(cx).list_state();
        assert_eq!(list.logical_scroll_top().item_ix, 1);
        assert!(list.logical_scroll_top().offset_in_item > px(600.));
        assert!(viewer.app.position_request().is_none());
        assert!(viewer.message.is_none());
    });
    scroll(cx, &view, 240.);
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 240.)
    });
    link(cx, window, &view, "#使い方-1");
    link(cx, window, &view, "#end");
    frame(cx, window);
    frame(cx, window);
    frame(cx, window);
    assert_heading(cx, &view, "End");
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let OperationResult::Pending(request) = viewer.request_operation(
                DocumentOperation::Back,
                OperationOrigin::External,
                None,
                cx,
            ) else {
                panic!("history")
            };
            viewer.complete(super::success(request, source), cx);
        })
    });
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        let list = viewer.document.text.read(cx).list_state();
        assert_eq!(list.logical_scroll_top().item_ix, 1);
        assert!(list.logical_scroll_top().offset_in_item > px(600.));
    });
}

fn external_open_supersedes_a_queued_link(cx: &mut TestAppContext, url: &str) {
    let (window, view) = super::open(cx, fixture());
    frame(cx, window);
    let departure = cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let path = viewer.path.with_file_name("heading-a.md");
            let OperationResult::Pending(request) = viewer.request_operation(
                DocumentOperation::Heading {
                    path: Some(path.clone()),
                    id: "使い方-2".into(),
                },
                OperationOrigin::Link,
                None,
                cx,
            ) else {
                panic!("heading read")
            };
            viewer.complete(super::success(request, fixture()), cx);
            path
        })
    });
    // A has no layout yet. Both fragment-only and file links must wait.
    link(cx, window, &view, url);
    let newest = cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            assert_eq!(viewer.deferred_links.len(), 1);
            let path = viewer.path.with_file_name("external-b.md");
            let OperationResult::Pending(request) = viewer.request_document(path.clone(), None, cx)
            else {
                panic!("external read")
            };
            viewer.complete(super::success(request, "Newest document B".into()), cx);
            path
        })
    });
    let requests = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = requests.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone());
        })
    });
    frame(cx, window);
    frame(cx, window);
    assert!(
        requests.borrow().is_empty(),
        "obsolete file link emitted a read"
    );
    view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.app.document().unwrap().path, newest);
        assert_eq!(viewer.app.document().unwrap().source, "Newest document B");
        assert!(!viewer.app.is_loading());
        assert!(viewer.app.position_request().is_none());
        assert!(
            viewer.message.is_none(),
            "obsolete fragment was reinterpreted in B"
        );
    });
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let OperationResult::Pending(request) = viewer.request_operation(
                DocumentOperation::Back,
                OperationOrigin::External,
                None,
                cx,
            ) else {
                panic!("history read")
            };
            assert_eq!(
                request.path, departure,
                "obsolete link added a history entry"
            );
            viewer.complete(super::success(request, fixture()), cx);
        })
    });
    frame(cx, window);
}

#[gpui_kit::test]
fn external_open_supersedes_queued_same_document_fragment(cx: &mut TestAppContext) {
    external_open_supersedes_a_queued_link(cx, "#old");
}

#[gpui_kit::test]
fn external_open_supersedes_queued_relative_file(cx: &mut TestAppContext) {
    external_open_supersedes_a_queued_link(cx, "notes/old.md#old");
}

#[gpui_kit::test]
fn failed_and_stale_heading_loads_keep_the_displayed_text_and_distance(cx: &mut TestAppContext) {
    let (window, view) = super::open(cx, fixture());
    frame(cx, window);
    scroll(cx, &view, 480.);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let text = viewer.document.text.clone();
            let request = |viewer: &mut Viewer, path: &str, cx: &mut gpui_kit::Context<Viewer>| {
                let OperationResult::Pending(request) = viewer.request_operation(
                    DocumentOperation::Heading {
                        path: Some(viewer.path.with_file_name(path)),
                        id: "使い方".into(),
                    },
                    OperationOrigin::Link,
                    None,
                    cx,
                ) else {
                    panic!("heading read")
                };
                request
            };
            let old = request(viewer, "old.md", cx);
            let failed = request(viewer, "missing.md", cx);
            viewer.complete(
                crate::app::ReadResult {
                    request: failed,
                    content: Err("missing".into()),
                },
                cx,
            );
            viewer.complete(super::success(old, "stale body".into()), cx);
            assert_eq!(viewer.document.text, text);
            assert_eq!(viewer.app.error(), Some("missing"));
            assert!(viewer.app.position_request().is_none());
        })
    });
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 480.);
        assert_eq!(viewer.app.document().unwrap().source, fixture());
    });
}
