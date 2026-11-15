//! Real file I/O -> parser -> prepared delivery -> app/view, on GPUI's headless platform.
use super::Viewer;
use crate::{
    app::{
        self, ExternalOperation, ExternalRequestId, Mode, OperationResult, ReadCompletion,
        ReadRequest,
    },
    infra, pdf, runtime,
};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, EntityInputHandler as _, Focusable as _,
    TestAppContext, VisualTestContext, test::TestWindowExt as _,
};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mmv-pdf-ui-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }
    fn pdf(&self, name: &str) -> PathBuf {
        self.write(name, &pdf::tests::fixture("3 0 R 3 0 R", 2, false))
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
    cx.run_until_parked();
}
fn press(cx: &mut TestAppContext, window: AnyWindowHandle, key: &str) {
    cx.update_window(window, |_, window, cx| window.press(key, cx))
        .unwrap();
    frame(cx, window);
}
fn input(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    cx.update_window(window, |_, window, cx| window.input(text, cx))
        .unwrap();
    frame(cx, window);
}
fn request(cx: &mut TestAppContext, view: &Entity<Viewer>, path: &Path, id: &str) -> ReadRequest {
    let result = cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.request_external(
                ExternalOperation::Open {
                    path: path.into(),
                    line: None,
                },
                ExternalRequestId(id.into()),
                cx,
            )
        })
    });
    let OperationResult::Pending(request) = result else {
        panic!("pending request");
    };
    request
}
fn body_pdf(cx: &TestAppContext, view: &Entity<Viewer>) -> Entity<pdf::PdfView> {
    view.read_with(cx, |view, _| match &view.document {
        crate::ui::document::DocumentView::Pdf { view, .. } => view.clone(),
        _ => panic!("expected actual PDF view"),
    })
}

#[gpui_kit::test]
fn runtime_prepared_adoption_moves_once_same_open_does_not_read_parse_or_reset(
    cx: &mut TestAppContext,
) {
    let files = Files::new();
    let path = files.pdf("valid.PDF");
    let markdown = files.write("return.md", b"Markdown return");
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    let registry = cx.update(|cx| runtime::connect_document_reads(&view, cx));
    let completions = Rc::new(RefCell::new(Vec::<ReadCompletion>::new()));
    let captured = completions.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |view, event: &ReadCompletion, cx| {
            if event.adopted {
                assert!(
                    matches!(
                        view.read(cx).document,
                        crate::ui::document::DocumentView::Pdf { .. }
                    ) || view.read(cx).document.as_markdown().is_some()
                );
            }
            captured.borrow_mut().push(event.clone());
        })
    });
    let before = pdf::prepare_calls();
    request(cx, &view, &path, "first");
    assert!(completions.borrow().is_empty());
    frame(cx, window);
    assert_eq!(pdf::prepare_calls(), before + 1);
    let first = body_pdf(cx, &view);
    let generation = view.read_with(cx, |view, _| {
        assert!(view.app.watch_target().is_none());
        assert_eq!(
            view.app.document().unwrap().content.format(),
            app::DocumentFormat::Pdf
        );
        view.app.document().unwrap().generation
    });
    first.update(cx, |view, cx| {
        view.set_zoom(1.7, cx).unwrap();
    });
    frame(cx, window);
    std::fs::write(&path, b"now invalid; same open must not read this").unwrap();
    request(cx, &view, &path, "same");
    frame(cx, window);
    assert_eq!(pdf::prepare_calls(), before + 1);
    assert_eq!(body_pdf(cx, &view), first);
    assert_eq!(
        view.read_with(cx, |view, _| view.app.document().unwrap().generation),
        generation
    );
    assert!((first.read_with(cx, |view, cx| view.zoom(cx)) - 1.7).abs() < 0.0001);
    assert_eq!(completions.borrow().len(), 2);
    assert!(!completions.borrow()[1].adopted);
    press(cx, window, "f5");
    assert_eq!(pdf::prepare_calls(), before + 1);
    request(cx, &view, &markdown, "markdown");
    frame(cx, window);
    assert!(first.read_with(cx, |view, cx| view.is_released(cx)));
    assert!(view.read_with(cx, |view, _| view.document.as_markdown().is_some()));
    assert_eq!(registry.borrow().pending_count(), 0);
}

#[gpui_kit::test]
fn failed_parser_and_same_pdf_line_rejection_keep_live_view_and_search_draft(
    cx: &mut TestAppContext,
) {
    let files = Files::new();
    let path = files.pdf("valid.pdf");
    let invalid = files.write("invalid.pdf", b"not pdf");
    let locked = files.write("locked.pdf", &pdf::tests::fixture("3 0 R", 1, true));
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    cx.update(|cx| runtime::connect_document_reads(&view, cx));
    request(cx, &view, &path, "valid");
    frame(cx, window);
    let original = body_pdf(cx, &view);
    let generation = view.read_with(cx, |view, _| view.app.document().unwrap().generation);
    let before = pdf::prepare_calls();
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            let pending = view.request_external(
                ExternalOperation::Open {
                    path: path.clone(),
                    line: std::num::NonZeroU64::new(2),
                },
                ExternalRequestId("line".into()),
                cx,
            );
            assert!(matches!(pending, OperationResult::Pending(_)));
        })
    });
    frame(cx, window);
    assert_eq!(pdf::prepare_calls(), before);
    assert_eq!(
        view.read_with(cx, |view, _| view.app.error().map(str::to_owned)),
        Some("Source line navigation is not supported for PDF".into())
    );
    for target in [invalid, locked] {
        request(cx, &view, &target, "failed");
        frame(cx, window);
        assert_eq!(body_pdf(cx, &view), original);
        assert_eq!(
            view.read_with(cx, |view, _| view.app.document().unwrap().generation),
            generation
        );
        assert!(!original.read_with(cx, |view, cx| view.is_released(cx)));
        assert!(view.read_with(cx, |view, _| view.app.error().is_some()));
    }
    for key in ["/", "n", "shift-n"] {
        press(cx, window, key);
        view.read_with(cx, |view, _| {
            assert_eq!(view.app.mode, Mode::Normal);
            assert_eq!(
                view.message.as_deref(),
                Some("Search is not supported for PDF yet")
            );
            assert!(view.app.matches().is_empty());
        });
    }
}

#[gpui_kit::test]
fn pdf_search_keeps_draft_or_confirmed_query_and_obeys_existing_ime_guard(cx: &mut TestAppContext) {
    let files = Files::new();
    let path = files.pdf("query.pdf");
    let markdown = files.write("query.md", b"A B A");
    for confirm_pdf in [false, true] {
        let (window, view) = super::open(cx, "A B A".into());
        frame(cx, window);
        cx.update(|cx| runtime::connect_document_reads(&view, cx));
        press(cx, window, "/");
        input(cx, window, "A");
        press(cx, window, "enter");
        press(cx, window, "/");
        if !confirm_pdf {
            input(cx, window, "B");
        }
        let old_generation = view.read_with(cx, |view, _| view.app.document().unwrap().generation);
        request(cx, &view, &path, "switch-in-search");
        frame(cx, window);
        view.read_with(cx, |view, _| {
            assert_eq!(view.app.mode, Mode::Search);
            assert!(view.app.matches().is_empty());
        });
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                view.accept_rendered_text(old_generation, cx);
                assert!(!view.app.rendered(old_generation, "stale A B A"));
                view.apply_search(true, cx);
            })
        });
        let prompt = view.read_with(cx, |view, _| view.input.state.clone());
        if confirm_pdf {
            cx.update_window(window, |_, window, cx| {
                prompt.update(cx, |input, cx| {
                    input.replace_and_mark_text_in_range(None, "B", Some(1..1), window, cx);
                })
            })
            .unwrap();
            frame(cx, window);
            press(cx, window, "enter");
            assert_eq!(
                view.read_with(cx, |view, _| view.app.mode.clone()),
                Mode::Search
            );
            cx.update_window(window, |_, window, cx| {
                prompt.update(cx, |input, cx| input.unmark_text(window, cx))
            })
            .unwrap();
            press(cx, window, "enter");
            view.read_with(cx, |view, _| {
                assert_eq!(view.app.mode, Mode::Normal);
                assert_eq!(
                    view.message.as_deref(),
                    Some("Search is not supported for PDF yet")
                );
            });
        } else {
            let mut visual = VisualTestContext::from_window(window, cx);
            let body = visual.debug_bounds("document-body").unwrap();
            visual.simulate_click(body.center(), Default::default());
            frame(cx, window);
            cx.update_window(window, |_, window, cx| {
                assert!(prompt.focus_handle(cx).is_focused(window))
            })
            .unwrap();
        }
        request(cx, &view, &markdown, "return-markdown");
        frame(cx, window);
        view.read_with(cx, |view, _| {
            assert_eq!(
                view.app.matches(),
                if confirm_pdf {
                    vec![0..1, 4..5]
                } else {
                    vec![2..3]
                }
            );
            assert_eq!(
                view.app.mode,
                if confirm_pdf {
                    Mode::Normal
                } else {
                    Mode::Search
                }
            );
            assert!(view.display_error.is_none());
        });
    }
}

#[gpui_kit::test]
fn pdf_history_recreates_top_fit_but_failed_switch_does_not_release(cx: &mut TestAppContext) {
    let files = Files::new();
    let path = files.pdf("history.pdf");
    let markdown = files.write("history.md", b"real Markdown");
    let (window, view) = super::open(cx, "initial".into());
    frame(cx, window);
    cx.update(|cx| runtime::connect_document_reads(&view, cx));
    request(cx, &view, &markdown, "md");
    frame(cx, window);
    request(cx, &view, &path, "pdf");
    frame(cx, window);
    let old = body_pdf(cx, &view);
    old.update(cx, |view, cx| {
        view.set_zoom(1.7, cx).unwrap();
        view.scroll_to_edge(true, cx).unwrap();
    });
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.request_external(
                ExternalOperation::Back,
                ExternalRequestId("back".into()),
                cx,
            );
        })
    });
    frame(cx, window);
    assert!(old.read_with(cx, |view, cx| view.is_released(cx)));
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.request_external(
                ExternalOperation::Forward,
                ExternalRequestId("forward".into()),
                cx,
            );
        })
    });
    frame(cx, window);
    let fresh = body_pdf(cx, &view);
    assert_ne!(old, fresh);
    assert!((fresh.read_with(cx, |view, cx| view.zoom(cx)) - 1.7).abs() > 0.01);
    assert!(view.read_with(cx, |view, _| view.app.position_request().is_none()));
}

#[gpui_kit::test]
fn real_prepared_reverse_delivery_duplicate_and_old_phase_keep_current_pdf(
    cx: &mut TestAppContext,
) {
    let files = Files::new();
    let a = files.pdf("a.pdf");
    let b = files.pdf("b.pdf");
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    let reads = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = reads.clone();
    let completions = Rc::new(RefCell::new(Vec::<ReadCompletion>::new()));
    let completed = completions.clone();
    let (_requests, _completions) = cx.update(|cx| {
        (
            cx.subscribe(&view, move |_, request: &ReadRequest, _| {
                captured.borrow_mut().push(request.clone())
            }),
            cx.subscribe(&view, move |view, event: &ReadCompletion, cx| {
                if event.adopted {
                    assert!(matches!(
                        view.read(cx).document,
                        crate::ui::document::DocumentView::Pdf { .. }
                    ));
                }
                completed.borrow_mut().push(event.clone());
            }),
        )
    });
    let resolve_a = request(cx, &view, &a, "A");
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.complete(runtime::test_read(resolve_a.clone()), cx)
        })
    });
    let load_a = reads.borrow().last().unwrap().clone();
    let prepared_a = runtime::test_read(load_a.clone());
    let resolve_b = request(cx, &view, &b, "B");
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.complete(runtime::test_read(resolve_b), cx)
        })
    });
    let load_b = reads.borrow().last().unwrap().clone();
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.complete(
                app::ReadResponse::Resolved {
                    request: resolve_a,
                    key: Ok(app::DocumentKey(a.clone())),
                },
                cx,
            );
            assert_eq!(view.app.read_request_status(&load_a), Some(false));
            view.complete(runtime::test_read(load_b), cx);
            view.complete(prepared_a, cx);
            view.complete(
                app::ReadResult {
                    request: load_a,
                    content: Err("duplicate".into()),
                },
                cx,
            );
        })
    });
    frame(cx, window);
    assert_eq!(completions.borrow().len(), 2);
    assert!(completions.borrow()[0].adopted);
    assert!(!completions.borrow()[1].adopted);
    assert!(
        completions
            .borrow()
            .iter()
            .all(|event| event.outcome.is_ok())
    );
    assert_eq!(view.read_with(cx, |view, _| view.path.clone()), b);
}

#[gpui_kit::test]
fn internal_cancellation_collects_late_real_parse_without_completion(cx: &mut TestAppContext) {
    let files = Files::new();
    let a = files.pdf("internal-a.pdf");
    let b = files.pdf("external-b.pdf");
    let gate = runtime::pause_after_prepare(a.clone());
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    let registry = cx.update(|cx| runtime::connect_document_reads(&view, cx));
    let events = Rc::new(RefCell::new(Vec::<ReadCompletion>::new()));
    let captured = events.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, event: &ReadCompletion, _| {
            captured.borrow_mut().push(event.clone())
        })
    });
    let internal = cx.update(|cx| {
        view.update(cx, |view, cx| {
            let OperationResult::Pending(request) = view.request_operation(
                app::DocumentOperation::Open(a.clone()),
                app::OperationOrigin::Link,
                None,
                cx,
            ) else {
                panic!();
            };
            request
        })
    });
    cx.run_until_parked();
    assert!(gate.reached());
    request(cx, &view, &b, "external");
    frame(cx, window);
    gate.release();
    frame(cx, window);
    assert_eq!(events.borrow().len(), 1);
    assert_eq!(
        events.borrow()[0].response_to,
        Some(ExternalRequestId("external".into()))
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.app.read_record_counts()),
        (0, 0)
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.app.read_request_status(&internal)),
        None
    );
    assert_eq!(registry.borrow().pending_count(), 0);
    assert_eq!(view.read_with(cx, |view, _| view.path.clone()), b);
}

#[gpui_kit::test]
fn window_close_drains_maps_and_collects_noninterruptible_prepared_after_view_drop(
    cx: &mut TestAppContext,
) {
    let files = Files::new();
    let path = files.pdf("closing.pdf");
    let gate = runtime::pause_after_prepare(path.clone());
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    let registry = cx.update(|cx| runtime::connect_document_reads(&view, cx));
    // Closing this viewer must not depend on whether another headless window exists.
    let (_other_window, _other_view) = super::open(cx, "other window".into());
    let pending = request(cx, &view, &path, "closing");
    cx.run_until_parked();
    assert!(gate.reached());
    cx.update_window(window, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert!(registry.borrow().is_stopped());
    assert_eq!(
        view.read_with(cx, |view, _| view.app.read_record_counts()),
        (0, 0)
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.app.read_request_status(&pending)),
        None
    );
    let weak = view.downgrade();
    drop(view);
    cx.run_until_parked();
    gate.release();
    cx.run_until_parked();
    assert_eq!(registry.borrow().pending_count(), 0);
    assert!(weak.upgrade().is_none());
}

#[gpui_kit::test]
fn shutdown_collector_join_waits_for_real_noninterruptible_prepare(cx: &mut TestAppContext) {
    let files = Files::new();
    let path = files.pdf("join.pdf");
    let gate = runtime::pause_after_prepare(path.clone());
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    let registry = cx.update(|cx| runtime::connect_document_reads(&view, cx));
    request(cx, &view, &path, "join");
    frame(cx, window);
    assert!(gate.reached());
    // Exercise the exact collector transfer used by on_app_quit, without starting
    // a native application or replacing the actual parsed result with a stub.
    let jobs = registry.borrow_mut().stop_and_collect();
    assert_eq!(jobs.len(), 1);
    cx.update(|cx| view.update(cx, |view, cx| view.shutdown_document_reads(cx)));
    let joined = Rc::new(std::cell::Cell::new(false));
    let flag = joined.clone();
    cx.update(|cx| {
        cx.spawn(async move |_| {
            for job in jobs {
                job.await;
            }
            flag.set(true);
        })
        .detach()
    });
    cx.run_until_parked();
    assert!(
        !joined.get(),
        "shutdown cannot finish while the parser result is outstanding"
    );
    gate.release();
    cx.run_until_parked();
    assert!(joined.get());
    assert_eq!(registry.borrow().pending_count(), 0);
    assert_eq!(
        view.read_with(cx, |view, _| view.app.read_record_counts()),
        (0, 0)
    );
}

#[gpui_kit::test]
fn synchronous_heading_cancels_internal_parse_without_a_new_read_event(cx: &mut TestAppContext) {
    let files = Files::new();
    let path = files.pdf("synchronous-cancel.pdf");
    let gate = runtime::pause_after_prepare(path.clone());
    let (window, view) = super::open(cx, "# Target\n\nretained".into());
    frame(cx, window);
    let registry = cx.update(|cx| runtime::connect_document_reads(&view, cx));
    let events = Rc::new(RefCell::new(Vec::<ReadCompletion>::new()));
    let captured = events.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, event: &ReadCompletion, _| {
            captured.borrow_mut().push(event.clone())
        })
    });
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.request_operation(
                app::DocumentOperation::Open(path),
                app::OperationOrigin::Link,
                None,
                cx,
            );
        })
    });
    frame(cx, window);
    assert!(gate.reached());
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            assert!(matches!(
                view.request_operation(
                    app::DocumentOperation::Heading {
                        path: None,
                        id: "target".into()
                    },
                    app::OperationOrigin::Link,
                    None,
                    cx
                ),
                OperationResult::Complete {
                    outcome: Ok(()),
                    ..
                }
            ));
        })
    });
    gate.release();
    frame(cx, window);
    assert!(
        events.borrow().is_empty(),
        "internal cancellation emits no completion"
    );
    assert_eq!(
        view.read_with(cx, |view, _| view.app.read_record_counts()),
        (0, 0)
    );
    assert_eq!(registry.borrow().pending_count(), 0);
    assert!(view.read_with(cx, |view, _| view.document.as_markdown().is_some()));
}

#[gpui_kit::test]
fn pdf_body_click_respects_command_input_and_normal_host_focus(cx: &mut TestAppContext) {
    let files = Files::new();
    let path = files.pdf("focus.pdf");
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    cx.update(|cx| runtime::connect_document_reads(&view, cx));
    press(cx, window, ":");
    input(cx, window, "keep-command");
    let prompt = view.read_with(cx, |view, _| view.input.state.clone());
    request(cx, &view, &path, "command-switch");
    frame(cx, window);
    {
        let mut visual = VisualTestContext::from_window(window, cx);
        let bounds = visual.debug_bounds("document-body").unwrap();
        visual.simulate_click(bounds.center(), Default::default());
    }
    frame(cx, window);
    cx.update_window(window, |_, window, cx| {
        assert!(prompt.focus_handle(cx).is_focused(window));
        assert_eq!(prompt.read(cx).value().as_ref(), "keep-command");
        assert_eq!(view.read(cx).app.mode, Mode::Command);
    })
    .unwrap();
    press(cx, window, "escape");
    cx.update_window(window, |_, window, cx| {
        prompt.focus_handle(cx).focus(window, cx)
    })
    .unwrap();
    {
        let mut visual = VisualTestContext::from_window(window, cx);
        let bounds = visual.debug_bounds("document-body").unwrap();
        visual.simulate_click(bounds.center(), Default::default());
    }
    frame(cx, window);
    cx.update_window(window, |_, window, cx| {
        assert!(view.read(cx).body_focus.is_focused(window))
    })
    .unwrap();
}

#[gpui_kit::test]
fn runtime_ignores_unknown_altered_and_old_phase_read_events(cx: &mut TestAppContext) {
    let files = Files::new();
    let path = files.pdf("registered.pdf");
    let other = files.pdf("forged.pdf");
    let gate = runtime::pause_after_prepare(path.clone());
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    let registry = cx.update(|cx| runtime::connect_document_reads(&view, cx));
    let reads = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = reads.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone())
        })
    });
    let resolve = request(cx, &view, &path, "registered");
    frame(cx, window);
    assert!(gate.reached());
    let load = reads.borrow().last().unwrap().clone();
    assert_eq!(load.phase, app::ReadPhase::Load);
    let count = pdf::prepare_calls();
    let mut altered = load.clone();
    altered.path = other.clone();
    let unknown = app::App::default().open(other);
    cx.update(|cx| {
        view.update(cx, |_, cx| {
            cx.emit(resolve);
            cx.emit(altered);
            cx.emit(unknown);
        })
    });
    cx.run_until_parked();
    assert_eq!(registry.borrow().pending_count(), 1);
    assert_eq!(pdf::prepare_calls(), count);
    assert_eq!(
        view.read_with(cx, |view, _| view.app.read_request_status(&load)),
        Some(true)
    );
    gate.release();
    frame(cx, window);
    assert_eq!(view.read_with(cx, |view, _| view.path.clone()), path);
    assert_eq!(registry.borrow().pending_count(), 0);
}

fn pump_until(cx: &mut TestAppContext, mut ready: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !ready() {
        cx.run_until_parked();
        assert!(Instant::now() < end, "isolated PDF pipe stalled");
        cx.executor().advance_clock(Duration::from_millis(10));
        thread::sleep(Duration::from_millis(2));
    }
}
fn pipe_request(
    namespace: infra::Namespace,
    path: PathBuf,
) -> (
    thread::JoinHandle<()>,
    mpsc::Receiver<std::io::Result<infra::Response>>,
) {
    pipe_request_with_timeout(namespace, path, None)
}
fn pipe_request_with_timeout(
    namespace: infra::Namespace,
    path: PathBuf,
    timeout: Option<Duration>,
) -> (
    thread::JoinHandle<()>,
    mpsc::Receiver<std::io::Result<infra::Response>>,
) {
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let operation = crate::cli::parse_from(
            [
                std::ffi::OsString::from("mmv"),
                std::ffi::OsString::from("open"),
                path.into_os_string(),
            ],
            &std::env::current_dir().unwrap(),
        )
        .unwrap();
        let client =
            infra::PipeClient::connect(&namespace, infra::Deadline::connection(Instant::now()))
                .unwrap();
        let deadline = timeout.map_or_else(
            || infra::Deadline::response(Instant::now()),
            infra::test_support::deadline,
        );
        tx.send(client.request(
            infra::Request {
                id: ExternalRequestId("collision".into()),
                operation,
            },
            deadline,
        ))
        .unwrap();
    });
    (worker, rx)
}

#[gpui_kit::test]
fn disconnected_cli_keeps_accepted_parse_outcome_and_never_adopts_stale_pdf(
    cx: &mut TestAppContext,
) {
    let files = Files::new();
    let a = files.pdf("timeout-a.pdf");
    let b = files.pdf("timeout-b.pdf");
    let gate = runtime::pause_after_prepare(a.clone());
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    let namespace = infra::test_support::namespace();
    let (registry, pipe) = cx.update(|cx| {
        (
            runtime::connect_document_reads(&view, cx),
            runtime::connect_pipe(&view, infra::PipeServer::bind(&namespace).unwrap(), cx),
        )
    });
    let events = Rc::new(RefCell::new(Vec::<ReadCompletion>::new()));
    let captured = events.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, event: &ReadCompletion, _| {
            captured.borrow_mut().push(event.clone())
        })
    });
    let (first, first_rx) =
        pipe_request_with_timeout(namespace.clone(), a, Some(Duration::from_millis(150)));
    pump_until(cx, || gate.reached());
    pump_until(cx, || first.is_finished());
    assert!(first_rx.recv().unwrap().is_err());
    first.join().unwrap();
    let (second, second_rx) = pipe_request(namespace, b.clone());
    pump_until(cx, || second.is_finished());
    assert_eq!(second_rx.recv().unwrap().unwrap().outcome, Ok(()));
    second.join().unwrap();
    gate.release();
    pump_until(cx, || registry.borrow().pending_count() == 0);
    assert_eq!(events.borrow().len(), 2);
    assert!(events.borrow().iter().all(|event| event.outcome.is_ok()));
    assert!(events.borrow()[0].adopted && !events.borrow()[1].adopted);
    assert_eq!(view.read_with(cx, |view, _| view.path.clone()), b);
    cx.update(|cx| pipe.borrow_mut().stop(cx));
    let joining = pipe.borrow_mut().take_joining().unwrap();
    let done = Rc::new(std::cell::Cell::new(false));
    let flag = done.clone();
    cx.update(|cx| {
        cx.spawn(async move |_| {
            joining.await;
            flag.set(true);
        })
        .detach()
    });
    pump_until(cx, || done.get());
}

#[gpui_kit::test]
fn isolated_pipe_returns_real_pdf_outcomes_in_reverse_order_and_parse_errors(
    cx: &mut TestAppContext,
) {
    let files = Files::new();
    let a = files.pdf("pipe-a.pdf");
    let b = files.pdf("pipe-b-日本語.PDF");
    let invalid = files.write("pipe-invalid.pdf", b"invalid");
    let gate = runtime::pause_after_prepare(a.clone());
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    let namespace = infra::test_support::namespace();
    let (registry, pipe) = cx.update(|cx| {
        (
            runtime::connect_document_reads(&view, cx),
            runtime::connect_pipe(&view, infra::PipeServer::bind(&namespace).unwrap(), cx),
        )
    });
    let (first, first_rx) = pipe_request(namespace.clone(), a);
    pump_until(cx, || gate.reached());
    assert!(
        !first.is_finished(),
        "no CLI success before actual prepared delivery"
    );
    let (second, second_rx) = pipe_request(namespace.clone(), b.clone());
    pump_until(cx, || second.is_finished());
    let response = second_rx.recv().unwrap().unwrap();
    assert_eq!(response.outcome, Ok(()));
    assert_eq!(crate::cli::execution_output(response.outcome).exit_code, 0);
    second.join().unwrap();
    gate.release();
    pump_until(cx, || first.is_finished());
    assert_eq!(first_rx.recv().unwrap().unwrap().outcome, Ok(()));
    first.join().unwrap();
    assert_eq!(view.read_with(cx, |view, _| view.path.clone()), b);
    let (failed, failed_rx) = pipe_request(namespace, invalid);
    pump_until(cx, || failed.is_finished());
    let response = failed_rx.recv().unwrap().unwrap();
    assert_eq!(response.outcome, Err("Unable to parse PDF".into()));
    assert_eq!(crate::cli::execution_output(response.outcome).exit_code, 1);
    failed.join().unwrap();
    assert_eq!(view.read_with(cx, |view, _| view.path.clone()), b);
    assert_eq!(registry.borrow().pending_count(), 0);
    cx.update(|cx| pipe.borrow_mut().stop(cx));
    let joining = pipe.borrow_mut().take_joining().unwrap();
    let done = Rc::new(std::cell::Cell::new(false));
    let flag = done.clone();
    cx.update(|cx| {
        cx.spawn(async move |_| {
            joining.await;
            flag.set(true);
        })
        .detach()
    });
    pump_until(cx, || done.get());
    assert!(pipe.borrow().is_stopped());
}
