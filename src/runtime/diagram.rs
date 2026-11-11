use crate::{app, infra, ui};

pub(super) fn connect_diagrams(view: &gpui_kit::Entity<ui::Viewer>, cx: &mut gpui_kit::App) {
    cx.subscribe(view, |view, request: &app::DiagramRequest, cx| {
        let request = request.clone();
        let result = cx.background_executor().spawn(async move {
            let result = infra::generate_diagram(request);
            infra::log_diagram_failure(&result);
            result
        });
        let view = view.downgrade();
        cx.spawn(async move |cx| {
            let result = result.await;
            let _ = view.update(cx, |view, cx| view.complete_diagram(result, cx));
        })
        .detach();
    })
    .detach();
}
