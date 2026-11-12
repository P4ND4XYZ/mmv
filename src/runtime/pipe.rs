use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use crate::{app, infra, ui};

type PendingReplies = BTreeMap<app::ReadRequestId, (app::ExternalRequestId, infra::Reply)>;
enum Delivery<T> {
    Disconnected,
    Message(Option<T>),
}
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
    fn release_disconnected(&mut self) {
        self.pending.retain(|_, (_, reply)| reply.is_alive());
    }
}

/// Transport only: app/Viewer owns acceptance, history and read ordering.
pub(crate) fn connect_pipe(
    view: &gpui_kit::Entity<ui::Viewer>,
    mut server: infra::PipeServer,
    cx: &mut gpui_kit::App,
) -> Rc<RefCell<PipeConnection>> {
    // These integration tests use real OS I/O, not the deterministic scheduler's
    // simulated I/O. Explicitly permit external-thread wakes in test-support.
    #[cfg(all(test, feature = "ui-tests"))]
    if cx.background_executor().is_test() {
        cx.background_executor().allow_parking();
    }
    let mut incoming = server.take_incoming().expect("one Pipe dispatcher");
    let disconnected = server.disconnect_notifications();
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
            // No timer or queue-full disconnect messages: Notify retains one
            // coalesced wake, while Reply's atomic state is authoritative.
            // Cross-thread Tokio wakes target GPUI's Send background task, not
            // its thread-local foreground task.
            // Only one event waiter exists; return the receiver for the next turn.
            let notifications = disconnected.clone();
            let (receiver, event) = executor
                .spawn(async move {
                    let event = tokio::select! {
                        _ = notifications.notified() => Delivery::Disconnected,
                        message = incoming.recv() => Delivery::Message(message),
                    };
                    (incoming, event)
                })
                .await;
            incoming = receiver;
            delivery_connection.borrow_mut().release_disconnected();
            let message = match event {
                Delivery::Disconnected => None,
                Delivery::Message(Some(message)) => Some(message),
                Delivery::Message(None) => return,
            };
            if let Some(message) = message {
                match message {
                    Err(error) => eprintln!("Pipe: {error}"),
                    Ok(incoming) => {
                        let id = incoming.request.id;
                        // Acceptance is still on GPUI, even if the CLI has just
                        // departed: disconnect never cancels an accepted operation.
                        let result = weak.update(cx, |view, cx| {
                            view.request_external(incoming.request.operation, id.clone(), cx)
                        });
                        match result {
                            Ok(app::OperationResult::Pending(request)) => {
                                // Race-safe if disconnect preceded delivery or
                                // happened during request_external. If it happens
                                // after this check, its stored Notify wakes us.
                                if incoming.reply.is_alive() {
                                    delivery_connection
                                        .borrow_mut()
                                        .pending
                                        .insert(request.id, (id, incoming.reply));
                                }
                            }
                            Ok(app::OperationResult::Complete { outcome, .. }) => {
                                let outcome = outcome.map_err(|error| match error {
                                    app::OperationError::DocumentUnavailable => {
                                        "document unavailable".to_string()
                                    }
                                    app::OperationError::NoPreviousEntry => {
                                        "no previous history entry".to_string()
                                    }
                                    app::OperationError::NoNextEntry => {
                                        "no next history entry".to_string()
                                    }
                                });
                                let _ = incoming.reply.send(infra::Response { id, outcome });
                            }
                            Err(_) => return,
                        }
                    }
                }
            }
            // A permanently ready producer must not monopolize the GPUI thread.
            // Yield the waker once per event; this is not time-based polling and
            // does not require a Tokio runtime on the GPUI executor.
            let mut yielded = false;
            std::future::poll_fn(|cx| {
                if std::mem::replace(&mut yielded, true) {
                    std::task::Poll::Ready(())
                } else {
                    cx.waker().wake_by_ref();
                    std::task::Poll::Pending
                }
            })
            .await;
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
