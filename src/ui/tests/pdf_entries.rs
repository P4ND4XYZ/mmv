//! Real tree/link/CLI inputs share runtime resolve + read + prepare + adoption.
use super::pdf::{Files, body_pdf, frame, input, press, request};
use super::*;
use crate::{app::Mode, pdf, runtime};
use gpui_kit::VisualTestContext;
use std::{ffi::OsString, path::Path, time::Duration};

fn select_tree(
    cx: &mut TestAppContext,
    window: gpui_kit::AnyWindowHandle,
    view: &gpui_kit::Entity<Viewer>,
    path: &Path,
) {
    press(cx, window, "ctrl-w");
    press(cx, window, "h");
    press(cx, window, "home");
    for _ in 0..20 {
        if view.read_with(cx, |view, _| view.app.tree.cursor.as_deref() == Some(path)) {
            return;
        }
        press(cx, window, "j");
    }
    panic!("tree entry not found: {}", path.display());
}

#[gpui_kit::test]
fn actual_tree_enter_and_l_open_only_md_and_pdf_and_same_pdf_is_noop(cx: &mut TestAppContext) {
    let files = Files::new();
    files.write(".marksman.toml", b"");
    let markdown = files.write("start.md", b"start");
    let targets = [
        files.write("a.md", b"lowercase"),
        files.write("b.MD", b"uppercase"),
        files.pdf("c.pdf"),
        files.pdf("d.PDF"),
    ];
    files.write("asset.txt", b"not opened");
    files.write("other.markdown", b"not opened from tree");
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    cx.update(|cx| {
        runtime::connect_document_reads(&view, cx);
        runtime::connect_file_tree(&view, cx);
    });
    request(cx, &view, &markdown, "start");
    frame(cx, window);
    press(cx, window, "ctrl-shift-e");
    let root = std::fs::canonicalize(&files.0).unwrap();
    assert_eq!(view.read_with(cx, |view, _| view.app.tree.rows().len()), 9);
    for (index, path) in targets.iter().enumerate() {
        let path = root.join(path.file_name().unwrap());
        select_tree(cx, window, &view, &path);
        let generation = view.read_with(cx, |view, _| view.app.document().unwrap().generation);
        press(cx, window, "right");
        assert_eq!(
            view.read_with(cx, |view, _| view.app.document().unwrap().generation),
            generation
        );
        press(cx, window, if index % 2 == 0 { "l" } else { "enter" });
        assert_eq!(
            view.read_with(cx, |view, _| view.app.document().unwrap().key.0.clone()),
            path
        );
        cx.update_window(window, |_, window, cx| {
            assert!(view.read(cx).body_focus.is_focused(window));
        })
        .unwrap();
    }
    let pdf = body_pdf(cx, &view);
    press(cx, window, "-");
    press(cx, window, "j");
    let top = super::pdf_operations::page_top(cx, window);
    let zoom = pdf.read_with(cx, |pdf, cx| pdf.zoom(cx));
    let calls = pdf::prepare_calls();
    let path = root.join("d.PDF");
    std::fs::write(&path, b"same open must not read or parse").unwrap();
    select_tree(cx, window, &view, &path);
    press(cx, window, "enter");
    assert_eq!(body_pdf(cx, &view), pdf);
    assert_eq!(super::pdf_operations::page_top(cx, window), top);
    assert_eq!(pdf::prepare_calls(), calls);
    assert_eq!(pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)), zoom);
    cx.update_window(window, |_, window, cx| {
        assert!(view.read(cx).body_focus.is_focused(window));
    })
    .unwrap();
    for name in ["asset.txt", "other.markdown"] {
        select_tree(cx, window, &view, &root.join(name));
        for key in ["l", "enter", "right"] {
            press(cx, window, key);
        }
        assert_eq!(body_pdf(cx, &view), pdf);
        assert_eq!(pdf::prepare_calls(), calls);
        cx.update_window(window, |_, window, cx| {
            assert!(view.read(cx).tree.focus.is_focused(window));
        })
        .unwrap();
    }
    // A new tree failure retains the live PDF, multiplier and tree input owner.
    let invalid = root.join("c.pdf");
    std::fs::write(&invalid, b"invalid PDF").unwrap();
    select_tree(cx, window, &view, &invalid);
    press(cx, window, "l");
    assert_eq!(body_pdf(cx, &view), pdf);
    assert_eq!(super::pdf_operations::page_top(cx, window), top);
    assert_eq!(pdf.read_with(cx, |pdf, cx| pdf.zoom(cx)), zoom);
    view.read_with(cx, |view, _| {
        assert_eq!(view.app.tree.cursor.as_deref(), Some(invalid.as_path()));
        assert!(view.app.error().unwrap().contains("Unable to parse PDF"));
    });
    cx.update_window(window, |_, window, cx| {
        assert!(view.read(cx).tree.focus.is_focused(window));
    })
    .unwrap();
    std::fs::remove_file(&invalid).unwrap();
    press(cx, window, "enter");
    assert_eq!(body_pdf(cx, &view), pdf);
    assert!(view.read_with(cx, |view, _| {
        view.app.error().unwrap().starts_with("File not found")
    }));
}

#[gpui_kit::test]
fn tree_pdf_delayed_success_does_not_steal_command_search_or_hidden_tree_focus(
    cx: &mut TestAppContext,
) {
    for mode in ["command", "search", "hidden"] {
        let files = Files::new();
        files.write(".marksman.toml", b"");
        let markdown = files.write("source.md", b"source");
        let path = std::fs::canonicalize(files.pdf("target.pdf")).unwrap();
        let gate = runtime::pause_after_prepare(path.clone());
        let (window, view) = super::open(cx, "old".into());
        frame(cx, window);
        cx.update(|cx| {
            runtime::connect_document_reads(&view, cx);
            runtime::connect_file_tree(&view, cx);
        });
        request(cx, &view, &markdown, "start");
        frame(cx, window);
        press(cx, window, "ctrl-shift-e");
        select_tree(cx, window, &view, &std::fs::canonicalize(&path).unwrap());
        press(cx, window, "l");
        assert!(gate.reached());
        press(cx, window, "ctrl-w");
        press(cx, window, "l");
        match mode {
            "command" => {
                press(cx, window, ":");
                input(cx, window, "draft");
            }
            "search" => {
                press(cx, window, "/");
                input(cx, window, "draft");
            }
            _ => press(cx, window, "ctrl-shift-e"),
        }
        gate.release();
        frame(cx, window);
        body_pdf(cx, &view);
        cx.update_window(window, |_, window, cx| {
            let view = view.read(cx);
            if mode == "hidden" {
                assert!(!view.tree.visible);
                assert!(view.body_focus.is_focused(window));
            } else {
                assert_eq!(
                    view.app.mode,
                    if mode == "command" {
                        Mode::Command
                    } else {
                        Mode::Search
                    }
                );
                assert_eq!(view.input.state.read(cx).value().as_ref(), "draft");
                assert!(
                    view.input
                        .state
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn markdown_pdf_links_click_and_panel_and_history_use_actual_common_open(cx: &mut TestAppContext) {
    let files = Files::new();
    let path = files.pdf("日本語 target.PDF");
    let source = files.write(
        "source.md",
        "[Local PDF](日本語%20target.PDF)\n\n"
            .repeat(100)
            .as_bytes(),
    );
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    cx.update(|cx| runtime::connect_document_reads(&view, cx));
    request(cx, &view, &source, "source");
    frame(cx, window);
    // Real Markdown click callback; rendered text is near the top of the body.
    let mut visual = VisualTestContext::from_window(window, cx);
    let bounds = visual.debug_bounds("document-body").unwrap();
    visual.simulate_click(
        bounds.origin + point(px(45.), px(20.)),
        gpui_kit::Modifiers::default(),
    );
    frame(cx, window);
    assert_eq!(
        view.read_with(cx, |view, _| view.app.document().unwrap().key.0.clone()),
        std::fs::canonicalize(&path).unwrap()
    );
    press(cx, window, "+");
    press(cx, window, "ctrl-d");
    let old_pdf = body_pdf(cx, &view);
    press(cx, window, "ctrl-o");
    assert!(view.read_with(cx, |view, _| view.document.as_markdown().is_some()));
    press(cx, window, "ctrl-d");
    let saved = view.read_with(cx, |view, cx| view.document.distance(cx));
    assert!(saved > 0.);
    press(cx, window, "f");
    input(cx, window, "Local PDF");
    press(cx, window, "enter");
    let fresh = body_pdf(cx, &view);
    assert_ne!(old_pdf, fresh);
    assert_eq!(view.read_with(cx, |view, _| view.path.clone()), path);
    press(cx, window, "ctrl-o");
    let restored = view.read_with(cx, |view, cx| view.document.distance(cx));
    assert!((restored - saved).abs() < 1.);
    press(cx, window, "ctrl-i");
    let history = body_pdf(cx, &view);
    assert_ne!(history, fresh);
    assert!(
        (history.read_with(cx, |pdf, cx| pdf.zoom(cx))
            - fresh.read_with(cx, |pdf, cx| pdf.zoom(cx)))
        .abs()
            < 0.0001
    );
}

#[gpui_kit::test]
fn both_cli_spellings_cross_isolated_pipe_and_same_open_preserves_view(cx: &mut TestAppContext) {
    let files = Files::new();
    let path = files.pdf("cli 日本語.PDF");
    let (window, view) = super::open(cx, "old".into());
    frame(cx, window);
    let namespace = crate::infra::test_support::namespace();
    let pipe = cx.update(|cx| {
        runtime::connect_document_reads(&view, cx);
        runtime::connect_pipe(
            &view,
            crate::infra::PipeServer::bind(&namespace).unwrap(),
            cx,
        )
    });
    let mut original = None;
    let mut calls = 0;
    for explicit in [true, false] {
        let namespace = namespace.clone();
        let cwd = files.0.clone();
        let name = path.file_name().unwrap().to_owned();
        let worker = std::thread::spawn(move || {
            let mut args = vec![OsString::from("mmv")];
            if explicit {
                args.push("open".into());
            }
            args.push(name);
            let operation = crate::cli::parse_from(args, &cwd).unwrap();
            let client = crate::infra::PipeClient::connect(
                &namespace,
                crate::infra::Deadline::connection(Instant::now()),
            )
            .unwrap();
            client
                .request(
                    crate::infra::Request {
                        id: ExternalRequestId("cli-entry".into()),
                        operation,
                    },
                    crate::infra::Deadline::response(Instant::now()),
                )
                .unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !worker.is_finished() {
            cx.run_until_parked();
            assert!(Instant::now() < deadline, "isolated CLI entry stalled");
            cx.executor().advance_clock(Duration::from_millis(10));
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(worker.join().unwrap().outcome, Ok(()));
        frame(cx, window);
        let current = body_pdf(cx, &view);
        if let Some(original) = &original {
            assert_eq!(&current, original);
            assert_eq!(pdf::prepare_calls(), calls);
        } else {
            press(cx, window, "+");
            press(cx, window, "j");
            calls = pdf::prepare_calls();
            original = Some(current);
            std::fs::write(&path, b"same open must not read").unwrap();
        }
    }
    cx.update(|cx| pipe.borrow_mut().stop(cx));
    let joining = pipe.borrow_mut().take_joining().unwrap();
    let done = Rc::new(std::cell::Cell::new(false));
    let flag = done.clone();
    cx.update(|cx| {
        cx.spawn(async move |_| {
            joining.await;
            flag.set(true);
        })
        .detach();
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done.get() {
        cx.run_until_parked();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(pipe.borrow().is_stopped());
}
