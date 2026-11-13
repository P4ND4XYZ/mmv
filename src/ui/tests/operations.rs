use std::{
    cell::RefCell,
    io::Write as _,
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

struct LocalImage(PathBuf);

impl LocalImage {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let (path, mut file) = loop {
            let path = std::env::temp_dir().join(format!(
                "mmv-position-{}-{}.png",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => break (path, file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create test PNG: {error}"),
            }
        };
        let image = Self(path);
        // A 40x180 transparent PNG. Its intrinsic height is much larger than
        // the unloaded/failed image placeholder; no explicit Markdown size is used.
        file.write_all(&[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 40, 0, 0, 0,
            180, 8, 6, 0, 0, 0, 230, 90, 110, 85, 0, 0, 0, 1, 115, 82, 71, 66, 0, 174, 206, 28,
            233, 0, 0, 0, 4, 103, 65, 77, 65, 0, 0, 177, 143, 11, 252, 97, 5, 0, 0, 0, 9, 112, 72,
            89, 115, 0, 0, 14, 195, 0, 0, 14, 195, 1, 199, 111, 168, 100, 0, 0, 0, 51, 73, 68, 65,
            84, 120, 94, 237, 193, 49, 1, 0, 0, 0, 194, 160, 245, 79, 109, 9, 79, 160, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 254, 6, 113,
            52, 0, 1, 144, 71, 219, 236, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ])
        .expect("write test PNG");
        image
    }
}

impl Drop for LocalImage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, TestAppContext, point, px, test::TestWindowExt as _,
};

use super::Viewer;

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

fn scroll(cx: &mut TestAppContext, view: &Entity<Viewer>, distance: f32) {
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.document.text.update(cx, |text, cx| {
                text.list_state()
                    .set_offset_from_scrollbar(point(px(0.), px(-distance)));
                cx.notify();
            });
        });
    });
}

fn replace(cx: &mut TestAppContext, view: &Entity<Viewer>, source: String, other: bool) {
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let operation = if other {
                DocumentOperation::Open(viewer.path.with_file_name("other.md"))
            } else {
                DocumentOperation::Reload
            };
            let OperationResult::Pending(request) =
                viewer.request_operation(operation, OperationOrigin::External, None, cx)
            else {
                panic!("read required");
            };
            viewer.complete(super::success(request, source), cx);
            let position = viewer.app.position_request().unwrap().clone();
            let text = viewer.document.text.clone();
            // A fresh TextView has no viewport yet, including for empty source.
            viewer.apply_position_after_layout(&position, &text, cx);
            assert_eq!(viewer.app.position_request(), Some(&position));
        });
    });
}

fn fixture() -> String {
    "# Start\n\n".to_owned()
        + &"A long wrapped paragraph with 日本語 text. ".repeat(200)
        + "\n\n| Column | Value |\n| --- | --- |\n"
        + &"| table cell | wrapped table contents 日本語 |\n".repeat(30)
        + "\n"
        + &"Offscreen target paragraph.\n\n".repeat(100)
}

#[gpui_kit::test]
fn production_distance_restoration_measures_rich_content_and_clamps(cx: &mut TestAppContext) {
    let image = LocalImage::new();
    let source = fixture().replacen(
        "Offscreen target paragraph.",
        &format!(
            "![local PNG]({})\n\nOffscreen target paragraph.",
            url::Url::from_file_path(&image.0).unwrap()
        ),
        1,
    );
    let (window, view) = super::open(cx, source.clone());
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        let list = viewer.document.text.read(cx).list_state();
        assert!(list.item_count() > 100);
        assert!(list.bounds_for_item(1).unwrap().size.height > px(360.));
        assert!(list.bounds_for_item(2).unwrap().size.height > px(360.));
    });
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.document.text.update(cx, |text, cx| {
                text.list_state().scroll_to(gpui_kit::ListOffset {
                    item_ix: 3,
                    offset_in_item: px(0.),
                });
                cx.notify();
            });
        });
    });
    frame(cx, window);
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        let list = viewer.document.text.read(cx).list_state();
        // Intrinsic image height plus TextView's 12px paragraph spacing.
        // The unloaded image block is only 12px high.
        assert_eq!(list.bounds_for_item(3).unwrap().size.height, px(192.));
    });
    scroll(cx, &view, 2500.);
    frame(cx, window);
    // Keep the decoded local image in the replacement too, with different
    // preceding block boundaries; a logical block index cannot restore pixels.
    let replacement = source.replacen(
        &"A long wrapped paragraph with 日本語 text. ".repeat(200),
        &"Replacement paragraph.\n\n".repeat(100),
        1,
    );
    replace(cx, &view, replacement, false);
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 2500.);
        assert!(viewer.app.position_request().is_none());
    });
    replace(cx, &view, "Short paragraph.\n\n".repeat(20), false);
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        let list = viewer.document.text.read(cx).list_state();
        let max = list.max_offset_for_scrollbar().y;
        assert!(max > px(0.) && max < px(2500.));
        assert_eq!(viewer.document.distance(cx), f32::from(max));
        assert_eq!(list.is_scrolled_to_end(), Some(true));
        assert!(viewer.app.position_request().is_none());
    });
    replace(cx, &view, "Tiny".into(), false);
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 0.)
    });
}

#[gpui_kit::test]
fn short_and_empty_documents_consume_initial_and_reload_positions(cx: &mut TestAppContext) {
    for source in ["Tiny", ""] {
        let (window, view) = super::open(cx, source.into());
        frame(cx, window);
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.document.distance(cx), 0.);
            assert!(
                viewer.app.position_request().is_none(),
                "initial {source:?}"
            );
        });
        frame(cx, window);
        view.read_with(cx, |viewer, _| {
            assert!(viewer.app.position_request().is_none())
        });

        replace(cx, &view, "Long paragraph.\n\n".repeat(100), false);
        frame(cx, window);
        scroll(cx, &view, 480.);
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.document.distance(cx), 480.)
        });
        replace(cx, &view, source.into(), false);
        frame(cx, window);
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.document.distance(cx), 0.);
            assert!(viewer.app.position_request().is_none(), "reload {source:?}");
        });
        frame(cx, window);
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.document.distance(cx), 0.);
            assert!(viewer.app.position_request().is_none());
        });
    }
}

#[gpui_kit::test]
fn failed_reload_keeps_distance_and_new_document_opens_at_top(cx: &mut TestAppContext) {
    let (window, view) = super::open(cx, fixture());
    frame(cx, window);
    scroll(cx, &view, 480.);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let text = viewer.document.text.clone();
            let OperationResult::Pending(request) = viewer.request_operation(
                DocumentOperation::Reload,
                OperationOrigin::External,
                None,
                cx,
            ) else {
                panic!("read required")
            };
            viewer.complete(
                crate::app::ReadResult {
                    request,
                    content: Err("missing".into()),
                },
                cx,
            );
            assert_eq!(viewer.document.text, text);
            assert!(viewer.app.position_request().is_none());
        });
    });
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 480.)
    });
    replace(cx, &view, fixture(), true);
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 0.);
        assert!(viewer.app.position_request().is_none());
    });
}

#[gpui_kit::test]
fn delayed_layout_rejects_old_generation_entity_and_same_generation_serial(
    cx: &mut TestAppContext,
) {
    let source = "Paragraph.\n\n".repeat(200);
    let (window, view) = super::open(cx, source.clone());
    frame(cx, window);
    scroll(cx, &view, 480.);
    let (stale, text) = cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let request = viewer.app.open(viewer.path.clone());
            viewer.complete(super::success(request, source.clone()), cx);
            let old_request = viewer.app.position_request().unwrap().clone();
            let old_text = viewer.document.text.clone();
            // Switch before parsing/layout. Identical source cannot identify a generation.
            let request = viewer.app.open(viewer.path.with_file_name("other.md"));
            viewer.complete(super::success(request, source), cx);
            let current = viewer.app.position_request().unwrap().clone();
            assert_ne!(old_request.generation, current.generation);
            viewer.apply_position_after_layout(&old_request, &old_text, cx);
            assert_eq!(viewer.app.position_request(), Some(&current));
            (current, viewer.document.text.clone())
        })
    });
    frame(cx, window);
    scroll(cx, &view, 240.);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            // A newer synchronous heading move has the same generation but a
            // distinct serial. The old request must neither apply nor consume it.
            viewer.request_operation(
                DocumentOperation::Heading {
                    path: None,
                    id: "two".into(),
                },
                OperationOrigin::External,
                None,
                cx,
            );
            let current = viewer.app.position_request().unwrap().clone();
            assert_eq!(stale.generation, current.generation);
            assert_ne!(stale.serial, current.serial);
            viewer.apply_position_after_layout(&stale, &text, cx);
            assert_eq!(viewer.app.position_request(), Some(&current));
        });
    });
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        // The current (unknown) heading, not the stale distance, is applied.
        assert_eq!(viewer.document.distance(cx), 0.);
        assert!(viewer.app.position_request().is_none());
        assert_eq!(viewer.message.as_deref(), Some("Heading not found: two"));
    });
}

#[gpui_kit::test]
fn applied_restore_does_not_repeat_after_user_scroll_or_diagram_completion(
    cx: &mut TestAppContext,
) {
    let source = "Paragraph.\n\n".repeat(100) + "```mermaid\ngraph TD\nA --> B\n```\n\n";
    let (window, view) = super::open(cx, source.clone());
    frame(cx, window);
    let old_diagram = view.read_with(cx, |viewer, _| super::running_diagram(viewer));
    scroll(cx, &view, 480.);
    replace(cx, &view, source, false);
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 480.);
        assert!(viewer.app.position_request().is_none());
    });
    scroll(cx, &view, 720.);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            // Release the old generation's worker, then complete the adopted diagram.
            viewer.complete_diagram(
                crate::app::DiagramResult {
                    request: old_diagram,
                    png: Err("superseded".into()),
                },
                cx,
            );
            let request = super::running_diagram(viewer);
            let png = vec![
                137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 20, 0, 0, 0,
                10, 8, 6, 0, 0, 0, 180, 85, 126, 230, 0, 0, 0, 1, 115, 82, 71, 66, 0, 174, 206, 28,
                233, 0, 0, 0, 4, 103, 65, 77, 65, 0, 0, 177, 143, 11, 252, 97, 5, 0, 0, 0, 9, 112,
                72, 89, 115, 0, 0, 14, 195, 0, 0, 14, 195, 1, 199, 111, 168, 100, 0, 0, 0, 16, 73,
                68, 65, 84, 56, 79, 99, 96, 24, 5, 163, 96, 20, 16, 6, 0, 3, 42, 0, 1, 148, 87, 35,
                248, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
            ];
            viewer.complete_diagram(
                crate::app::DiagramResult {
                    request,
                    png: Ok(png),
                },
                cx,
            );
        });
    });
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 720.)
    });
}

use crate::app::{
    DocumentOperation, ExternalRequestId, Mode, OperationError, OperationOrigin, OperationResult,
    PositionTarget, ReadRequest,
};

#[gpui_kit::test]
fn reload_passes_the_live_pixel_distance_to_the_app_contract(cx: &mut TestAppContext) {
    let source = "Long paragraph for scrolling.\n\n".repeat(100);
    let (window, view) = super::open(cx, source.clone());
    cx.run_until_parked();
    let requests = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = requests.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone());
        })
    });
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        view.update(cx, |viewer, cx| {
            viewer.document.text.update(cx, |text, cx| {
                text.list_state()
                    .set_offset_from_scrollbar(point(px(0.), px(-240.)));
                cx.notify();
            });
        });
        window.press("f5", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let request = requests
        .borrow_mut()
        .pop()
        .expect("F5 reads the current file");
    assert_eq!(request.origin, OperationOrigin::Keyboard);
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |viewer, cx| {
            viewer.document.text.update(cx, |text, cx| {
                text.list_state()
                    .set_offset_from_scrollbar(point(px(0.), px(-480.)));
                cx.notify();
            });
            assert_eq!(viewer.document.distance(cx), 480.);
            viewer.complete(super::success(request, source), cx);
            assert_eq!(
                viewer.app.position_request().unwrap().target,
                PositionTarget::Distance(480.)
            );
        });
        window.render_frame(cx);
    })
    .unwrap();
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 480.);
        assert!(viewer.app.position_request().is_none());
    });
}

#[gpui_kit::test]
fn consecutive_automatic_completions_before_layout_keep_the_departure_distance(
    cx: &mut TestAppContext,
) {
    let (window, view) = super::open(cx, fixture());
    frame(cx, window);
    scroll(cx, &view, 480.);
    let requests = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = requests.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone())
        })
    });
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let target = viewer.app.watch_target().unwrap().clone();
            viewer.watch_changed(&target, cx);
            viewer.watch_changed(&target, cx);
        })
    });
    let first = requests.borrow_mut().pop().unwrap();
    // No frame is allowed between the two adopted completions.
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.complete(super::success(first, fixture()), cx)
        })
    });
    let trailing = requests.borrow_mut().pop().unwrap();
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            assert_eq!(viewer.document.distance(cx), 0.);
            viewer.complete(super::success(trailing, fixture()), cx);
            assert_eq!(
                viewer.app.position_request().unwrap().target,
                PositionTarget::Distance(480.)
            );
        })
    });
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 480.)
    });
    // Once laid out, actual user movement, rather than the previous request, wins.
    scroll(cx, &view, 640.);
    replace(cx, &view, fixture(), false);
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 640.)
    });
}

#[gpui_kit::test]
fn production_watch_connection_catches_up_a_save_before_registration(cx: &mut TestAppContext) {
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    let path = std::env::temp_dir().join(format!("mmv-watch-catch-up-{}.md", std::process::id()));
    std::fs::write(&path, "old body").unwrap();
    let (window, view) = super::open(cx, "previous body".into());
    frame(cx, window);
    let stale_result = cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let OperationResult::Pending(request) = viewer.request_operation(
                DocumentOperation::Initial(path.clone()),
                OperationOrigin::External,
                None,
                cx,
            ) else {
                panic!("read required")
            };
            crate::infra::read_document(request)
        })
    });
    // This save predates watcher registration and produces no watched OS event.
    std::fs::write(&path, "new body saved before registration").unwrap();
    let connection = cx.update(|cx| crate::runtime::connect_watch(&view, cx));
    cx.update(|cx| crate::runtime::connect_document_reads(&view, cx));
    let (done, completed) = mpsc::channel();
    let _completion = cx.update(|cx| {
        cx.subscribe(
            &view,
            move |_, completion: &crate::app::ReadCompletion, _| {
                done.send(completion.clone()).unwrap();
            },
        )
    });
    cx.update(|cx| view.update(cx, |viewer, cx| viewer.complete(stale_result, cx)));
    assert!(completed.try_recv().unwrap().adopted);
    let deadline = Instant::now() + Duration::from_secs(5);
    let caught_up = loop {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        if let Ok(completion) = completed.recv_timeout(Duration::from_millis(10)) {
            break completion.adopted;
        }
        if Instant::now() >= deadline {
            break false;
        }
    };
    view.read_with(cx, |viewer, _| {
        assert_eq!(
            viewer.app.document().unwrap().source.as_str(),
            "new body saved before registration"
        )
    });
    assert!(caught_up);
    frame(cx, window);
    let stable_until = Instant::now() + Duration::from_millis(150);
    while Instant::now() < stable_until {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        assert!(matches!(
            completed.recv_timeout(Duration::from_millis(10)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
    }
    // Closing the actual window and releasing its last Entity invokes the
    // production lifecycle callback, not a test-only stop method.
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    cx.update(|_| drop(view));
    cx.run_until_parked();
    let join = {
        let mut connection = connection.borrow_mut();
        assert!(connection.is_stopped());
        connection
            .take_joining()
            .expect("release must start worker join")
    };
    let (joined, worker_stopped) = mpsc::channel();
    cx.update(|cx| {
        cx.spawn(async move |_| {
            join.await;
            joined.send(()).unwrap();
        })
        .detach()
    });
    cx.run_until_parked();
    worker_stopped.recv_timeout(Duration::from_secs(5)).unwrap();
    std::fs::remove_file(path).unwrap();
}

#[gpui_kit::test]
fn real_notify_save_and_same_file_source_line_preserve_both_arrival_orders(
    cx: &mut TestAppContext,
) {
    use crate::app::{ExternalOperation, PositionTarget};
    use std::time::Duration;

    struct SavedFile(PathBuf);
    impl Drop for SavedFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    // Real file, canonical identity, notify worker, app and TextView. Only
    // dispatch/read-result delivery is controlled to make the order explicit.
    for save_first in [true, false] {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let file = SavedFile(std::env::temp_dir().join(format!(
            "mmv-source-line-save-{}-{}.md",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        let old_source = (0..240)
            .map(|ix| format!("old paragraph {ix}\n\n"))
            .collect::<String>();
        let new_source = (0..240)
            .map(|ix| format!("new paragraph {ix}\n\n"))
            .collect::<String>();
        std::fs::write(&file.0, &old_source).unwrap();
        let (window, view) = super::open(cx, String::new());
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                let OperationResult::Pending(request) = viewer.request_operation(
                    DocumentOperation::Initial(file.0.clone()),
                    OperationOrigin::External,
                    None,
                    cx,
                ) else {
                    panic!("initial read");
                };
                viewer.complete(crate::infra::read_document(request), cx);
            })
        });
        frame(cx, window);
        let target = view.read_with(cx, |viewer, _| viewer.app.watch_target().unwrap().clone());
        let (watch, notices) = crate::infra::FileWatch::new();
        watch.set_target(target.clone());
        let crate::infra::WatchNotice::Ready(actual, ready) =
            notices.recv_timeout(Duration::from_secs(5)).unwrap()
        else {
            panic!("watch ready");
        };
        assert_eq!(actual, target);
        ready.unwrap();
        let requests = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
        let captured = requests.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&view, move |_, request: &ReadRequest, _| {
                captured.borrow_mut().push(request.clone())
            })
        });
        let open_line = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                view.update(cx, |viewer, cx| {
                    let generation = viewer.app.document().unwrap().generation;
                    let text = viewer.document.text.clone();
                    let OperationResult::Pending(request) = viewer.request_external(
                        ExternalOperation::Open {
                            path: file.0.clone(),
                            line: std::num::NonZeroU64::new(241),
                        },
                        ExternalRequestId("save-line".into()),
                        cx,
                    ) else {
                        panic!("identity check");
                    };
                    viewer.complete(crate::infra::read_document(request), cx);
                    assert_eq!(viewer.app.document().unwrap().generation, generation);
                    assert_eq!(viewer.document.text, text);
                })
            });
        };
        if !save_first {
            open_line(cx);
            for _ in 0..6 {
                frame(cx, window);
            }
        }
        std::fs::write(&file.0, &new_source).unwrap();
        let crate::infra::WatchNotice::Changed(actual) =
            notices.recv_timeout(Duration::from_secs(5)).unwrap()
        else {
            panic!("actual save notification");
        };
        cx.update(|cx| view.update(cx, |viewer, cx| viewer.watch_changed(&actual, cx)));
        let reload = {
            let mut requests = requests.borrow_mut();
            let ix = requests
                .iter()
                .position(|request| request.origin == OperationOrigin::Automatic)
                .expect("save must start reload");
            requests.remove(ix)
        };
        let result = crate::infra::read_document(reload);
        if save_first {
            open_line(cx);
            view.read_with(cx, |viewer, _| {
                assert!(
                    viewer.app.is_loading(),
                    "line confirmation must not cancel saved body"
                )
            });
        }
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                viewer.complete(result, cx);
                assert_eq!(viewer.app.document().unwrap().source.as_str(), new_source);
                assert_eq!(
                    viewer.app.position_request().unwrap().target,
                    PositionTarget::SourceLine(std::num::NonZeroU64::new(241).unwrap())
                );
                assert_eq!(viewer.app.watch_target(), Some(&target));
            })
        });
        for _ in 0..8 {
            frame(cx, window);
        }
        view.read_with(cx, |viewer, cx| {
            let list = viewer.document.text.read(cx).list_state();
            // Paragraph 120 is source line 241 (one blank line per paragraph).
            let bounds = list.bounds_for_item(120).unwrap();
            let viewport = list.viewport_bounds();
            assert!(
                bounds.bottom() > viewport.top() && bounds.top() < viewport.bottom(),
                "save_first={save_first}: {bounds:?} / {viewport:?}"
            );
            assert!(viewer.app.position_request().is_none());
        });
        let worker = watch.stop();
        let (done, stopped) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            worker.join().unwrap();
            let _ = done.send(());
        });
        stopped.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}

#[gpui_kit::test]
fn real_save_uses_reload_distance_and_preserves_search_command_input(cx: &mut TestAppContext) {
    use std::time::Duration;
    let dir = std::env::temp_dir().join(format!("mmv-ui-watch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("document.md");
    std::fs::write(&path, fixture()).unwrap();
    let (window, view) = super::open(cx, fixture());
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let OperationResult::Pending(request) = viewer.request_operation(
                DocumentOperation::Initial(path.clone()),
                OperationOrigin::External,
                None,
                cx,
            ) else {
                panic!("read required")
            };
            viewer.complete(crate::infra::read_document(request), cx);
        })
    });
    frame(cx, window);
    let target = view.read_with(cx, |viewer, _| viewer.app.watch_target().unwrap().clone());
    let (watch, notices) = crate::infra::FileWatch::new();
    watch.set_target(target.clone());
    match notices.recv_timeout(Duration::from_secs(5)).unwrap() {
        crate::infra::WatchNotice::Ready(actual, result) => {
            assert_eq!(actual, target);
            result.unwrap();
        }
        _ => panic!("expected readiness"),
    }
    let requests = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = requests.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone());
        })
    });
    for (key, mode) in [("/", Mode::Search), (":", Mode::Command)] {
        cx.update_window(window, |_, window, cx| {
            window.press(key, cx);
            window.input("edit", cx);
        })
        .unwrap();
        frame(cx, window);
        let input = view.read_with(cx, |viewer, _| viewer.input.state.clone());
        scroll(cx, &view, 480.);
        std::fs::write(
            &path,
            "new edit target\n\n".to_owned() + &"Paragraph.\n\n".repeat(200),
        )
        .unwrap();
        let crate::infra::WatchNotice::Changed(actual) =
            notices.recv_timeout(Duration::from_secs(5)).unwrap()
        else {
            panic!("expected change")
        };
        cx.update(|cx| view.update(cx, |viewer, cx| viewer.watch_changed(&actual, cx)));
        let request = requests.borrow_mut().pop().unwrap();
        assert_eq!(request.origin, OperationOrigin::Automatic);
        // Reading is completed explicitly so the replacement-time distance is deterministic.
        let result = crate::infra::read_document(request);
        scroll(cx, &view, 640.);
        cx.update(|cx| view.update(cx, |viewer, cx| viewer.complete(result, cx)));
        frame(cx, window);
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.app.mode, mode);
            assert_eq!(viewer.input.state, input);
            assert_eq!(viewer.input.state.read(cx).value().as_ref(), "edit");
            assert_eq!(viewer.document.distance(cx), 640.);
            assert_eq!(viewer.app.watch_target(), Some(&target));
            if mode == Mode::Search {
                assert_eq!(viewer.app.matches().len(), 1);
                assert_eq!(viewer.app.matches()[0], 4..8);
            }
        });
        cx.update_window(window, |_, window, cx| window.input("x", cx))
            .unwrap();
        frame(cx, window);
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.input.state.read(cx).value().as_ref(), "editx")
        });
        cx.update_window(window, |_, window, cx| window.press("escape", cx))
            .unwrap();
        frame(cx, window);
    }
    // F5 shares the same replacement-time distance policy as the save above.
    scroll(cx, &view, 640.);
    cx.update_window(window, |_, window, cx| window.press("f5", cx))
        .unwrap();
    let request = requests.borrow_mut().pop().unwrap();
    assert_eq!(request.origin, OperationOrigin::Keyboard);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.complete(crate::infra::read_document(request), cx)
        })
    });
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 640.)
    });
    // A short saved document clamps automatic restoration to its actual end.
    std::fs::write(&path, "Tiny").unwrap();
    let crate::infra::WatchNotice::Changed(actual) =
        notices.recv_timeout(Duration::from_secs(5)).unwrap()
    else {
        panic!("expected change")
    };
    cx.update(|cx| view.update(cx, |viewer, cx| viewer.watch_changed(&actual, cx)));
    let request = requests.borrow_mut().pop().unwrap();
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.complete(crate::infra::read_document(request), cx)
        })
    });
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), 0.)
    });
    let worker = watch.stop();
    let (done, stopped) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        worker.join().unwrap();
        done.send(()).unwrap();
    });
    stopped.recv_timeout(Duration::from_secs(5)).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[gpui_kit::test]
fn external_replacement_keeps_search_and_command_input_entity_text_and_focus(
    cx: &mut TestAppContext,
) {
    let (window, view) = super::open(cx, fixture());
    cx.run_until_parked();
    let requests = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = requests.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone());
        })
    });
    for (key, mode) in [("/", Mode::Search), (":", Mode::Command)] {
        cx.update_window(window, |_, window, cx| {
            window.press(key, cx);
            window.input("edit", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let input = view.read_with(cx, |viewer, _| viewer.input.state.clone());
        scroll(cx, &view, 480.);
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                let result = viewer.request_operation(
                    DocumentOperation::Reload,
                    OperationOrigin::External,
                    Some(ExternalRequestId("open".into())),
                    cx,
                );
                let OperationResult::Pending(request) = result else {
                    panic!("open must return a pending read");
                };
                assert_eq!(request.response_to, Some(ExternalRequestId("open".into())));
            })
        });
        let request = requests.borrow_mut().pop().unwrap();
        assert_eq!(request.origin, OperationOrigin::External);
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                viewer.complete(
                    super::success(
                        request,
                        "new edit target\n\n".to_owned() + &"Paragraph.\n\n".repeat(200),
                    ),
                    cx,
                );
            })
        });
        cx.run_until_parked();
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.app.mode, mode);
            assert_eq!(viewer.input.state, input);
            assert_eq!(viewer.input.state.read(cx).value().as_ref(), "edit");
            assert_eq!(viewer.document.distance(cx), 480.);
            assert!(viewer.app.position_request().is_none());
            if mode == Mode::Search {
                assert_eq!(viewer.app.matches().len(), 1);
                assert_eq!(viewer.app.matches()[0], 4..8);
            }
        });
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                let path = viewer.path.with_file_name(if mode == Mode::Search {
                    "search-switch.md"
                } else {
                    "command-switch.md"
                });
                let OperationResult::Pending(request) = viewer.request_document(path, None, cx)
                else {
                    panic!("read required");
                };
                viewer.complete(
                    super::success(
                        request,
                        "new edit target\n\n".to_owned() + &"Paragraph.\n\n".repeat(200),
                    ),
                    cx,
                );
            });
        });
        assert_eq!(requests.borrow_mut().drain(..).count(), 1);
        frame(cx, window);
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.document.distance(cx), 0.);
            assert_eq!(viewer.app.mode, mode);
            assert_eq!(viewer.input.state, input);
            assert_eq!(viewer.input.state.read(cx).value().as_ref(), "edit");
            if mode == Mode::Search {
                assert_eq!(viewer.app.matches().len(), 1);
                assert_eq!(viewer.app.matches()[0], 4..8);
            }
        });
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                let before = viewer.app.position_request().cloned();
                let failure_id = ExternalRequestId("forward-unavailable".into());
                assert_eq!(
                    viewer.request_operation(
                        DocumentOperation::Forward,
                        OperationOrigin::External,
                        Some(failure_id.clone()),
                        cx,
                    ),
                    OperationResult::Complete {
                        response_to: Some(failure_id),
                        outcome: Err(OperationError::NoNextEntry),
                    }
                );
                assert_eq!(viewer.app.position_request(), before.as_ref());
                let success_id = ExternalRequestId("heading-success".into());
                assert_eq!(
                    viewer.request_operation(
                        DocumentOperation::Heading {
                            path: None,
                            id: "target".into()
                        },
                        OperationOrigin::External,
                        Some(success_id.clone()),
                        cx,
                    ),
                    OperationResult::Complete {
                        response_to: Some(success_id),
                        outcome: Ok(())
                    }
                );
                assert_eq!(viewer.app.mode, mode);
                assert_eq!(viewer.input.state, input);
                assert_eq!(viewer.input.state.read(cx).value().as_ref(), "edit");
            });
        });
        cx.run_until_parked();
        assert!(requests.borrow().is_empty());
        cx.update_window(window, |_, window, cx| {
            window.input("x", cx);
        })
        .unwrap();
        cx.run_until_parked();
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.input.state.read(cx).value().as_ref(), "editx");
            assert_eq!(viewer.app.mode, mode);
        });
        cx.update_window(window, |_, window, cx| window.press("escape", cx))
            .unwrap();
        cx.run_until_parked();
    }
}
