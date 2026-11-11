use super::*;

#[gpui_kit::test]
fn startup_configuration_notice_survives_first_document_adoption(cx: &mut TestAppContext) {
    let (window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        super::super::bind_keys(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                Viewer::with_config(
                    PathBuf::new(),
                    Instant::now(),
                    Default::default(),
                    true,
                    window,
                    cx,
                )
            })
        })
        .unwrap()
    });
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let request = viewer.app.open(PathBuf::from("example.md"));
            viewer.complete(success(request, "body".into()), cx);
        })
    });
    cx.run_until_parked();
    view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.status_text(), "Unable to load configuration");
        assert!(viewer.app.document().is_some());
    });
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    assert!(visual.debug_bounds("command-line").is_some());
}
