extern crate gpui_kit as gpui;

mod app;
mod cli;
mod core;
mod infra;
mod ui;

use clap::Parser;
use gpui_kit::{AppContext as _, WindowOptions, assets::Assets};
use std::{path::PathBuf, sync::Arc, time::Instant};

#[derive(Debug, Parser)]
#[command(version)]
struct Args {
    /// UTF-8 Markdown file to display
    #[arg(value_name = "FILE.md")]
    file: PathBuf,
}

fn main() {
    let started = Instant::now();
    let args = Args::parse();
    let path = match std::path::absolute(args.file) {
        Ok(path) => path,
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
            connect_document_reads(&view, cx);
            cx.subscribe(&view, |view, request: &app::DiagramRequest, cx| {
                let request = request.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move { infra::generate_diagram(request) });
                let view = view.downgrade();
                cx.spawn(async move |cx| {
                    let result = result.await;
                    let _ = view.update(cx, |view, cx| view.complete_diagram(result, cx));
                })
                .detach();
            })
            .detach();
            connect_watch(&view, cx);
            // GPUI activates subscriptions at the end of the current update.
            cx.defer(move |cx| view.update(cx, |view, cx| view.open(cx)));
            cx.activate(true);
        });
}

fn connect_document_reads(view: &gpui_kit::Entity<ui::Viewer>, cx: &mut gpui_kit::App) {
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

struct WatchConnection {
    watch: Option<infra::FileWatch>,
    delivery: Option<gpui_kit::Task<()>>,
    joining: Option<gpui_kit::Task<()>>,
}

impl WatchConnection {
    fn stop(&mut self, cx: &gpui_kit::App) {
        self.delivery.take();
        if let Some(watch) = self.watch.take() {
            let worker = watch.stop();
            self.joining = Some(cx.background_executor().spawn(async move {
                let _ = worker.join();
            }));
        }
    }
}

/// Connects only transport and lifecycle; app decides which notifications matter.
fn connect_watch(
    view: &gpui_kit::Entity<ui::Viewer>,
    cx: &mut gpui_kit::App,
) -> std::rc::Rc<std::cell::RefCell<WatchConnection>> {
    use std::{cell::RefCell, rc::Rc, time::Duration};
    let (watch, notices) = infra::FileWatch::new();
    let connection = Rc::new(RefCell::new(WatchConnection {
        watch: Some(watch),
        delivery: None,
        joining: None,
    }));
    let subscription_connection = connection.clone();
    cx.subscribe(view, move |_, target: &app::WatchTarget, _| {
        if let Some(watch) = subscription_connection.borrow().watch.as_ref() {
            watch.set_target(target.clone());
        }
    })
    .detach();
    let weak = view.downgrade();
    let executor = cx.background_executor().clone();
    let delivery = cx.spawn(async move |cx| {
        loop {
            for notice in notices.try_iter() {
                match notice {
                    // Registration closes the read-to-watch gap. Treat readiness
                    // like a save: the shared app path also defers behind a switch.
                    infra::WatchNotice::Changed(target)
                    | infra::WatchNotice::Ready(target, Ok(())) => {
                        if weak
                            .update(cx, |view, cx| view.watch_changed(&target, cx))
                            .is_err()
                        {
                            return;
                        }
                    }
                    infra::WatchNotice::Ready(target, Err(error)) => {
                        eprintln!("Watch {}: {error}", target.key.0.display());
                    }
                }
            }
            executor.timer(Duration::from_millis(20)).await;
        }
    });
    connection.borrow_mut().delivery = Some(delivery);
    let release_connection = connection.clone();
    view.update(cx, |_, cx| {
        cx.on_release(move |_, cx| release_connection.borrow_mut().stop(cx))
            .detach();
    });
    let quit_connection = connection.clone();
    cx.on_app_quit(move |cx| {
        let task = {
            let mut connection = quit_connection.borrow_mut();
            connection.stop(cx);
            connection.joining.take()
        };
        async move {
            if let Some(task) = task {
                task.await;
            }
        }
    })
    .detach();
    connection
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn parses_one_document_path_without_requiring_the_file_to_exist() {
        for path in ["notes/日本語 memo.md", r"C:\documents\sample.md"] {
            let args = Args::try_parse_from(["my-markdown-viewer", path]).unwrap();
            assert_eq!(args.file, PathBuf::from(path));
        }
    }

    #[test]
    fn rejects_missing_extra_and_unknown_arguments() {
        for (args, expected) in [
            (
                vec!["my-markdown-viewer"],
                ErrorKind::MissingRequiredArgument,
            ),
            (
                vec!["my-markdown-viewer", "first.md", "second.md"],
                ErrorKind::UnknownArgument,
            ),
            (
                vec!["my-markdown-viewer", "--unknown"],
                ErrorKind::UnknownArgument,
            ),
        ] {
            let error = Args::try_parse_from(args).unwrap_err();
            assert_eq!(error.kind(), expected);
            assert_eq!(error.exit_code(), 2);
        }
    }

    #[test]
    fn help_and_version_exit_successfully_without_a_document() {
        for (flag, expected) in [
            ("-h", ErrorKind::DisplayHelp),
            ("--help", ErrorKind::DisplayHelp),
            ("--version", ErrorKind::DisplayVersion),
        ] {
            let error = Args::try_parse_from(["my-markdown-viewer", flag]).unwrap_err();
            assert_eq!(error.kind(), expected);
            assert_eq!(error.exit_code(), 0);
        }
    }
}
