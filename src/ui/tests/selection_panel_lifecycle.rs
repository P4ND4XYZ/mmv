use super::selection_panel::{frame, input, panel, press};
use super::*;
use crate::app::{
    DocumentOperation, ExternalOperation, Mode, OperationError, OperationOrigin, OperationResult,
    ScrollDirection,
};
use gpui_kit::EntityInputHandler as _;

// Embedded fixture only: no new fixture path or filesystem/network dependency.
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
    let selection = panel(cx, &view);
    let (generation, query, selected) = view.read_with(cx, |viewer, cx| {
        let panel = selection.read(cx);
        (
            viewer.app.document().unwrap().generation,
            panel.input.read(cx).value(),
            panel.command.read(cx).selected_index(),
        )
    });
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let old =
                pending(viewer.request_document(viewer.path.with_file_name("old.md"), None, cx));
            let latest = pending(viewer.request_document(
                viewer.path.with_file_name("missing.md"),
                None,
                cx,
            ));
            assert!(viewer.link_panel.is_some(), "request alone is not adoption");
            viewer.complete(
                ReadResult {
                    request: latest,
                    content: Err("not found".into()),
                },
                cx,
            );
            viewer.complete(success(old, "stale body".into()), cx);
        })
    });
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.app.document().unwrap().generation, generation);
        assert_eq!(viewer.app.error(), Some("not found"));
        assert_eq!(viewer.link_panel.as_ref().unwrap().panel, selection);
        let panel = selection.read(cx);
        assert_eq!(panel.input.read(cx).value(), query);
        assert_eq!(panel.command.read(cx).selected_index(), selected);
        assert!(viewer.message.is_none());
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
        let old_panel = panel(cx, &view);
        cx.update_window(window, |_, window, cx| {
            let query = old_panel.read(cx).input.clone();
            query.update(cx, |query, cx| {
                query.replace_and_mark_text_in_range(None, "old", Some(3..3), window, cx)
            });
        })
        .unwrap();
        frame(cx, window);
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                let request =
                    pending(viewer.request_operation(DocumentOperation::Reload, origin, None, cx));
                assert!(viewer.link_panel.is_some());
                viewer.complete(success(request, "[New](new.md)".into()), cx);
                assert!(viewer.link_panel.is_none());
            })
        });
        frame(cx, window);
        view.read_with(cx, |viewer, _| {
            assert_eq!(
                viewer.message.as_deref(),
                Some("本文が更新されたためリンク選択を閉じました")
            );
            assert!(!viewer.message_is_error);
            assert_eq!(
                viewer.status_severity(),
                Some(crate::config::Group::Warning)
            );
            assert_eq!(viewer.app.mode, Mode::Normal);
        });
        let reads = Rc::new(RefCell::new(Vec::new()));
        let captured = reads.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&view, move |_, request: &ReadRequest, _| {
                captured.borrow_mut().push(request.clone())
            })
        });
        // Retaining the old entity does not retain its execution subscription.
        cx.update(|cx| {
            old_panel.update(cx, |_, cx| {
                cx.emit(crate::ui::selection_panel::Event::Confirm(0))
            })
        });
        frame(cx, window);
        assert!(reads.borrow().is_empty());
        press(cx, window, "f");
        panel(cx, &view).read_with(cx, |panel, cx| {
            assert!(panel.input.read(cx).value().is_empty());
            assert_eq!(panel.items[0].primary, "New");
        });
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
    let selection = panel(cx, &view);
    for operation in [
        ExternalOperation::Scroll(ScrollDirection::Down),
        ExternalOperation::Bottom,
        ExternalOperation::Top,
    ] {
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                let result =
                    viewer.request_external(operation, ExternalRequestId("move".into()), cx);
                assert!(matches!(
                    result,
                    OperationResult::Complete {
                        outcome: Ok(()),
                        ..
                    }
                ));
            })
        });
        frame(cx, window);
        assert_eq!(panel(cx, &view), selection);
    }
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.complete_diagram(
                DiagramResult {
                    request: running_diagram(viewer),
                    png: Ok(PNG.to_vec()),
                },
                cx,
            );
            cx.notify();
        })
    });
    frame(cx, window);
    assert_eq!(panel(cx, &view), selection);
    selection.read_with(cx, |panel, cx| {
        assert_eq!(panel.input.read(cx).value().as_ref(), "dsg")
    });
    view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.document.diagram_images.len(), 1)
    });
    for _ in 0..3 {
        frame(cx, window);
    }
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    assert_eq!(
        visual.debug_bounds("mermaid-21").unwrap().size,
        size(px(10.), px(5.)),
        "the completed Mermaid PNG must reach actual layout"
    );
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
    let selection = panel(cx, &view);
    let wake = {
        let mut gate = gate.lock().unwrap();
        gate.0 = true;
        gate.1.take().unwrap()
    };
    wake.wake();
    for _ in 0..3 {
        frame(cx, window);
    }
    assert_eq!(panel(cx, &view), selection);
    selection.read_with(cx, |panel, cx| {
        assert_eq!(panel.input.read(cx).value().as_ref(), "dsg")
    });
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
fn selection_panel_confirm_then_read_failure_keeps_original_body_distance_and_history_without_reopen(
    cx: &mut TestAppContext,
) {
    let source = "[Missing](missing.md)\n\n".to_owned() + &"Original body\n\n".repeat(100);
    let (window, view) = open(cx, source.clone());
    frame(cx, window);
    press(cx, window, "ctrl-d");
    let distance = view.read_with(cx, |viewer, cx| viewer.document.distance(cx));
    let reads = Rc::new(RefCell::new(Vec::new()));
    let captured = reads.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone())
        })
    });
    press(cx, window, "f");
    press(cx, window, "enter");
    let request = reads.borrow_mut().pop().unwrap();
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.complete(
                ReadResult {
                    request,
                    content: Err("missing destination".into()),
                },
                cx,
            )
        })
    });
    frame(cx, window);
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |viewer, cx| {
            assert!(viewer.link_panel.is_none());
            assert_eq!(viewer.app.mode, Mode::Normal);
            assert_eq!(viewer.app.error(), Some("missing destination"));
            assert_eq!(viewer.app.document().unwrap().source, source);
            assert_eq!(viewer.document.distance(cx), distance);
            assert!(viewer.body_focus.is_focused(window));
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
    })
    .unwrap();
}

#[gpui_kit::test]
fn selection_panel_external_history_adoption_closes_only_on_success(cx: &mut TestAppContext) {
    let (window, view) = open(cx, "[First](first.md)".into());
    frame(cx, window);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let request =
                pending(viewer.request_document(viewer.path.with_file_name("second.md"), None, cx));
            viewer.complete(success(request, "[Second](second-link.md)".into()), cx);
        })
    });
    frame(cx, window);
    press(cx, window, "f");
    input(cx, window, "second");
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let failed = pending(viewer.request_external(
                ExternalOperation::Back,
                ExternalRequestId("back-failed".into()),
                cx,
            ));
            viewer.complete(
                ReadResult {
                    request: failed,
                    content: Err("history missing".into()),
                },
                cx,
            );
            assert!(viewer.link_panel.is_some());
        })
    });
    frame(cx, window);
    panel(cx, &view).read_with(cx, |panel, cx| {
        assert_eq!(panel.input.read(cx).value().as_ref(), "second")
    });
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let request = pending(viewer.request_external(
                ExternalOperation::Back,
                ExternalRequestId("back".into()),
                cx,
            ));
            viewer.complete(success(request, "[First](first.md)".into()), cx);
        })
    });
    frame(cx, window);
    view.read_with(cx, |viewer, _| {
        assert!(viewer.link_panel.is_none());
        assert_eq!(
            viewer.message.as_deref(),
            Some("本文が更新されたためリンク選択を閉じました")
        );
    });
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
            viewer.complete(success(request, "External body".into()), cx);
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
