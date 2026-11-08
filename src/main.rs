extern crate gpui_kit as gpui;

mod app;
mod cli;
mod config;
mod core;
mod infra;
mod ui;

use gpui_kit::{AppContext as _, WindowOptions, assets::Assets};
use std::{cell::RefCell, collections::BTreeMap, path::PathBuf, rc::Rc, sync::Arc, time::Instant};

fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() == 2 && args[1] == infra::INTERNAL_GUI_ARGUMENT {
        return print_output(cli::execution_output(run_gui()));
    }
    // Help/version/usage are handled before any GUI or Windows resources exist.
    let cwd = std::env::current_dir();
    let operation = match cli::parse_from(args, cwd.as_deref().unwrap_or(std::path::Path::new("")))
    {
        Ok(operation) => operation,
        Err(output) => return print_output(output),
    };
    let result = (|| -> Result<(), String> {
        let namespace = infra::Namespace::current_user().map_err(|error| error.to_string())?;
        let client =
            infra::connect_or_start(&namespace, &operation).map_err(|error| error.to_string())?;
        // Independent of app generations. One invocation sends exactly one request.
        let id = app::ExternalRequestId(format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        client
            .request(
                infra::Request { id, operation },
                infra::Deadline::response(Instant::now()),
            )
            .map_err(|error| error.to_string())?
            .outcome
    })();
    print_output(cli::execution_output(result))
}

fn print_output(output: cli::CliOutput) -> std::process::ExitCode {
    print!("{}", output.stdout);
    eprint!("{}", output.stderr);
    std::process::ExitCode::from(output.exit_code)
}

fn run_gui() -> std::io::Result<()> {
    let namespace = infra::Namespace::current_user()?;
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
            let (_, view) = gpui_kit::open_window(
                WindowOptions {
                    focus: false,
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("Markdown Viewer");
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
            )
            .expect("failed to open viewer window");
            connect_document_reads(&view, cx);
            cx.subscribe(&view, |view, request: &app::DiagramRequest, cx| {
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
            connect_watch(&view, cx);
            connect_pipe(&view, server, cx);
        });
    Ok(())
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

type PendingReplies = BTreeMap<app::ReadRequestId, (app::ExternalRequestId, infra::Reply)>;
struct PipeConnection {
    server: Option<infra::PipeServer>,
    pending: PendingReplies,
    delivery: Option<gpui_kit::Task<()>>,
    joining: Option<gpui_kit::Task<()>>,
}
impl PipeConnection {
    fn stop(&mut self, cx: &gpui_kit::App) {
        self.delivery.take();
        self.pending.clear();
        if let Some(mut server) = self.server.take() {
            server.request_shutdown();
            self.joining = Some(cx.background_executor().spawn(async move {
                server.shutdown();
            }));
        }
    }
}

/// Transport only: app/Viewer owns acceptance, history and read ordering.
fn connect_pipe(
    view: &gpui_kit::Entity<ui::Viewer>,
    server: infra::PipeServer,
    cx: &mut gpui_kit::App,
) -> Rc<RefCell<PipeConnection>> {
    let connection = Rc::new(RefCell::new(PipeConnection {
        server: Some(server),
        pending: BTreeMap::new(),
        delivery: None,
        joining: None,
    }));
    let completion_connection = connection.clone();
    cx.subscribe(view, move |_, completion: &app::ReadCompletion, _| {
        let pending = completion_connection
            .borrow_mut()
            .pending
            .remove(&completion.request_id);
        if let Some((id, reply)) = pending {
            let _ = reply.send(infra::Response {
                id,
                outcome: completion.outcome.clone(),
            });
        }
    })
    .detach();
    let delivery_connection = connection.clone();
    let weak = view.downgrade();
    let executor = cx.background_executor().clone();
    let delivery = cx.spawn(async move |cx| {
        loop {
            let incoming: Vec<_> = {
                let mut connection = delivery_connection.borrow_mut();
                // A read may never finish; disconnected/timed-out clients must not
                // keep their response destination until that read completes.
                connection.pending.retain(|_, (_, reply)| reply.is_alive());
                let Some(server) = &connection.server else {
                    return;
                };
                server.incoming.try_iter().collect()
            };
            for incoming in incoming {
                let incoming = match incoming {
                    Ok(incoming) => incoming,
                    Err(error) => {
                        eprintln!("Pipe: {error}");
                        continue;
                    }
                };
                let id = incoming.request.id;
                let result = weak.update(cx, |view, cx| {
                    view.request_external(incoming.request.operation, id.clone(), cx)
                });
                match result {
                    Ok(app::OperationResult::Pending(request)) => {
                        delivery_connection
                            .borrow_mut()
                            .pending
                            .insert(request.id, (id, incoming.reply));
                    }
                    Ok(app::OperationResult::Complete { outcome, .. }) => {
                        let outcome = outcome.map_err(|error| match error {
                            app::OperationError::DocumentUnavailable => {
                                "document unavailable".to_string()
                            }
                            app::OperationError::NoPreviousEntry => {
                                "no previous history entry".to_string()
                            }
                            app::OperationError::NoNextEntry => "no next history entry".to_string(),
                        });
                        let _ = incoming.reply.send(infra::Response { id, outcome });
                    }
                    Err(_) => return,
                }
            }
            executor.timer(std::time::Duration::from_millis(10)).await;
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
) -> Rc<RefCell<WatchConnection>> {
    use std::time::Duration;
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
