use super::*;
#[cfg(feature = "ui-tests")]
use gpui_kit::{Point, TestAppContext, div, px, size};

// Self-authored minimal PDF objects; no customer documents, fonts or external assets.
pub(crate) fn fixture(pages: &str, count: usize, encrypted: bool) -> Arc<Vec<u8>> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!("<< /Type /Pages /Kids [{pages}] /Count {count} >>"),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_owned(),
    ];
    if encrypted {
        // Standard revision-2 handler with nonmatching owner/user credentials.
        objects.push(format!(
            "<< /Filter /Standard /V 1 /R 2 /Length 40 /P -4 /O <{}> /U <{}> >>",
            "00".repeat(32),
            "00".repeat(32),
        ));
    }
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    let encryption = if encrypted {
        " /Encrypt 4 0 R /ID [<00112233445566778899aabbccddeeff> <00112233445566778899aabbccddeeff>]"
    } else {
        ""
    };
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R{encryption} >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len(),
        )
        .as_bytes(),
    );
    Arc::new(bytes)
}

#[test]
fn real_prepare_measures_every_page_and_reclaims_shared_bytes_without_window() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<PreparedPdf>();
    let bytes = fixture("3 0 R 3 0 R", 2, false);
    let weak = Arc::downgrade(&bytes);
    let prepared = prepare(bytes.clone(), &PrepareCancellation::default()).unwrap();
    assert_eq!(
        prepared.inner.page_dimensions(),
        &[(612.0, 792.0), (612.0, 792.0)]
    );
    drop(bytes);
    assert!(weak.upgrade().is_some());
    drop(prepared);
    assert!(weak.upgrade().is_none());
}

#[test]
fn real_prepare_rejects_invalid_and_empty_documents() {
    for (bytes, expected) in [
        (
            Arc::new(b"not a PDF".to_vec()),
            PrepareErrorKind::InvalidDocument,
        ),
        (fixture("", 0, false), PrepareErrorKind::NoPages),
    ] {
        assert_eq!(
            prepare(bytes, &PrepareCancellation::default())
                .err()
                .unwrap()
                .kind(),
            expected
        );
    }
}

#[test]
fn real_prepare_reports_locked_without_adding_password_ui() {
    assert_eq!(
        prepare(fixture("3 0 R", 1, true), &PrepareCancellation::default())
            .err()
            .unwrap()
            .kind(),
        PrepareErrorKind::Locked,
    );
}

#[test]
fn cloned_token_cancels_but_dropping_a_token_does_not() {
    let token = PrepareCancellation::default();
    drop(token.clone());
    assert!(!token.is_cancelled());
    let bytes = fixture("3 0 R", 1, false);
    let weak = Arc::downgrade(&bytes);
    token.clone().cancel();
    assert!(token.is_cancelled());
    assert_eq!(
        prepare(bytes, &token).err().unwrap().kind(),
        PrepareErrorKind::Abandoned
    );
    assert!(weak.upgrade().is_none());
}

#[cfg(feature = "ui-tests")]
#[gpui_kit::test]
fn actual_unlaid_view_maps_operations_and_releases_without_window(cx: &mut TestAppContext) {
    let bytes = fixture("3 0 R", 1, false);
    let weak = Arc::downgrade(&bytes);
    let prepared = prepare(bytes, &PrepareCancellation::default()).unwrap();
    let view = cx.new(|cx| PdfView::from_prepared(prepared, cx));
    view.update(cx, |view, cx| {
        assert_eq!(view.inner.read(cx).controls(), gpui_pdf::Controls::Host);
        assert_eq!(
            view.scroll_by(f32::NAN, cx).unwrap_err().kind,
            OperationErrorKind::InvalidArgument
        );

        assert_eq!(
            view.scroll_by(40.0, cx).unwrap().status,
            OperationStatus::DeferredUntilLayout
        );
        assert_eq!(
            view.scroll_viewport(0.5, cx).unwrap().status,
            OperationStatus::DeferredUntilLayout
        );
        assert_eq!(
            view.scroll_to_edge(true, cx).unwrap().status,
            OperationStatus::DeferredUntilLayout
        );

        view.request_zoom(ZoomOperation::ZoomIn, cx);
        view.request_zoom(ZoomOperation::FitPage, cx);
        assert_eq!(view.target().zoom(), None);
        view.release(None, cx);
        assert!(view.pending_zoom.is_empty());
        view.release(None, cx);
        assert!(view.is_released(cx));

        assert_eq!(
            view.scroll_to_edge(false, cx).unwrap_err().kind,
            OperationErrorKind::Released
        );
    });
    assert!(weak.upgrade().is_none());
}

#[cfg(feature = "ui-tests")]
#[gpui_kit::test]
fn body_click_bridge_notifies_host_and_disconnects_on_release(cx: &mut TestAppContext) {
    let prepared = prepare(fixture("3 0 R", 1, false), &PrepareCancellation::default()).unwrap();
    let view = cx.new(|cx| PdfView::from_prepared(prepared, cx));
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed = count.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, _: &BodyClicked, _| {
            observed.set(observed.get() + 1)
        })
    });
    view.update(cx, |view, cx| {
        view.inner
            .update(cx, |_, cx| cx.emit(gpui_pdf::PdfEvent::BodyClicked));
    });
    cx.run_until_parked();
    assert_eq!(count.get(), 1);
    view.update(cx, |view, cx| {
        view.release(None, cx);
        view.inner
            .update(cx, |_, cx| cx.emit(gpui_pdf::PdfEvent::BodyClicked));
    });
    cx.run_until_parked();
    assert_eq!(count.get(), 1);
}

#[cfg(feature = "ui-tests")]
#[gpui_kit::test]
fn wrapper_drop_releases_consumed_prepared_without_window(cx: &mut TestAppContext) {
    let bytes = fixture("3 0 R", 1, false);
    let weak = Arc::downgrade(&bytes);
    let prepared = prepare(bytes, &PrepareCancellation::default()).unwrap();
    let view = cx.new(|cx| PdfView::from_prepared(prepared, cx));
    assert!(weak.upgrade().is_some());
    drop(view);
    cx.update(|_| {});
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
}

#[cfg(feature = "ui-tests")]
#[gpui_kit::test]
fn real_layout_fit_and_release_with_window(cx: &mut TestAppContext) {
    let prepared = prepare(
        fixture("3 0 R 3 0 R", 2, false),
        &PrepareCancellation::default(),
    )
    .unwrap();
    let view = cx.new(|cx| PdfView::from_prepared(prepared, cx));
    let cx = cx.add_empty_window();
    for _ in 0..2 {
        let view = view.clone();
        cx.draw(Point::default(), size(px(600.0), px(400.0)), |_, _| {
            div().size_full().child(view).into_any_element()
        });
    }
    view.update(cx, |view, cx| {
        view.request_zoom(ZoomOperation::FitWidth, cx);
        assert_eq!(
            view.scroll_by(40.0, cx).unwrap().status,
            OperationStatus::Applied
        );
        assert_eq!(
            view.scroll_to_edge(false, cx).unwrap().status,
            OperationStatus::Applied
        );
        assert_eq!(
            view.scroll_to_edge(false, cx).unwrap().status,
            OperationStatus::Unchanged
        );
        view.request_zoom(ZoomOperation::SetZoom(1.1), cx);
        assert!((view.zoom(cx) - 1.1).abs() < 0.0001);
        view.request_zoom(ZoomOperation::FitWidth, cx);
        view.request_zoom(ZoomOperation::FitWidth, cx);
    });
    cx.update(|window, cx| view.update(cx, |view, cx| view.release(Some(window), cx)));
    view.read_with(cx, |view, cx| assert!(view.is_released(cx)));
    cx.run_until_parked();
}

#[cfg(feature = "ui-tests")]
#[gpui_kit::test]
fn zoom_requests_wait_for_layout_and_apply_relative_steps_in_fifo(cx: &mut TestAppContext) {
    let prepared = prepare(
        fixture("3 0 R 3 0 R", 2, false),
        &PrepareCancellation::default(),
    )
    .unwrap();
    let view = cx.new(|cx| PdfView::from_prepared(prepared, cx));
    let target = view.read_with(cx, |view, _| view.target());
    view.update(cx, |view, cx| {
        for operation in [
            ZoomOperation::SetZoom(0.5),
            ZoomOperation::ZoomIn,
            ZoomOperation::ZoomIn,
            ZoomOperation::ZoomOut,
        ] {
            view.request_zoom(operation, cx);
        }
        assert_eq!(target.zoom(), None);
        assert_eq!(view.zoom(cx), 1.0);
    });
    let cx = cx.add_empty_window();
    for _ in 0..4 {
        let view = view.clone();
        cx.draw(Point::default(), size(px(600.0), px(400.0)), |_, _| {
            div().size_full().child(view).into_any_element()
        });
        cx.run_until_parked();
    }
    assert!((target.zoom().unwrap() - 0.55).abs() < 0.0001);
    view.update(cx, |view, cx| {
        assert!(view.pending_zoom.is_empty());
        view.request_zoom(ZoomOperation::SetZoom(3.0), cx);
        view.request_zoom(ZoomOperation::ZoomIn, cx);
        assert_eq!(target.zoom(), Some(3.0));
        view.request_zoom(ZoomOperation::SetZoom(0.01), cx);
        view.request_zoom(ZoomOperation::ZoomOut, cx);
        assert_eq!(target.zoom(), Some(0.01));
        view.release(None, cx);
        assert_eq!(target.zoom(), None);
        view.request_zoom(ZoomOperation::ZoomIn, cx);
        assert!(view.pending_zoom.is_empty());
    });
}

#[cfg(feature = "ui-tests")]
#[gpui_kit::test]
fn native_page_fit_uses_topmost_not_center_and_resize_uses_current_page(cx: &mut TestAppContext) {
    // Two valid, self-authored blank pages with distinct aspect ratios.
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 1224] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 2448] >>",
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let prepared = prepare(Arc::new(bytes), &Default::default()).unwrap();
    assert_eq!(
        prepared.inner.page_dimensions(),
        &[(612.0, 1224.0), (612.0, 2448.0)]
    );
    let view = cx.new(|cx| PdfView::from_prepared(prepared, cx));
    let cx = cx.add_empty_window();
    for _ in 0..3 {
        let view = view.clone();
        cx.draw(Point::default(), size(px(600.0), px(400.0)), |_, _| {
            div().size_full().child(view).into_any_element()
        });
        cx.run_until_parked();
    }
    view.update(cx, |view, cx| {
        view.request_zoom(ZoomOperation::SetZoom(0.5), cx);
        // First page ends at y=836. At scroll y=740 the center is on page 2,
        // while the viewport top remains on page 1.
        view.scroll_by(740.0, cx).unwrap();
        view.request_zoom(ZoomOperation::FitPage, cx);
        assert!((view.zoom(cx) - 368.0 / (820.0 * 2.0)).abs() < 0.0001);
        view.inner.update(cx, |inner, cx| inner.go_to_page(1, cx));
    });
    for _ in 0..3 {
        let view = view.clone();
        cx.draw(Point::default(), size(px(600.0), px(500.0)), |_, _| {
            div().size_full().child(view).into_any_element()
        });
        cx.run_until_parked();
    }
    view.read_with(cx, |view, cx| {
        assert!((view.zoom(cx) - 468.0 / (820.0 * 4.0)).abs() < 0.0001);
        assert_eq!(view.target().zoom(), Some(view.zoom(cx)));
    });
}
