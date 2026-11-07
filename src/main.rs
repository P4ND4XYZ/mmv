extern crate gpui_kit as gpui;

mod app;
mod core;
mod infra;
mod ui;

use gpui_kit::{AppContext as _, WindowOptions, assets::Assets};
use std::{path::PathBuf, sync::Arc, time::Instant};

fn document_argument() -> Result<Option<PathBuf>, String> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next();
    if args.next().is_some() {
        return Err("usage: my-markdown-viewer [FILE.md]".into());
    }
    if path
        .as_deref()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "GPUI evaluation prototype (V-01 / V-02)\nusage: my-markdown-viewer [FILE.md]\n\n/ : search; Enter/Esc : return to document; n/N : next/previous; F5 : reload"
        );
        return Ok(None);
    }
    let path = path.map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("docs/design/prototypes/gpui-evaluation/fixtures/basic.md")
    });
    std::path::absolute(path)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn main() {
    let started = Instant::now();
    let path = match document_argument() {
        Ok(Some(path)) => path,
        Ok(None) => return,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    gpui_kit::application()
        .with_assets(Assets)
        .with_http_client(Arc::new(reqwest_client::ReqwestClient::new()))
        .run(move |cx| {
            gpui_kit::init(cx);
            ui::bind_keys(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let (_, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                window.set_window_title("GPUI Markdown evaluation");
                cx.new(|cx| ui::Viewer::new(path, started, window, cx))
            })
            .expect("failed to open evaluation window");
            cx.subscribe(&view, |view, request: &app::ReadRequest, cx| {
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
            // GPUI activates subscriptions at the end of the current update.
            cx.defer(move |cx| view.update(cx, |view, cx| view.open(cx)));
            cx.activate(true);
        });
}
