use std::{cell::RefCell, rc::Rc};

use gpui_kit::{AppContext as _, TestAppContext, point, px, test::TestWindowExt as _};

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
    // Applying the request after layout is deliberately a stage-2 concern.
}

#[gpui_kit::test]
fn external_replacement_keeps_search_and_command_input_entity_text_and_focus(
    cx: &mut TestAppContext,
) {
    let (window, view) = super::open(cx, "old target".into());
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
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                let result = viewer.request_document(
                    viewer.path.clone(),
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
                viewer.complete(super::success(request, "new edit target".into()), cx);
            })
        });
        cx.run_until_parked();
        view.read_with(cx, |viewer, cx| {
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
