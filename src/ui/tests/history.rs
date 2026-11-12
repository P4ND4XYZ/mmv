use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, TestAppContext, point, px, test::TestWindowExt as _,
};

use super::Viewer;
use crate::app::{
    DocumentOperation, Mode, OperationError, OperationOrigin, OperationResult, ReadRequest,
};

struct Files(PathBuf);

impl Files {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "mmv-history-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let files = Self(path);
        for name in ["a.md", "b.md", "c.md"] {
            files.write(name, &fixture());
        }
        files
    }

    fn write(&self, name: &str, source: &str) {
        std::fs::write(self.0.join(name), source).unwrap();
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> String {
    "# Start\n\n".to_owned()
        + &"A long wrapped paragraph with 日本語. ".repeat(180)
        + "\n\n# Target\n\n"
        + &"Tail paragraph.\n\n".repeat(100)
}

struct Session {
    window: AnyWindowHandle,
    view: Entity<Viewer>,
    requests: Rc<RefCell<Vec<ReadRequest>>>,
    _subscription: gpui_kit::Subscription,
    files: Files,
}

impl Session {
    fn new(cx: &mut TestAppContext) -> Self {
        let files = Files::new();
        let (window, view) = super::open(cx, fixture());
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                let OperationResult::Pending(request) =
                    viewer.request_document(files.0.join("a.md"), None, cx)
                else {
                    panic!("initial real file read")
                };
                viewer.complete(crate::infra::read_document(request), cx);
            })
        });
        let requests = Rc::new(RefCell::new(Vec::new()));
        let captured = requests.clone();
        let subscription = cx.update(|cx| {
            cx.subscribe(&view, move |_, request: &ReadRequest, _| {
                captured.borrow_mut().push(request.clone());
            })
        });
        let session = Self {
            window,
            view,
            requests,
            _subscription: subscription,
            files,
        };
        session.frame(cx);
        session
    }

    fn frame(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(self.window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn press(&self, cx: &mut TestAppContext, key: &str) {
        cx.update_window(self.window, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn scroll(&self, cx: &mut TestAppContext, distance: f32) {
        cx.update(|cx| {
            self.view.update(cx, |viewer, cx| {
                viewer.document.text.update(cx, |text, cx| {
                    text.list_state()
                        .set_offset_from_scrollbar(point(px(0.), px(-distance)));
                    cx.notify();
                });
            })
        });
    }

    fn assert_distance(&self, cx: &TestAppContext, expected: f32) {
        self.view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.document.distance(cx), expected);
            assert!(viewer.app.position_request().is_none());
        });
    }

    fn link(&self, cx: &mut TestAppContext, url: &str) {
        cx.update_window(self.window, |_, window, cx| {
            self.view
                .update(cx, |viewer, cx| viewer.open_link(url, window, cx));
        })
        .unwrap();
    }

    fn take_read(&self, name: &str, origin: OperationOrigin) -> ReadRequest {
        let mut requests = self.requests.borrow_mut();
        assert_eq!(requests.len(), 1);
        let request = requests.pop().unwrap();
        assert_eq!(request.path, self.files.0.join(name));
        assert_eq!(request.origin, origin);
        request
    }

    fn complete(&self, cx: &mut TestAppContext, request: ReadRequest) {
        let result = crate::infra::read_document(request);
        cx.update(|cx| {
            self.view
                .update(cx, |viewer, cx| viewer.complete(result, cx))
        });
        self.frame(cx);
    }

    fn travel(&self, cx: &mut TestAppContext, key: &str, name: &str) {
        self.press(cx, key);
        self.complete(cx, self.take_read(name, OperationOrigin::Keyboard));
    }

    fn external(&self, cx: &mut TestAppContext, operation: DocumentOperation) -> OperationResult {
        cx.update(|cx| {
            self.view.update(cx, |viewer, cx| {
                viewer.request_operation(operation, OperationOrigin::External, None, cx)
            })
        })
    }
}

#[gpui_kit::test]
fn history_keys_restore_same_heading_and_other_file_with_live_departure(cx: &mut TestAppContext) {
    let s = Session::new(cx);
    s.scroll(cx, 240.);
    s.link(cx, "#target");
    s.frame(cx);
    let heading = s
        .view
        .read_with(cx, |viewer, cx| viewer.document.distance(cx));
    assert!(heading > 1000.);
    s.travel(cx, "ctrl-o", "a.md");
    s.assert_distance(cx, 240.);
    s.travel(cx, "ctrl-i", "a.md");
    s.assert_distance(cx, heading);

    s.link(cx, "b.md#target");
    let request = s.take_read("b.md", OperationOrigin::Link);
    // Completion, not request acceptance, is the moment the document is left.
    s.scroll(cx, 720.);
    s.complete(cx, request);
    s.scroll(cx, 480.);
    s.press(cx, "ctrl-o");
    let request = s.take_read("a.md", OperationOrigin::Keyboard);
    s.scroll(cx, 960.);
    s.complete(cx, request);
    s.assert_distance(cx, 720.);
    s.travel(cx, "ctrl-i", "b.md");
    s.assert_distance(cx, 960.);
}

#[gpui_kit::test]
fn only_a_successful_new_link_discards_forward_history(cx: &mut TestAppContext) {
    let s = Session::new(cx);
    s.link(cx, "b.md");
    s.complete(cx, s.take_read("b.md", OperationOrigin::Link));
    s.scroll(cx, 480.);
    s.travel(cx, "ctrl-o", "a.md");
    s.link(cx, "missing.md");
    s.complete(cx, s.take_read("missing.md", OperationOrigin::Link));
    s.view
        .read_with(cx, |viewer, _| assert!(viewer.app.error().is_some()));
    s.travel(cx, "ctrl-i", "b.md");
    s.assert_distance(cx, 480.);
    s.travel(cx, "ctrl-o", "a.md");
    s.link(cx, "c.md");
    s.complete(cx, s.take_read("c.md", OperationOrigin::Link));
    s.travel(cx, "ctrl-o", "a.md");
    // The surviving forward entry is C, not the discarded B.
    s.travel(cx, "ctrl-i", "c.md");
    assert_eq!(
        s.external(cx, DocumentOperation::Forward),
        OperationResult::Complete {
            response_to: None,
            outcome: Err(OperationError::NoNextEntry),
        }
    );
    assert!(s.requests.borrow().is_empty());
}

#[gpui_kit::test]
fn changed_history_file_restores_pixels_clamps_and_failed_read_keeps_cursor(
    cx: &mut TestAppContext,
) {
    let s = Session::new(cx);
    s.scroll(cx, 2500.);
    s.link(cx, "b.md");
    s.complete(cx, s.take_read("b.md", OperationOrigin::Link));
    s.files
        .write("a.md", &"Changed block boundaries.\n\n".repeat(180));
    s.travel(cx, "ctrl-o", "a.md");
    s.assert_distance(cx, 2500.);
    s.travel(cx, "ctrl-i", "b.md");
    s.files.write("a.md", &"Short paragraph.\n\n".repeat(20));
    s.travel(cx, "ctrl-o", "a.md");
    s.view.read_with(cx, |viewer, cx| {
        let list = viewer.document.text.read(cx).list_state();
        let max = f32::from(list.max_offset_for_scrollbar().y);
        assert!(max > 0. && max < 2500.);
        assert_eq!(viewer.document.distance(cx), max);
        assert_eq!(list.is_scrolled_to_end(), Some(true));
        assert!(viewer.app.position_request().is_none());
    });
    s.travel(cx, "ctrl-i", "b.md");
    s.scroll(cx, 640.);
    let text = s
        .view
        .read_with(cx, |viewer, _| viewer.document.text.clone());
    std::fs::remove_file(s.files.0.join("a.md")).unwrap();
    s.travel(cx, "ctrl-o", "a.md");
    s.assert_distance(cx, 640.);
    s.view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.document.text, text);
        assert_eq!(viewer.app.document().unwrap().source.as_str(), fixture());
        assert!(viewer.app.error().is_some());
    });
    assert_eq!(
        s.external(cx, DocumentOperation::Forward),
        OperationResult::Complete {
            response_to: None,
            outcome: Err(OperationError::NoNextEntry),
        }
    );
    // A retry must still target A; failure must not have moved the cursor.
    s.files.write("a.md", &fixture());
    s.travel(cx, "ctrl-o", "a.md");
    s.travel(cx, "ctrl-i", "b.md");
    s.assert_distance(cx, 640.);
}

#[gpui_kit::test]
fn history_keyboard_is_normal_only_but_external_travel_preserves_input(cx: &mut TestAppContext) {
    let s = Session::new(cx);
    s.scroll(cx, 240.);
    s.link(cx, "b.md");
    s.complete(cx, s.take_read("b.md", OperationOrigin::Link));
    s.scroll(cx, 720.);
    for (key, mode) in [("/", Mode::Search), (":", Mode::Command)] {
        s.press(cx, key);
        cx.update_window(s.window, |_, window, cx| window.input("edit", cx))
            .unwrap();
        s.frame(cx);
        let input = s.view.read_with(cx, |viewer, _| viewer.input.state.clone());
        // Both directions are available in turn, so an incorrectly global binding is observable.
        for (operation, blocked_key, name, distance) in [
            (DocumentOperation::Back, "ctrl-o", "a.md", 240.),
            (DocumentOperation::Forward, "ctrl-i", "b.md", 720.),
        ] {
            s.press(cx, blocked_key);
            assert!(s.requests.borrow().is_empty());
            s.view.read_with(cx, |viewer, cx| {
                assert_eq!(viewer.app.mode, mode);
                assert_eq!(viewer.input.state, input);
                assert_eq!(input.read(cx).value().as_ref(), "edit");
            });
            assert!(matches!(
                s.external(cx, operation),
                OperationResult::Pending(_)
            ));
            s.complete(cx, s.take_read(name, OperationOrigin::External));
            s.assert_distance(cx, distance);
            s.view.read_with(cx, |viewer, cx| {
                assert_eq!(viewer.app.mode, mode);
                assert_eq!(viewer.input.state, input);
                assert_eq!(input.read(cx).value().as_ref(), "edit");
            });
            // Real text input after each replacement proves focus stayed in the prompt.
            cx.update_window(s.window, |_, window, cx| window.input("x", cx))
                .unwrap();
            s.frame(cx);
            s.view.read_with(cx, |viewer, cx| {
                assert_eq!(viewer.app.mode, mode);
                assert_eq!(input.read(cx).value().as_ref(), "editx");
            });
            s.press(cx, "backspace");
        }
        s.press(cx, "escape");
    }
}
