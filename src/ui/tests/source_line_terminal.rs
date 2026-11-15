use super::super::document::SourceLineOutcome;
use super::source_lines::{frame, navigate};
use super::*;
use crate::app::{
    DocumentOperation, ExternalOperation, OperationOrigin, OperationResult, ScrollDirection,
};
use gpui_kit::base::text::{
    RangeHighlight, TextRevealCancellation, TextRevealFailure, TextRevealStatus,
};

fn start(
    cx: &mut TestAppContext,
    view: &gpui_kit::Entity<Viewer>,
    line: u64,
) -> (
    crate::app::PositionRequest,
    gpui_kit::base::text::TextRevealHandle,
) {
    let request = navigate(cx, view, line);
    let handle = cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.settle_position(cx);
            viewer
                .document
                .source_handle()
                .expect("strict adapter handle")
        })
    });
    (request, handle)
}

fn frames(cx: &mut TestAppContext, window: gpui_kit::AnyWindowHandle) {
    for _ in 0..8 {
        frame(cx, window);
    }
}

#[gpui_kit::test]
fn adapter_repeat_latest_wins_and_queued_movement_does_not_reissue(cx: &mut TestAppContext) {
    let (window, view) = open(
        cx,
        "```text\n".to_owned() + &"body row\n".repeat(240) + "```\n",
    );
    frame(cx, window);
    let (old_request, old) = start(cx, &view, 180);
    let (new_request, new) = start(cx, &view, 180);
    assert_ne!(old.id(), new.id());
    assert_ne!(old_request.serial, new_request.serial);
    assert_eq!(
        old.status(),
        TextRevealStatus::Cancelled(TextRevealCancellation::Superseded)
    );
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let text = viewer.document.text.clone();
            viewer.finish_source_reveal(
                &old_request,
                &text,
                Some(old.id()),
                SourceLineOutcome::Reveal(old.status()),
                cx,
            );
            assert_eq!(viewer.app.position_request(), Some(&new_request));
            viewer.request_external(
                ExternalOperation::Scroll(ScrollDirection::Down),
                ExternalRequestId("move".into()),
                cx,
            );
            assert_eq!(viewer.document.source_handle().unwrap().id(), new.id());
        })
    });
    frames(cx, window);
    assert_eq!(new.status(), TextRevealStatus::Shown);
    view.read_with(cx, |viewer, _| {
        assert!(viewer.app.position_request().is_none())
    });
}

#[gpui_kit::test]
fn adapter_cancel_and_no_frame_timeout_release_queued_movement(cx: &mut TestAppContext) {
    for timeout in [false, true] {
        let (window, view) = open(cx, "paragraph\n\n".repeat(180));
        frame(cx, window);
        let (_, handle) = start(cx, &view, 101);
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                viewer.request_external(
                    ExternalOperation::Bottom,
                    ExternalRequestId("end".into()),
                    cx,
                );
            })
        });
        if timeout {
            cx.executor()
                .advance_clock(std::time::Duration::from_secs(1));
        } else {
            assert!(handle.cancel());
        }
        cx.run_until_parked();
        let expected = if timeout {
            TextRevealStatus::Failed(TextRevealFailure::TimedOut)
        } else {
            TextRevealStatus::Cancelled(TextRevealCancellation::Explicit)
        };
        assert_eq!(handle.status(), expected);
        view.read_with(cx, |viewer, _| {
            assert!(viewer.app.position_request().is_none());
            assert_eq!(
                viewer.document.source_outcome,
                Some(SourceLineOutcome::Reveal(expected))
            );
            // run_until_parked may also paint the queued anchor; terminal
            // release itself must not depend on another strict reveal frame.
        });
        frames(cx, window);
        view.read_with(cx, |viewer, cx| {
            assert!(!viewer.document.normalization_pending());
            assert!(
                viewer
                    .document
                    .text
                    .read(cx)
                    .list_state()
                    .is_scrolled_to_end()
                    .unwrap()
            );
        });
    }
}

#[gpui_kit::test]
fn adapter_end_and_overshooting_pages_save_normalized_history(cx: &mut TestAppContext) {
    for end in [false, true] {
        let source = "paragraph\n\n".repeat(80);
        let (window, view) = open(cx, "first document".into());
        frame(cx, window);
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                let OperationResult::Pending(read) =
                    viewer.request_document(viewer.path.with_file_name("second.md"), None, cx)
                else {
                    panic!()
                };
                viewer.complete(success(read, source.clone()), cx);
            })
        });
        frame(cx, window);
        let (_, handle) = start(cx, &view, 41);
        let reads = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
        let captured = reads.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&view, move |_, read: &ReadRequest, _| {
                captured.borrow_mut().push(read.clone())
            })
        });
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                for _ in 0..if end { 1 } else { 30 } {
                    viewer.request_external(
                        if end {
                            ExternalOperation::Bottom
                        } else {
                            ExternalOperation::Scroll(ScrollDirection::PageDown)
                        },
                        ExternalRequestId("move".into()),
                        cx,
                    );
                }
                viewer.request_external(
                    ExternalOperation::Back,
                    ExternalRequestId("back".into()),
                    cx,
                );
            })
        });
        assert!(reads.borrow().is_empty());
        // Exercise terminal failure too; Shown uses the same matching finalizer.
        if !end {
            assert!(handle.cancel());
        }
        for _ in 0..40 {
            frame(cx, window);
        }
        let normalized = view.read_with(cx, |viewer, cx| {
            assert!(!viewer.document.normalization_pending());
            assert!(
                viewer
                    .document
                    .text
                    .read(cx)
                    .list_state()
                    .is_scrolled_to_end()
                    .unwrap()
            );
            viewer.document.distance(cx)
        });
        let back = reads
            .borrow_mut()
            .pop()
            .expect("history released after normalization");
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                viewer.complete(success(back, "first document".into()), cx)
            })
        });
        frames(cx, window);
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                let OperationResult::Pending(read) = viewer.request_external(
                    ExternalOperation::Forward,
                    ExternalRequestId("forward".into()),
                    cx,
                ) else {
                    panic!()
                };
                viewer.complete(success(read, source.clone()), cx);
            })
        });
        frames(cx, window);
        view.read_with(cx, |viewer, cx| {
            assert!((viewer.document.distance(cx) - normalized).abs() < 0.5)
        });
    }
}

#[gpui_kit::test]
fn adapter_manual_cancel_and_new_read_wait_for_normalization(cx: &mut TestAppContext) {
    let (window, view) = open(cx, "paragraph\n\n".repeat(100));
    frame(cx, window);
    let (_, handle) = start(cx, &view, 81);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.document.scroll_edge(true, cx);
            assert_eq!(
                handle.status(),
                TextRevealStatus::Cancelled(TextRevealCancellation::Explicit)
            );
            let OperationResult::Pending(read) = viewer.request_operation(
                DocumentOperation::Open(viewer.path.with_file_name("replacement.md")),
                OperationOrigin::External,
                None,
                cx,
            ) else {
                panic!()
            };
            viewer.complete(success(read, "replacement".into()), cx);
            assert_ne!(
                viewer
                    .app
                    .document()
                    .unwrap()
                    .markdown_source()
                    .unwrap()
                    .as_str(),
                "replacement"
            );
        })
    });
    frames(cx, window);
    view.read_with(cx, |viewer, _| {
        assert_eq!(
            viewer
                .app
                .document()
                .unwrap()
                .markdown_source()
                .unwrap()
                .as_str(),
            "replacement"
        )
    });
}

#[gpui_kit::test]
fn adapter_mapping_failure_releases_movement_and_gates_read_completion(cx: &mut TestAppContext) {
    let source = "before\n\n".repeat(80) + "![opaque](missing.png)\n\n" + &"after\n\n".repeat(80);
    let (window, view) = open(cx, source);
    frame(cx, window);
    navigate(cx, &view, 161);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.request_external(
                ExternalOperation::Bottom,
                ExternalRequestId("end".into()),
                cx,
            );
            assert!(viewer.app.position_request().is_none());
            assert!(matches!(
                viewer.document.source_outcome,
                Some(SourceLineOutcome::Mapping(_))
            ));
            assert!(viewer.document.normalization_pending());
            let OperationResult::Pending(read) =
                viewer.request_document(viewer.path.with_file_name("next.md"), None, cx)
            else {
                panic!()
            };
            viewer.complete(success(read, "replacement".into()), cx);
            assert_ne!(
                viewer
                    .app
                    .document()
                    .unwrap()
                    .markdown_source()
                    .unwrap()
                    .as_str(),
                "replacement"
            );
        })
    });
    frames(cx, window);
    view.read_with(cx, |viewer, _| {
        assert_eq!(
            viewer
                .app
                .document()
                .unwrap()
                .markdown_source()
                .unwrap()
                .as_str(),
            "replacement"
        )
    });
}

const MARK: u32 = 0xf307b1;
fn mark_source(cx: &mut TestAppContext, view: &gpui_kit::Entity<Viewer>, source_start: usize) {
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let text = viewer.document.text.clone();
            let range = text
                .read(cx)
                .rendered_text()
                .range_for_source(source_start..source_start + 1)
                .expect("exact source projection");
            text.update(cx, |text, cx| {
                text.set_range_highlights([RangeHighlight::new(range, gpui_kit::rgb(MARK))], cx)
                    .unwrap()
            });
        })
    });
}
fn assert_marked_row_contained(
    cx: &mut TestAppContext,
    window: gpui_kit::AnyWindowHandle,
    view: &gpui_kit::Entity<Viewer>,
) {
    let row = cx
        .update_window(window, |_, window, _| {
            let background: gpui_kit::Background = gpui_kit::rgb(MARK).into();
            let quad = window
                .painted_quads()
                .into_iter()
                .find(|q| q.background == background)
                .expect("actual target row painted");
            let scale = window.scale_factor();
            (
                quad.bounds.origin.y.as_f32() / scale,
                (quad.bounds.origin.y.as_f32() + quad.bounds.size.height.as_f32()) / scale,
            )
        })
        .unwrap();
    view.read_with(cx, |viewer, cx| {
        assert_eq!(
            viewer.document.source_outcome,
            Some(SourceLineOutcome::Reveal(TextRevealStatus::Shown))
        );
        let viewport = viewer.document.text.read(cx).list_state().viewport_bounds();
        assert!(
            row.0 >= f32::from(viewport.top()) - 0.5 && row.1 <= f32::from(viewport.bottom()) + 0.5,
            "target row must be wholly contained"
        );
    });
}

#[gpui_kit::test]
fn adapter_long_code_shown_visible_zero_and_partial_minimum(cx: &mut TestAppContext) {
    let source = "```text\n".to_owned() + &"target row\n".repeat(240) + "```\n";
    let source_start = source.match_indices("target row").nth(179).unwrap().0;
    let (window, view) = open(cx, source);
    frame(cx, window);
    start(cx, &view, 181);
    mark_source(cx, &view, source_start);
    frames(cx, window);
    assert_marked_row_contained(cx, window, &view);
    let distance = view.read_with(cx, |viewer, cx| viewer.document.distance(cx));
    start(cx, &view, 181);
    frames(cx, window);
    assert_eq!(
        distance,
        view.read_with(cx, |viewer, cx| viewer.document.distance(cx))
    );
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer
                .document
                .scroll(-5. / super::super::status::FOOTER_LINE_HEIGHT, 0., cx)
        })
    });
    frames(cx, window);
    start(cx, &view, 181);
    frames(cx, window);
    let after = view.read_with(cx, |viewer, cx| viewer.document.distance(cx));
    assert!(
        (after - distance).abs() < 0.5,
        "partial row requires only the missing five pixels"
    );
    assert_marked_row_contained(cx, window, &view);
}

pub(super) fn private_native_adapter(cx: &mut TestAppContext) {
    let Some(path) = std::env::var_os("MMV_PRIVATE_SOURCE_FIXTURE") else {
        eprintln!("PRIVATE_NATIVE_ADAPTER not_run=fixture_absent");
        return;
    };
    let source = std::fs::read_to_string(path).expect("private fixture read");
    let line = std::env::var("MMV_PRIVATE_SOURCE_LINE")
        .ok()
        .map(|s| s.parse::<u64>().expect("private line"))
        .unwrap_or(109);
    let source_start = source
        .split_inclusive('\n')
        .take((line - 1) as usize)
        .map(str::len)
        .sum();
    let (window, view) = open(cx, source);
    frame(cx, window);
    let (_, handle) = start(cx, &view, line);
    mark_source(cx, &view, source_start);
    frames(cx, window);
    assert_eq!(
        handle.status(),
        TextRevealStatus::Shown,
        "private source line through adapter"
    );
    assert_marked_row_contained(cx, window, &view);
    eprintln!("PRIVATE_NATIVE_ADAPTER line={line} shown=true target_row_contained=true");
}
