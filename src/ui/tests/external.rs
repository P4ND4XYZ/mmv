use std::{cell::RefCell, rc::Rc};

use gpui_kit::{AppContext as _, Focusable as _, TestAppContext, test::TestWindowExt as _};

use crate::app::{
    DocumentOperation, ExternalOperation, ExternalRequestId, Mode, OperationError, OperationOrigin,
    OperationResult, ReadCompletion, ReadRequest, ReadResult, ScrollDirection,
};

fn frame(cx: &mut TestAppContext, window: gpui_kit::AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn external_scrolls_and_edges_use_keyboard_distances_without_changing_input(
    cx: &mut TestAppContext,
) {
    for key in [None, Some("/"), Some(":")] {
        let (window, view) = super::open(cx, "edit paragraph.\n\n".repeat(200));
        frame(cx, window);
        if let Some(key) = key {
            cx.update_window(window, |_, window, cx| {
                window.press(key, cx);
                window.input("edit", cx);
            })
            .unwrap();
            frame(cx, window);
        }
        let (mode, input) = view.read_with(cx, |viewer, _| {
            (viewer.app.mode.clone(), viewer.input.state.clone())
        });
        for (direction, lines, pages) in [
            (ScrollDirection::Down, 1., 0.),
            (ScrollDirection::Up, -1., 0.),
            (ScrollDirection::HalfDown, 0., 0.5),
            (ScrollDirection::HalfUp, 0., -0.5),
            (ScrollDirection::PageDown, 0., 1.),
            (ScrollDirection::PageUp, 0., -1.),
        ] {
            let expected = cx.update(|cx| {
                view.update(cx, |viewer, cx| {
                    viewer
                        .document
                        .text
                        .read(cx)
                        .list_state()
                        .set_offset_from_scrollbar(gpui_kit::point(
                            gpui_kit::px(0.),
                            gpui_kit::px(-1000.),
                        ));
                    viewer.document.scroll(lines, pages, cx);
                    let expected = viewer.document.distance(cx);
                    viewer
                        .document
                        .text
                        .read(cx)
                        .list_state()
                        .set_offset_from_scrollbar(gpui_kit::point(
                            gpui_kit::px(0.),
                            gpui_kit::px(-1000.),
                        ));
                    let id = ExternalRequestId(format!("{direction:?}"));
                    assert_eq!(
                        viewer.request_external(
                            ExternalOperation::Scroll(direction),
                            id.clone(),
                            cx
                        ),
                        OperationResult::Complete {
                            response_to: Some(id),
                            outcome: Ok(())
                        }
                    );
                    expected
                })
            });
            frame(cx, window);
            view.read_with(cx, |viewer, cx| {
                assert_eq!(viewer.document.distance(cx), expected);
                assert_ne!(expected, 1000., "{direction:?} must actually move");
            });
        }
        for (operation, end) in [
            (ExternalOperation::Bottom, true),
            (ExternalOperation::Top, false),
        ] {
            cx.update(|cx| {
                view.update(cx, |viewer, cx| {
                    assert_eq!(
                        viewer.request_external(operation, ExternalRequestId("edge".into()), cx),
                        OperationResult::Complete {
                            response_to: Some(ExternalRequestId("edge".into())),
                            outcome: Ok(())
                        }
                    );
                })
            });
            frame(cx, window);
            view.read_with(cx, |viewer, cx| {
                if end {
                    assert_eq!(
                        viewer
                            .document
                            .text
                            .read(cx)
                            .list_state()
                            .is_scrolled_to_end(),
                        Some(true)
                    );
                } else {
                    assert_eq!(viewer.document.distance(cx), 0.);
                }
            });
        }
        cx.update_window(window, |_, window, cx| {
            view.read_with(cx, |viewer, cx| {
                assert_eq!(viewer.app.mode, mode);
                assert_eq!(viewer.input.state, input);
                assert_eq!(
                    viewer.input.state.read(cx).value().as_ref(),
                    if key.is_some() { "edit" } else { "" }
                );
                let focus = if key.is_some() {
                    input.focus_handle(cx)
                } else {
                    viewer.body_focus.clone()
                };
                assert!(focus.is_focused(window));
            });
            if key.is_some() {
                window.input("x", cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
        if key.is_some() {
            view.read_with(cx, |viewer, cx| {
                assert_eq!(viewer.input.state.read(cx).value().as_ref(), "editx")
            });
        }
    }
}

#[gpui_kit::test]
fn external_open_and_history_wait_for_reads_and_keep_search_command_and_focus(
    cx: &mut TestAppContext,
) {
    for (key, mode) in [("/", Mode::Search), (":", Mode::Command)] {
        let source = "edit paragraph.\n\n".repeat(200);
        let (window, view) = super::open(cx, source.clone());
        frame(cx, window);
        let reads = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
        let completions = Rc::new(RefCell::new(Vec::<ReadCompletion>::new()));
        let (captured_reads, captured_completions) = (reads.clone(), completions.clone());
        let (_reads, _completions) = cx.update(|cx| {
            (
                cx.subscribe(&view, move |_, request: &ReadRequest, _| {
                    captured_reads.borrow_mut().push(request.clone())
                }),
                cx.subscribe(&view, move |_, completion: &ReadCompletion, _| {
                    captured_completions.borrow_mut().push(completion.clone())
                }),
            )
        });
        cx.update_window(window, |_, window, cx| {
            window.press(key, cx);
            window.input("edit", cx);
        })
        .unwrap();
        frame(cx, window);
        let input = view.read_with(cx, |viewer, _| viewer.input.state.clone());
        let original = view.read_with(cx, |viewer, _| viewer.path.clone());
        let other = original.with_file_name("external 日本語.md");
        for (operation, target, distance) in [
            (
                ExternalOperation::Open {
                    path: other.clone(),
                    line: None,
                },
                other.clone(),
                0.,
            ),
            (ExternalOperation::Back, original.clone(), 480.),
            (ExternalOperation::Forward, other.clone(), 480.),
            (
                ExternalOperation::Open {
                    path: other.clone(),
                    line: None,
                },
                other.clone(),
                480.,
            ),
        ] {
            let id = ExternalRequestId(format!("{operation:?}"));
            let previous = view.read_with(cx, |viewer, _| viewer.path.clone());
            let same_open =
                matches!(operation, ExternalOperation::Open { .. }) && previous == target;
            let result = cx.update(|cx| {
                view.update(cx, |viewer, cx| {
                    viewer
                        .document
                        .text
                        .read(cx)
                        .list_state()
                        .set_offset_from_scrollbar(gpui_kit::point(
                            gpui_kit::px(0.),
                            gpui_kit::px(-480.),
                        ));
                    viewer.request_external(operation, id.clone(), cx)
                })
            });
            let OperationResult::Pending(request) = result else {
                panic!("must wait for reading");
            };
            assert_eq!(reads.borrow_mut().pop(), Some(request.clone()));
            assert_eq!(request.path, target);
            assert_eq!(request.response_to, Some(id.clone()));
            assert_eq!(request.origin, OperationOrigin::External);
            assert!(completions.borrow().is_empty());
            view.read_with(cx, |viewer, _| assert_eq!(viewer.path, previous));
            cx.update(|cx| {
                view.update(cx, |viewer, cx| {
                    viewer.complete(super::success(request, source.clone()), cx)
                })
            });
            frame(cx, window);
            let completion = completions.borrow_mut().pop().unwrap();
            assert_eq!(completion.response_to, Some(id));
            assert_eq!(completion.outcome, Ok(()));
            assert_eq!(completion.adopted, !same_open);
            cx.update_window(window, |_, window, cx| {
                view.read_with(cx, |viewer, cx| {
                    assert_eq!(viewer.path, target);
                    assert_eq!(viewer.document.distance(cx), distance);
                    assert_eq!(viewer.app.mode, mode);
                    assert_eq!(viewer.input.state, input);
                    assert_eq!(input.read(cx).value().as_ref(), "edit");
                    assert!(input.focus_handle(cx).is_focused(window));
                    if mode == Mode::Search {
                        assert_eq!(viewer.app.matches().len(), 200);
                    }
                })
            })
            .unwrap();
        }
        let result = cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                viewer.request_external(
                    ExternalOperation::Forward,
                    ExternalRequestId("no-forward".into()),
                    cx,
                )
            })
        });
        assert_eq!(
            result,
            OperationResult::Complete {
                response_to: Some(ExternalRequestId("no-forward".into())),
                outcome: Err(OperationError::NoNextEntry)
            }
        );
        assert!(reads.borrow().is_empty());
        // A read error is not acknowledged as success at acceptance, in either mode.
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                let OperationResult::Pending(request) = viewer.request_external(
                    ExternalOperation::Open {
                        path: original.with_file_name("missing.md"),
                        line: None,
                    },
                    ExternalRequestId("failed".into()),
                    cx,
                ) else {
                    panic!("read required");
                };
                viewer.complete(
                    ReadResult {
                        request,
                        content: Err("file not found".into()),
                    },
                    cx,
                );
            })
        });
        frame(cx, window);
        let completion = completions.borrow_mut().pop().unwrap();
        assert_eq!(
            completion.response_to,
            Some(ExternalRequestId("failed".into()))
        );
        assert_eq!(completion.outcome, Err("file not found".into()));
        cx.update_window(window, |_, window, cx| {
            view.read_with(cx, |viewer, cx| {
                assert_eq!(viewer.path, other);
                assert_eq!(viewer.app.mode, mode);
                assert_eq!(viewer.input.state, input);
                assert_eq!(input.read(cx).value().as_ref(), "edit");
                assert!(input.focus_handle(cx).is_focused(window));
            })
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn external_bottom_after_adoption_is_not_overwritten_by_pending_top(cx: &mut TestAppContext) {
    let source = "Paragraph.\n\n".repeat(200);
    let (window, view) = super::open(cx, source.clone());
    frame(cx, window);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let OperationResult::Pending(request) = viewer.request_external(
                ExternalOperation::Open {
                    path: viewer.path.with_file_name("before-layout.md"),
                    line: None,
                },
                ExternalRequestId("open-before-layout".into()),
                cx,
            ) else {
                panic!("read required");
            };
            viewer.complete(super::success(request, source), cx);
            assert_eq!(
                viewer.app.position_request().unwrap().target,
                crate::app::PositionTarget::Top
            );
            assert_eq!(
                viewer.request_external(
                    ExternalOperation::Bottom,
                    ExternalRequestId("bottom-before-layout".into()),
                    cx
                ),
                OperationResult::Complete {
                    response_to: Some(ExternalRequestId("bottom-before-layout".into())),
                    outcome: Ok(()),
                }
            );
        })
    });
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(
            viewer
                .document
                .text
                .read(cx)
                .list_state()
                .is_scrolled_to_end(),
            Some(true),
            "a successful external bottom must not be undone by the pending Top"
        );
    });
}

#[gpui_kit::test]
fn movements_before_layout_follow_top_and_distance_and_are_consumed_once(cx: &mut TestAppContext) {
    let source = "Paragraph.\n\n".repeat(200);
    for (reload, movement, expected) in [
        (false, ExternalOperation::Scroll(ScrollDirection::Down), 20.),
        (true, ExternalOperation::Scroll(ScrollDirection::Down), 500.),
        (true, ExternalOperation::Top, 0.),
    ] {
        let (window, view) = super::open(cx, source.clone());
        frame(cx, window);
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                viewer
                    .document
                    .text
                    .read(cx)
                    .list_state()
                    .set_offset_from_scrollbar(gpui_kit::point(
                        gpui_kit::px(0.),
                        gpui_kit::px(-480.),
                    ));
                let path = if reload {
                    viewer.path.clone()
                } else {
                    viewer.path.with_file_name("new.md")
                };
                let result = if reload {
                    viewer.request_operation(
                        DocumentOperation::Reload,
                        OperationOrigin::External,
                        Some(ExternalRequestId("read".into())),
                        cx,
                    )
                } else {
                    viewer.request_external(
                        ExternalOperation::Open { path, line: None },
                        ExternalRequestId("read".into()),
                        cx,
                    )
                };
                let OperationResult::Pending(request) = result else {
                    panic!("read required");
                };
                viewer.complete(super::success(request, source.clone()), cx);
                assert!(viewer.app.position_request().is_some());
                assert_eq!(
                    viewer.request_external(
                        movement.clone(),
                        ExternalRequestId("movement".into()),
                        cx
                    ),
                    OperationResult::Complete {
                        response_to: Some(ExternalRequestId("movement".into())),
                        outcome: Ok(())
                    }
                );
            })
        });
        frame(cx, window);
        view.read_with(cx, |viewer, cx| {
            assert_eq!(
                viewer.document.distance(cx),
                expected,
                "reload={reload}, movement={movement:?}"
            );
            assert!(viewer.app.position_request().is_none());
        });
        // Later layouts must not replay an already consumed intent.
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                viewer
                    .document
                    .text
                    .read(cx)
                    .list_state()
                    .set_offset_from_scrollbar(gpui_kit::point(
                        gpui_kit::px(0.),
                        gpui_kit::px(-1000.),
                    ));
                cx.notify();
            })
        });
        frame(cx, window);
        frame(cx, window);
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.document.distance(cx), 1000.)
        });
    }
}

#[gpui_kit::test]
fn relative_movements_wait_for_nested_heading_and_preserve_acceptance_order(
    cx: &mut TestAppContext,
) {
    let source = "Paragraph.\n\n".repeat(80) + "> # Nested target\n\n" + &"Tail.\n\n".repeat(100);
    let (window, view) = super::open(cx, source.clone());
    frame(cx, window);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.request_operation(
                DocumentOperation::Heading {
                    path: None,
                    id: "nested-target".into(),
                },
                OperationOrigin::External,
                None,
                cx,
            );
        })
    });
    frame(cx, window);
    frame(cx, window);
    let heading_distance = view.read_with(cx, |viewer, cx| {
        assert!(viewer.app.position_request().is_none());
        let distance = viewer.document.distance(cx);
        assert!(distance > 1000.);
        distance
    });
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let OperationResult::Pending(request) = viewer.request_operation(
                DocumentOperation::Heading {
                    path: Some(viewer.path.with_file_name("nested.md")),
                    id: "nested-target".into(),
                },
                OperationOrigin::External,
                None,
                cx,
            ) else {
                panic!("read required");
            };
            viewer.complete(super::success(request, source), cx);
            // Up, then down by two lines: clamping at the fresh document's top
            // would produce 40 rather than the required heading distance + 20.
            for direction in [
                ScrollDirection::Up,
                ScrollDirection::Down,
                ScrollDirection::Down,
            ] {
                assert!(matches!(
                    viewer.request_external(
                        ExternalOperation::Scroll(direction),
                        ExternalRequestId(format!("{direction:?}")),
                        cx
                    ),
                    OperationResult::Complete {
                        outcome: Ok(()),
                        ..
                    }
                ));
            }
        })
    });
    frame(cx, window);
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert!(viewer.app.position_request().is_none());
        assert_eq!(viewer.document.distance(cx), heading_distance + 20.);
    });
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), heading_distance + 20.)
    });
}

#[gpui_kit::test]
fn newer_document_and_same_generation_heading_supersede_unapplied_movements(
    cx: &mut TestAppContext,
) {
    let source = "# Start\n\n".to_owned() + &"Paragraph.\n\n".repeat(200);
    // Different-key Open, same-generation Heading, and loaded same-key Heading
    // all supersede intent, unlike an Open that only reloads the same key.
    for navigation in 0..3 {
        let (window, view) = super::open(cx, source.clone());
        frame(cx, window);
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                let OperationResult::Pending(request) = viewer.request_external(
                    ExternalOperation::Open {
                        path: viewer.path.with_file_name("first.md"),
                        line: None,
                    },
                    ExternalRequestId("first".into()),
                    cx,
                ) else {
                    panic!("read required");
                };
                viewer.complete(super::success(request, source.clone()), cx);
                viewer.request_external(
                    ExternalOperation::Bottom,
                    ExternalRequestId("old-bottom".into()),
                    cx,
                );
                viewer.request_external(
                    ExternalOperation::Scroll(ScrollDirection::Up),
                    ExternalRequestId("old-up".into()),
                    cx,
                );
                if navigation == 0 {
                    let OperationResult::Pending(request) = viewer.request_external(
                        ExternalOperation::Open {
                            path: viewer.path.with_file_name("second.md"),
                            line: None,
                        },
                        ExternalRequestId("second".into()),
                        cx,
                    ) else {
                        panic!("read required");
                    };
                    viewer.complete(super::success(request, source.clone()), cx);
                } else if navigation == 1 {
                    let OperationResult::Pending(request) = viewer.request_operation(
                        DocumentOperation::Heading {
                            path: Some(viewer.path.clone()),
                            id: "start".into(),
                        },
                        OperationOrigin::External,
                        None,
                        cx,
                    ) else {
                        panic!("read required");
                    };
                    viewer.complete(super::success(request, source.clone()), cx);
                } else {
                    viewer.request_operation(
                        DocumentOperation::Heading {
                            path: None,
                            id: "start".into(),
                        },
                        OperationOrigin::External,
                        None,
                        cx,
                    );
                }
            })
        });
        frame(cx, window);
        frame(cx, window);
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.document.distance(cx), 0.);
            assert!(viewer.app.position_request().is_none());
        });
    }
}

// No frame between initial adoption, movement acceptance, and replacement:
// this models watcher readiness/catch-up before the first body layout.
fn replacement_keeps_unapplied_movement(
    cx: &mut TestAppContext,
    automatic: bool,
    movement: ExternalOperation,
    fail: bool,
) {
    let source = "Paragraph.\n\n".repeat(200);
    let (window, view) = super::open(cx, source.clone());
    frame(cx, window);
    let reads = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = reads.clone();
    let replacement_source = source.clone();
    let _reads = cx.update(|cx| {
        cx.subscribe(&view, move |view, request: &ReadRequest, cx| {
            captured.borrow_mut().push(request.clone());
            if request.origin == OperationOrigin::Automatic
                || request.response_to == Some(ExternalRequestId("replacement".into()))
            {
                // Complete during event delivery, before a frame can prepare layout.
                view.update(cx, |viewer, cx| {
                    assert!(viewer.app.position_request().is_some());
                    if fail {
                        viewer.complete(
                            ReadResult {
                                request: request.clone(),
                                content: Err("missing".into()),
                            },
                            cx,
                        );
                    } else {
                        viewer.complete(
                            super::success(request.clone(), replacement_source.clone()),
                            cx,
                        );
                    }
                });
            }
        })
    });
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let OperationResult::Pending(request) = viewer.request_external(
                ExternalOperation::Open {
                    path: viewer.path.with_file_name("before-reload.md"),
                    line: None,
                },
                ExternalRequestId("initial".into()),
                cx,
            ) else {
                panic!("read required");
            };
            viewer.complete(super::success(request, source.clone()), cx);
            assert_eq!(
                viewer.request_external(movement.clone(), ExternalRequestId("move".into()), cx),
                OperationResult::Complete {
                    response_to: Some(ExternalRequestId("move".into())),
                    outcome: Ok(()),
                }
            );
            if automatic {
                let target = viewer.app.watch_target().unwrap().clone();
                viewer.watch_changed(&target, cx);
            } else {
                let path = if fail {
                    viewer.path.with_file_name("missing.md")
                } else {
                    viewer.path.clone()
                };
                viewer.request_external(
                    ExternalOperation::Open { path, line: None },
                    ExternalRequestId("replacement".into()),
                    cx,
                );
            }
        })
    });
    let request = reads.borrow_mut().pop().expect("replacement read emitted");
    assert_eq!(
        request.origin,
        if automatic {
            OperationOrigin::Automatic
        } else {
            OperationOrigin::External
        }
    );
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        if movement == ExternalOperation::Bottom {
            assert_eq!(
                viewer
                    .document
                    .text
                    .read(cx)
                    .list_state()
                    .is_scrolled_to_end(),
                Some(true)
            );
        } else {
            assert_eq!(viewer.document.distance(cx), 20.);
        }
        assert!(viewer.app.position_request().is_none());
    });
    // Prove that carrying the intent to a new generation does not replay it.
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer
                .document
                .text
                .read(cx)
                .list_state()
                .set_offset_from_scrollbar(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-480.)));
            cx.notify();
        })
    });
    frame(cx, window);
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 480.)
    });
}

#[gpui_kit::test]
fn automatic_reload_preserves_unapplied_bottom(cx: &mut TestAppContext) {
    replacement_keeps_unapplied_movement(cx, true, ExternalOperation::Bottom, false);
}

#[gpui_kit::test]
fn automatic_reload_preserves_unapplied_relative_scroll(cx: &mut TestAppContext) {
    replacement_keeps_unapplied_movement(
        cx,
        true,
        ExternalOperation::Scroll(ScrollDirection::Down),
        false,
    );
}

#[gpui_kit::test]
fn same_document_open_preserves_unapplied_bottom(cx: &mut TestAppContext) {
    replacement_keeps_unapplied_movement(cx, false, ExternalOperation::Bottom, false);
}

#[gpui_kit::test]
fn same_document_open_preserves_unapplied_relative_scroll(cx: &mut TestAppContext) {
    replacement_keeps_unapplied_movement(
        cx,
        false,
        ExternalOperation::Scroll(ScrollDirection::Down),
        false,
    );
}

#[gpui_kit::test]
fn failed_document_switch_preserves_unapplied_movements(cx: &mut TestAppContext) {
    for movement in [
        ExternalOperation::Bottom,
        ExternalOperation::Scroll(ScrollDirection::Down),
    ] {
        replacement_keeps_unapplied_movement(cx, false, movement, true);
    }
}

#[gpui_kit::test]
fn external_movement_without_a_document_returns_failure_with_the_request_id(
    cx: &mut TestAppContext,
) {
    let (window, view) = super::open(cx, "body".into());
    cx.update(|cx| view.update(cx, |viewer, _| viewer.app = Default::default()));
    for operation in [
        ExternalOperation::Scroll(ScrollDirection::Down),
        ExternalOperation::Top,
        ExternalOperation::Bottom,
        ExternalOperation::Back,
        ExternalOperation::Forward,
    ] {
        let result = cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                viewer.request_external(
                    operation.clone(),
                    ExternalRequestId("unavailable".into()),
                    cx,
                )
            })
        });
        let expected = match operation {
            ExternalOperation::Back => OperationError::NoPreviousEntry,
            ExternalOperation::Forward => OperationError::NoNextEntry,
            _ => OperationError::DocumentUnavailable,
        };
        assert_eq!(
            result,
            OperationResult::Complete {
                response_to: Some(ExternalRequestId("unavailable".into())),
                outcome: Err(expected)
            }
        );
    }
    frame(cx, window);
}
