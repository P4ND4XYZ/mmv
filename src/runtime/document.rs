use crate::{app, infra, ui};

pub(crate) fn connect_document_reads(view: &gpui_kit::Entity<ui::Viewer>, cx: &mut gpui_kit::App) {
    cx.subscribe(view, |view, request: &app::ReadRequest, cx| {
        let request = request.clone();
        let result = cx
            .background_executor()
            .spawn(async move { infra::read_document(request) });
        let view = view.downgrade();
        cx.spawn(async move |cx| {
            let result = result.await;
            let _ = view.update(cx, |view, cx| view.complete(result, cx));
        })
        .detach();
    })
    .detach();
}
