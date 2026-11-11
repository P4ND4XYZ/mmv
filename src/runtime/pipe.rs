use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use crate::{app, infra, ui};

type PendingReplies = BTreeMap<app::ReadRequestId, (app::ExternalRequestId, infra::Reply)>;
pub(crate) struct PipeConnection {
    server: Option<infra::PipeServer>,
    pending: PendingReplies,
    delivery: Option<gpui_kit::Task<()>>,
    joining: Option<gpui_kit::Task<()>>,
}
impl PipeConnection {
    pub(crate) fn stop(&mut self, cx: &gpui_kit::App) {
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
pub(crate) fn connect_pipe(
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

#[cfg(all(test, feature = "ui-tests"))]
impl PipeConnection {
    pub(crate) fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.server.is_none()
    }

    pub(crate) fn take_joining(&mut self) -> Option<gpui_kit::Task<()>> {
        self.joining.take()
    }
}
