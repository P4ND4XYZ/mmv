//! Real Named Pipe -> production delivery -> Viewer -> production reply wiring.
use crate::{
    app::{ExternalOperation, ExternalRequestId, ReadRequest},
    infra, runtime,
};
use gpui_kit::{AppContext as _, Focusable as _, TestAppContext, test::TestWindowExt as _};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn pump_until(cx: &mut TestAppContext, mut ready: impl FnMut() -> bool) {
    let limit = Instant::now() + Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if ready() {
            return;
        }
        assert!(Instant::now() < limit, "production Pipe delivery stalled");
        cx.executor().advance_clock(Duration::from_millis(10));
        thread::sleep(Duration::from_millis(2));
    }
}

fn request(
    namespace: &infra::Namespace,
    operation: ExternalOperation,
    id: &str,
    timeout: bool,
) -> (
    thread::JoinHandle<()>,
    mpsc::Receiver<std::io::Result<infra::Response>>,
) {
    let namespace = namespace.clone();
    let id = ExternalRequestId(id.into());
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let client =
            infra::PipeClient::connect(&namespace, infra::Deadline::connection(Instant::now()))
                .unwrap();
        let deadline = if timeout {
            infra::test_support::deadline(Duration::from_millis(150))
        } else {
            infra::Deadline::response(Instant::now())
        };
        let _ = tx.send(client.request(infra::Request { id, operation }, deadline));
    });
    (worker, rx)
}

fn join(cx: &mut TestAppContext, connection: &Rc<RefCell<runtime::PipeConnection>>) {
    let task = connection.borrow_mut().take_joining().unwrap();
    let done = Rc::new(std::cell::Cell::new(false));
    let flag = done.clone();
    cx.update(|cx| {
        cx.spawn(async move |_| {
            task.await;
            flag.set(true);
        })
        .detach()
    });
    pump_until(cx, || done.get());
}

#[gpui_kit::test]
fn production_pipe_reverse_completions_with_colliding_external_ids_and_disconnect(
    cx: &mut TestAppContext,
) {
    let (_, view) = super::open(cx, "original".into());
    let namespace = infra::test_support::namespace();
    let connection = cx.update(|cx| {
        runtime::connect_pipe(&view, infra::PipeServer::bind(&namespace).unwrap(), cx)
    });
    let reads = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = reads.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, read: &ReadRequest, _| {
            captured.borrow_mut().push(read.clone())
        })
    });
    let path = view.read_with(cx, |view, _| view.path.clone());
    let (a, a_rx) = request(
        &namespace,
        ExternalOperation::Open {
            path: path.with_file_name("a.md"),
            line: None,
        },
        "collision",
        false,
    );
    pump_until(cx, || reads.borrow().len() == 1);
    let (b, b_rx) = request(
        &namespace,
        ExternalOperation::Open {
            path: path.with_file_name("b.md"),
            line: None,
        },
        "collision",
        false,
    );
    pump_until(cx, || reads.borrow().len() == 2);
    assert_eq!(connection.borrow().pending_count(), 2);
    let b_read = reads.borrow_mut().pop().unwrap();
    let a_read = reads.borrow_mut().pop().unwrap();
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.complete(super::success(b_read.clone(), "newest".into()), cx)
        })
    });
    pump_until(cx, || b.is_finished());
    assert_eq!(b_rx.recv().unwrap().unwrap().outcome, Ok(()));
    assert!(matches!(a_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.complete(
                crate::app::ReadResult {
                    request: a_read,
                    content: Err("older read failed".into()),
                },
                cx,
            )
        })
    });
    pump_until(cx, || a.is_finished());
    assert_eq!(
        a_rx.recv().unwrap().unwrap().outcome,
        Err("older read failed".into())
    );
    a.join().unwrap();
    b.join().unwrap();
    view.read_with(cx, |view, _| assert_eq!(view.path, b_read.path));

    let (gone, gone_rx) = request(
        &namespace,
        ExternalOperation::Open {
            path: path.with_file_name("late.md"),
            line: None,
        },
        "gone",
        true,
    );
    pump_until(cx, || reads.borrow().len() == 1);
    pump_until(cx, || gone.is_finished());
    assert_eq!(
        gone_rx.recv().unwrap().unwrap_err().kind(),
        std::io::ErrorKind::TimedOut
    );
    gone.join().unwrap();
    pump_until(cx, || connection.borrow().pending_count() == 0);
    let late = reads.borrow_mut().pop().unwrap();
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.complete(super::success(late.clone(), "after disconnect".into()), cx)
        })
    });
    view.read_with(cx, |view, _| assert_eq!(view.path, late.path));
    let (next, rx) = request(&namespace, ExternalOperation::Top, "next", false);
    pump_until(cx, || next.is_finished());
    assert_eq!(rx.recv().unwrap().unwrap().outcome, Ok(()));
    next.join().unwrap();
    let (held, held_rx) = request(
        &namespace,
        ExternalOperation::Open {
            path: path.with_file_name("held.md"),
            line: None,
        },
        "shutdown",
        false,
    );
    pump_until(cx, || reads.borrow().len() == 1);
    cx.update(|cx| connection.borrow_mut().stop(cx));
    assert_eq!(connection.borrow().pending_count(), 0);
    pump_until(cx, || held.is_finished());
    assert!(held_rx.recv().unwrap().is_err());
    held.join().unwrap();
    join(cx, &connection);
    assert!(connection.borrow().is_stopped());
}

#[gpui_kit::test]
fn production_pipe_reads_real_files_and_preserves_prompt_composition_and_focus(
    cx: &mut TestAppContext,
) {
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = std::env::temp_dir().join(format!(
        "mmv-production-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    let fixture = Fixture(dir);
    let path = fixture.0.join("日本語 memo.md");
    std::fs::write(&path, "edit paragraph\n\n".repeat(100)).unwrap();
    for key in ["/", ":"] {
        let (window, view) = super::open(cx, "old".into());
        let namespace = infra::test_support::namespace();
        let connection = cx.update(|cx| {
            runtime::connect_document_reads(&view, cx);
            runtime::connect_pipe(&view, infra::PipeServer::bind(&namespace).unwrap(), cx)
        });
        cx.update_window(window, |_, window, cx| {
            window.press(key, cx);
            window.input("edit", cx);
        })
        .unwrap();
        cx.run_until_parked();
        let input = view.read_with(cx, |view, _| view.input.state.clone());
        cx.update_window(window, |_, window, cx| {
            input.update(cx, |input, cx| {
                gpui_kit::EntityInputHandler::replace_and_mark_text_in_range(
                    input,
                    None,
                    "編集中",
                    Some(0..3),
                    window,
                    cx,
                )
            });
        })
        .unwrap();
        let mode = view.read_with(cx, |view, _| view.app.mode.clone());
        for (operation, success) in [
            (
                ExternalOperation::Open {
                    path: path.clone(),
                    line: std::num::NonZeroU64::new(150),
                },
                true,
            ),
            (
                ExternalOperation::Open {
                    path: path.clone(),
                    line: std::num::NonZeroU64::new(175),
                },
                true,
            ),
            (ExternalOperation::Bottom, true),
            (
                ExternalOperation::Open {
                    path: path.with_file_name("missing.md"),
                    line: None,
                },
                false,
            ),
            (ExternalOperation::Top, true),
        ] {
            let (worker, rx) = request(&namespace, operation, "same", false);
            pump_until(cx, || worker.is_finished());
            assert_eq!(rx.recv().unwrap().unwrap().outcome.is_ok(), success);
            worker.join().unwrap();
            cx.update_window(window, |_, window, cx| {
                view.read_with(cx, |view, cx| {
                    assert_eq!(view.app.mode, mode);
                    assert_eq!(view.input.state, input);
                    assert!(input.read(cx).value().contains("編集中"));
                    assert!(input.focus_handle(cx).is_focused(window));
                })
            })
            .unwrap();
            cx.update_window(window, |_, window, cx| {
                input.update(cx, |input, cx| {
                    assert!(
                        gpui_kit::EntityInputHandler::marked_text_range(input, window, cx)
                            .is_some()
                    );
                })
            })
            .unwrap();
        }
        cx.update(|cx| connection.borrow_mut().stop(cx));
        join(cx, &connection);
    }
}
