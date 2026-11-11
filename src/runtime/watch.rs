use std::{cell::RefCell, rc::Rc};

use crate::{app, infra, ui};

pub(crate) struct WatchConnection {
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
pub(crate) fn connect_watch(
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

#[cfg(all(test, feature = "ui-tests"))]
impl WatchConnection {
    pub(crate) fn is_stopped(&self) -> bool {
        self.watch.is_none() && self.delivery.is_none()
    }

    pub(crate) fn take_joining(&mut self) -> Option<gpui_kit::Task<()>> {
        self.joining.take()
    }
}
