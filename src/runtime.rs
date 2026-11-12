mod diagram;
mod document;
mod file_tree;
mod pipe;
mod watch;

use gpui_kit::{AppContext as _, WindowOptions, assets::Assets};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc, time::Instant};

use crate::{infra, ui};
use diagram::connect_diagrams;
pub(crate) use document::connect_document_reads;
pub(crate) use file_tree::connect_file_tree;
pub(crate) use pipe::connect_pipe;
pub(crate) use watch::connect_watch;

#[cfg(all(test, feature = "ui-tests"))]
pub(crate) use pipe::PipeConnection;

pub(crate) fn run_gui() -> std::io::Result<()> {
    let namespace = infra::Namespace::current_user()?;
    #[cfg(all(test, feature = "ui-tests"))]
    let namespace = infra::test_support::process_namespace().unwrap_or(namespace);
    // Thread-owned guard outlives the window, Pipe shutdown and application run.
    let Some(_live) = infra::InstanceGuard::claim_gui(&namespace)? else {
        return Ok(());
    };
    let server = infra::PipeServer::bind(&namespace)?;
    let loaded = infra::load_config();
    if let Some(error) = &loaded.error {
        infra::log_config_failure(error);
    }
    let config_notice = loaded.error.is_some();
    let config = loaded.config;
    let started = Instant::now();
    let startup_error = Rc::new(RefCell::new(None));
    let report_error = startup_error.clone();
    gpui_kit::application()
        .with_assets(Assets)
        .with_http_client(Arc::new(reqwest_client::ReqwestClient::new()))
        .run(move |cx| {
            gpui_kit::init(cx);
            // Never scan TEMP or wait for cleanup on the UI/document-read path.
            cx.background_executor()
                .spawn(async { infra::cleanup_mermaid_scratch() })
                .detach();
            ui::bind_keys(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let opened = gpui_kit::open_window(
                WindowOptions {
                    focus: false,
                    window_min_size: Some(gpui_kit::size(
                        gpui_kit::px(ui::pane::MIN_WINDOW_WIDTH),
                        gpui_kit::px(120.),
                    )),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("mmv");
                    window.zoom_window();
                    // No Initial read: the first external Open uses the shared app path.
                    cx.new(|cx| {
                        ui::Viewer::with_config(
                            PathBuf::new(),
                            started,
                            config,
                            config_notice,
                            window,
                            cx,
                        )
                    })
                },
            );
            let (_, view) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    *report_error.borrow_mut() = Some(std::io::Error::other(error.to_string()));
                    // No window owns the server yet. Reap on the background
                    // executor and make application quit await that join too.
                    let mut server = server;
                    server.request_shutdown();
                    let mut joining = Some(cx.background_executor().spawn(async move {
                        server.shutdown();
                    }));
                    cx.on_app_quit(move |_| {
                        let joining = joining.take();
                        async move {
                            if let Some(joining) = joining {
                                joining.await;
                            }
                        }
                    })
                    .detach();
                    cx.quit();
                    return;
                }
            };
            connect_document_reads(&view, cx);
            connect_file_tree(&view, cx);
            connect_diagrams(&view, cx);
            connect_watch(&view, cx);
            connect_pipe(&view, server, cx);
        });
    match startup_error.borrow_mut().take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
