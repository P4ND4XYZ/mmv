use super::selection_panel::{capture_reads, frame, input, is_open, panel, press, query, selected};
use super::*;
use crate::app::{
    DocumentOperation, ExternalOperation, Mode, OperationError, OperationOrigin, OperationResult,
    ScrollDirection,
};
use gpui_kit::EntityInputHandler as _;

const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 20, 0, 0, 0, 10, 8, 6,
    0, 0, 0, 180, 85, 126, 230, 0, 0, 0, 1, 115, 82, 71, 66, 0, 174, 206, 28, 233, 0, 0, 0, 4, 103,
    65, 77, 65, 0, 0, 177, 143, 11, 252, 97, 5, 0, 0, 0, 9, 112, 72, 89, 115, 0, 0, 14, 195, 0, 0,
    14, 195, 1, 199, 111, 168, 100, 0, 0, 0, 16, 73, 68, 65, 84, 56, 79, 99, 96, 24, 5, 163, 96,
    20, 16, 6, 0, 3, 42, 0, 1, 148, 87, 35, 248, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

fn pending(result: OperationResult) -> ReadRequest {
    let OperationResult::Pending(request) = result else {
        panic!("expected pending read")
    };
    request
}

#[gpui_kit::test]
fn selection_panel_read_request_failure_and_stale_success_preserve_query_selection_and_generation(
    cx: &mut TestAppContext,
) {
    let (window, view) = open(cx, "[Design](design.md)\n\n[Design extra](extra.md)".into());
    frame(cx, window);
    press(cx, window, "f");
    input(cx, window, "dsg");
    press(cx, window, "down");
    let owner = panel(cx, &view).owner;
    let selection = selected(cx, &view);
    let q = query(cx, &view);
    let generation = view.read_with(cx, |v, _| v.app.document().unwrap().generation);
    view.update(cx, |v, cx| {
        let old = pending(v.request_document(v.path.with_file_name("old.md"), None, cx));
        let latest = pending(v.request_document(v.path.with_file_name("missing.md"), None, cx));
        assert_eq!(v.lua.context().ui.borrow().windows[0].owner, owner);
        v.complete_fixture(
            ReadResult {

                request: latest,
                content: Err("not found".into()),
            },
            cx,
        );
        v.complete_fixture(success(old, "stale body".into()), cx);

    });
    frame(cx, window);
    assert_eq!(panel(cx, &view).owner, owner);
    assert_eq!(selected(cx, &view), selection);
    assert_eq!(query(cx, &view), q);
    assert_eq!(q.read_with(cx, |q, _| q.value().to_string()), "dsg");
    view.read_with(cx, |v, _| {
        assert_eq!(v.app.document().unwrap().generation, generation);
        assert_eq!(v.app.error(), Some("not found"));
    });
}

#[gpui_kit::test]
fn selection_panel_only_adopted_body_closes_panel_warns_and_old_ids_cannot_execute(
    cx: &mut TestAppContext,
) {
    for origin in [
        OperationOrigin::External,
        OperationOrigin::Automatic,
        OperationOrigin::Keyboard,
    ] {
        let (window, view) = open(cx, "[Old](old.md)".into());
        frame(cx, window);
        press(cx, window, "f");
        let p = panel(cx, &view);
        let c = &p.components["links-list"];
        let old = crate::lua::ui::list::ListHandle {
            owner: p.owner,
            identity: c.identity,
            context: view.read_with(cx, |v, _| v.lua.context()),
        };
        let q = query(cx, &view);
        cx.update_window(window, |_, w, cx| {
            q.update(cx, |q, cx| {
                q.replace_and_mark_text_in_range(None, "old", Some(3..3), w, cx)
            })
        })
        .unwrap();
        frame(cx, window);
        view.update(cx, |v, cx| {
            let request = pending(v.request_operation(DocumentOperation::Reload, origin, None, cx));
            assert!(!v.lua.context().ui.borrow().windows.is_empty());
            v.complete_fixture(success(request, "[New](new.md)".into()), cx);

        });
        frame(cx, window);
        assert!(!is_open(cx, &view));
        view.read_with(cx, |v, _| {
            assert_eq!(
                v.message.as_deref(),
                Some("Link selection closed because the document changed")
            );
            assert!(!v.message_is_error);
            assert_eq!(v.app.mode, Mode::Normal);
        });
        let (reads, _sub) = capture_reads(cx, &view);
        view.update(cx, |v, cx| {
            assert!(!old.confirm(v.lua.lua()).unwrap());
            v.pump_lua(cx);
        });
        assert!(reads.borrow().is_empty());
        press(cx, window, "f");
        assert_ne!(panel(cx, &view).owner, p.owner);
        assert_eq!(
            query(cx, &view).read_with(cx, |q, _| q.value().to_string()),
            ""
        );
    }
}

#[gpui_kit::test]
fn selection_panel_external_movements_and_mermaid_completion_keep_panel_and_input(
    cx: &mut TestAppContext,
) {
    let source = "[Design](design.md)\n\n```mermaid\nflowchart TD\nA --> B\n```\n\n".to_owned()
        + &"body\n\n".repeat(100);
    let (window, view) = open(cx, source);
    frame(cx, window);
    press(cx, window, "f");
    input(cx, window, "dsg");
    let owner = panel(cx, &view).owner;
    for operation in [
        ExternalOperation::Scroll(ScrollDirection::Down),
        ExternalOperation::Bottom,
        ExternalOperation::Top,
    ] {
        view.update(cx, |v, cx| {
            assert!(matches!(
                v.request_external(operation, ExternalRequestId("move".into()), cx),
                OperationResult::Complete {
                    outcome: Ok(()),
                    ..
                }
            ))
        });
        frame(cx, window);
        assert_eq!(panel(cx, &view).owner, owner);
    }
    view.update(cx, |v, cx| {
        v.complete_diagram(
            DiagramResult {
                request: running_diagram(v),
                png: Ok(PNG.to_vec()),
            },
            cx,
        )
    });
    for _ in 0..3 {
        frame(cx, window);
    }
    assert_eq!(panel(cx, &view).owner, owner);
    assert_eq!(
        query(cx, &view).read_with(cx, |q, _| q.value().to_string()),
        "dsg"
    );
    view.read_with(cx, |v, _| assert_eq!(v.document.diagram_images.len(), 1));
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    assert_eq!(
        visual.debug_bounds("mermaid-21").unwrap().size,
        size(px(10.), px(5.))
    );
}

#[gpui_kit::test]
fn selection_panel_confirm_then_read_failure_keeps_original_body_distance_and_history_without_reopen(
    cx: &mut TestAppContext,
) {
    let source = "[Missing](missing.md)\n\n".to_owned() + &"Original body\n\n".repeat(100);
    let (window, view) = open(cx, source.clone());
    frame(cx, window);
    press(cx, window, "ctrl-d");
    let distance = view.read_with(cx, |v, cx| v.document.distance(cx));
    let (reads, _sub) = capture_reads(cx, &view);
    press(cx, window, "f");
    press(cx, window, "enter");
    let request = reads.borrow_mut().pop().unwrap();
    view.update(cx, |v, cx| {
        v.complete_fixture(
            ReadResult {

                request,
                content: Err("missing destination".into()),
            },
            cx,
        )
    });
    frame(cx, window);
    assert!(!is_open(cx, &view));
    cx.update_window(window, |_, w, cx| {
        view.update(cx, |v, cx| {
            assert_eq!(v.app.mode, Mode::Normal);
            assert_eq!(v.status_text(), "missing destination");
            assert_eq!(v.status_severity(), Some(crate::config::Group::Error));
            assert_eq!(v.app.document().unwrap().markdown_source().unwrap().as_str(), source);
            assert_eq!(v.document.distance(cx), distance);
            assert!(v.body_focus.is_focused(w));
            assert!(matches!(
                v.request_operation(DocumentOperation::Back, OperationOrigin::Keyboard, None, cx),
                OperationResult::Complete {
                    outcome: Err(OperationError::NoPreviousEntry),
                    ..
                }
            ));
        })
    })
    .unwrap();
}

#[gpui_kit::test]
fn selection_panel_external_history_adoption_closes_only_on_success(cx: &mut TestAppContext) {
    let (window, view) = open(cx, "[First](first.md)".into());
    frame(cx, window);
    view.update(cx, |v, cx| {
        let request = pending(v.request_document(v.path.with_file_name("second.md"), None, cx));
        v.complete_fixture(success(request, "[Second](second-link.md)".into()), cx);

    });
    frame(cx, window);
    press(cx, window, "f");
    input(cx, window, "second");
    let owner = panel(cx, &view).owner;
    view.update(cx, |v, cx| {
        let request = pending(v.request_external(
            ExternalOperation::Back,
            ExternalRequestId("back-failed".into()),
            cx,
        ));
        v.complete_fixture(
            ReadResult {

                request,
                content: Err("history missing".into()),
            },
            cx,
        );
    });
    frame(cx, window);
    assert_eq!(panel(cx, &view).owner, owner);
    assert_eq!(
        query(cx, &view).read_with(cx, |q, _| q.value().to_string()),
        "second"
    );
    view.update(cx, |v, cx| {
        let request = pending(v.request_external(
            ExternalOperation::Back,
            ExternalRequestId("back".into()),
            cx,
        ));
        v.complete_fixture(success(request, "[First](first.md)".into()), cx);

    });
    frame(cx, window);
    assert!(!is_open(cx, &view));
    view.read_with(cx, |v, _| {
        assert_eq!(
            v.message.as_deref(),
            Some("Link selection closed because the document changed")
        )
    });
}

#[gpui_kit::test]
fn selection_panel_real_image_decode_completion_does_not_close_or_reset_panel(
    cx: &mut TestAppContext,
) {
    use gpui_kit::http_client::{AsyncBody, FakeHttpClient, Response};
    use std::{
        sync::{Arc, Mutex},
        task::{Poll, Waker},
    };
    let gate: Arc<Mutex<(bool, Option<Waker>)>> = Arc::new(Mutex::new((false, None)));
    let waiting = gate.clone();
    let client = FakeHttpClient::create(move |request| {
        assert_eq!(
            request.uri().to_string(),
            "https://images.example.test/pixel.png"
        );
        let waiting = waiting.clone();
        async move {
            std::future::poll_fn(|cx| {
                let mut gate = waiting.lock().unwrap();
                if gate.0 {
                    Poll::Ready(())
                } else {
                    gate.1 = Some(cx.waker().clone());
                    Poll::Pending
                }
            })
            .await;
            Ok(Response::builder()
                .status(200)
                .body(AsyncBody::from(PNG.to_vec()))
                .unwrap())
        }
    });
    cx.update(|cx| cx.set_http_client(client));
    let (window, view) = open(
        cx,
        "[Design](design.md)\n\n![pixel](https://images.example.test/pixel.png)".into(),
    );
    frame(cx, window);
    assert!(
        gate.lock().unwrap().1.is_some(),
        "the real image read must be pending before opening the panel"
    );
    press(cx, window, "f");
    input(cx, window, "dsg");
    let selection = panel(cx, &view).owner;
    let wake = {
        let mut gate = gate.lock().unwrap();
        gate.0 = true;
        gate.1.take().unwrap()
    };
    wake.wake();
    for _ in 0..3 {
        frame(cx, window);
    }
    assert_eq!(panel(cx, &view).owner, selection);
    assert_eq!(
        query(cx, &view).read_with(cx, |q, _| q.value().to_string()),
        "dsg"
    );
    view.read_with(cx, |viewer, cx| {
        let row = viewer
            .document
            .text
            .read(cx)
            .list_state()
            .bounds_for_item(1)
            .unwrap();
        assert_eq!(
            row.size.height,
            px(10.),
            "the actual PNG must have decoded and reached layout"
        );
    });
}

#[gpui_kit::test]
fn selection_panel_cancel_preserves_search_matches_highlights_position_and_history(
    cx: &mut TestAppContext,
) {
    let (window, view) = open(cx, "[Link](link.md) target target\n\n".repeat(100));
    frame(cx, window);
    press(cx, window, "/");
    input(cx, window, "target");
    press(cx, window, "enter");
    press(cx, window, "n");
    press(cx, window, ":");
    input(cx, window, "noh");
    press(cx, window, "enter");
    press(cx, window, "ctrl-d");
    let (matches, current, distance) = view.read_with(cx, |viewer, cx| {
        (
            viewer.app.matches().to_vec(),
            viewer.app.current_match(),
            viewer.document.distance(cx),
        )
    });
    press(cx, window, "f");
    input(cx, window, "unmatched");
    press(cx, window, "escape");
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.app.matches(), matches);
        assert_eq!(viewer.app.current_match(), current);
        assert!(!viewer.app.highlights_visible());
        assert_eq!(viewer.document.distance(cx), distance);
    });
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            assert!(matches!(
                viewer.request_operation(
                    DocumentOperation::Back,
                    OperationOrigin::Keyboard,
                    None,
                    cx
                ),
                OperationResult::Complete {
                    outcome: Err(OperationError::NoPreviousEntry),
                    ..
                }
            ));
        })
    });
    // An empty body search still reuses the original confirmed query.
    press(cx, window, "/");
    press(cx, window, "enter");
    view.read_with(cx, |viewer, _| assert_eq!(viewer.app.matches(), matches));
}

#[gpui_kit::test]
fn selection_panel_external_adoption_restores_only_logical_focus_and_does_not_activate(
    cx: &mut TestAppContext,
) {
    let (window, view) = open(cx, "[Old](old.md)".into());
    frame(cx, window);
    press(cx, window, "f");
    // A second headless window represents the external app's active window.
    let (other, _) = open(cx, "Other application".into());
    cx.update_window(other, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.active_window()), Some(other));
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let request = pending(viewer.request_document(
                viewer.path.with_file_name("external.md"),
                None,
                cx,
            ));
            viewer.complete_fixture(success(request, "External body".into()), cx);
        })
    });
    frame(cx, window);
    cx.update_window(window, |_, window, cx| {
        assert_eq!(
            cx.active_window(),
            Some(other),
            "external adoption must not activate the viewer"
        );
        assert!(view.read(cx).body_focus.is_focused(window));
    })
    .unwrap();
}
