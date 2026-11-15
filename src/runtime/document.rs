mod registry;

use crate::{app, infra, pdf, ui};
use std::{cell::RefCell, rc::Rc, sync::Arc};

/// Only the I/O + actual prepare path can construct a PDF delivery.
pub(crate) struct PreparedRead {
    inner: PreparedKind,
    failure: Option<infra::ReadFailureKind>,
}
enum PreparedKind {
    Response(app::ReadResponse),
    Pdf {
        request: app::ReadRequest,
        key: app::DocumentKey,
        bytes: Arc<Vec<u8>>,
        prepared: pdf::PreparedPdf,
    },
    Abandoned(app::ReadRequest),
}

impl PreparedRead {
    pub(crate) fn failure(&self) -> Option<infra::ReadFailureKind> {
        self.failure
    }

    pub(crate) fn with_failure(mut self, failure: Option<infra::ReadFailureKind>) -> Self {
        self.failure = failure;
        self
    }

    pub(crate) fn request(&self) -> &app::ReadRequest {
        match &self.inner {
            PreparedKind::Response(app::ReadResponse::Resolved { request, .. })
            | PreparedKind::Pdf { request, .. }
            | PreparedKind::Abandoned(request) => request,
            PreparedKind::Response(app::ReadResponse::Loaded(result)) => &result.request,
        }
    }

    pub(crate) fn is_abandoned(&self) -> bool {
        matches!(self.inner, PreparedKind::Abandoned(_))
    }

    pub(crate) fn same_file_resolution(&self, key: Option<&app::DocumentKey>) -> bool {
        matches!(&self.inner, PreparedKind::Response(app::ReadResponse::Resolved { request, key: Ok(resolved) })
            if request.skip_same && key == Some(resolved))
    }

    pub(crate) fn into_parts(self) -> Option<(app::ReadResponse, Option<pdf::PreparedPdf>)> {
        match self.inner {
            PreparedKind::Response(result) => Some((result, None)),
            PreparedKind::Pdf {
                request,
                key,
                bytes,
                prepared,
            } => Some((
                app::ReadResult {
                    request,
                    content: Ok(app::LoadedDocument {
                        key,
                        content: app::DocumentContent::Pdf { bytes },
                    }),
                }
                .into(),
                Some(prepared),
            )),
            PreparedKind::Abandoned(_) => None,
        }
    }
}

impl From<app::ReadResponse> for PreparedRead {
    fn from(mut response: app::ReadResponse) -> Self {
        // Compatibility fixture/Markdown callers cannot introduce raw PDF success.
        if let app::ReadResponse::Loaded(result) = &mut response
            && result
                .content
                .as_ref()
                .is_ok_and(|loaded| loaded.content.format() == app::DocumentFormat::Pdf)
        {
            result.content = Err("PDF success requires actual preparation".into());
        }
        Self {
            inner: PreparedKind::Response(response),
            failure: None,
        }
    }
}
impl From<app::ReadResult> for PreparedRead {
    fn from(result: app::ReadResult) -> Self {
        app::ReadResponse::from(result).into()
    }
}

fn read_and_prepare(request: app::ReadRequest, cancel: &pdf::PrepareCancellation) -> PreparedRead {
    if cancel.is_cancelled() {
        return PreparedRead {
            inner: PreparedKind::Abandoned(request),
            failure: None,
        };
    }
    let (result, failure) = infra::read_document_input_classified(request);
    match result {
        infra::DocumentRead::Response(result) => PreparedRead::from(result).with_failure(failure),
        infra::DocumentRead::PdfBytes {
            request,
            key,
            bytes,
        } => {
            if cancel.is_cancelled() {
                return PreparedRead {
                    inner: PreparedKind::Abandoned(request),
                    failure: None,
                };
            }
            match pdf::prepare(bytes.clone(), cancel) {
                Ok(prepared) => PreparedRead {
                    inner: PreparedKind::Pdf {
                        request,
                        key,
                        bytes,
                        prepared,
                    },
                    failure: None,
                },
                Err(error) if error.kind() == pdf::PrepareErrorKind::Abandoned => PreparedRead {
                    inner: PreparedKind::Abandoned(request),
                    failure: None,
                },
                Err(error) => app::ReadResult {
                    request,
                    content: Err(error.to_string()),
                }
                .into(),
            }
        }
    }
}

pub(crate) fn connect_document_reads(
    view: &gpui_kit::Entity<ui::Viewer>,
    cx: &mut gpui_kit::App,
) -> Rc<RefCell<registry::ReadRegistry>> {
    let registry = Rc::new(RefCell::new(registry::ReadRegistry::default()));
    let requests = registry.clone();
    cx.subscribe(view, move |view, request: &app::ReadRequest, cx| {
        if requests.borrow().is_stopped() || view.read(cx).read_request_status(request).is_none() {
            return;
        }
        view.update(cx, |view, cx| requests.borrow_mut().cancel_stale(view, cx));
        let request = request.clone();
        let Some(cancel) = requests.borrow_mut().begin(request.clone()) else {
            return;
        };
        let background_cancel = cancel.clone();
        let identity = request.clone();
        let result = cx.background_executor().spawn(async move {
            let result = read_and_prepare(request, &background_cancel);
            #[cfg(test)]
            tests::after_prepare(result.request()).await;
            result
        });
        let weak = view.downgrade();
        let collecting = requests.clone();
        let task_request = identity.clone();
        let job = cx.spawn(async move |cx| {
            // This collector owns and awaits the noninterruptible background job,
            // even after the viewer/window is gone or cancellation was requested.
            let result = result.await;
            let result = if cancel.is_cancelled() {
                PreparedRead {
                    inner: PreparedKind::Abandoned(result.request().clone()),
                    failure: None,
                }
            } else {
                result
            };
            let deliver = collecting.borrow_mut().ready(result.request());
            if deliver {
                let request = result.request().clone();
                let abandoned = result.is_abandoned();
                if weak
                    .update(cx, |view, cx| view.complete(result, cx))
                    .is_err()
                    || abandoned
                {
                    collecting.borrow_mut().retire(&request);
                }
            } else {
                collecting.borrow_mut().retire(&task_request);
            }
        });
        requests.borrow_mut().set_job(&identity, job);
    })
    .detach();
    let changed = registry.clone();
    cx.subscribe(view, move |view, _: &ui::ReadStateChanged, cx| {
        view.update(cx, |view, cx| changed.borrow_mut().cancel_stale(view, cx));
    })
    .detach();
    let completions = registry.clone();
    cx.subscribe(view, move |_, completion: &app::ReadCompletion, _| {
        completions.borrow_mut().completed(completion.request_id);
    })
    .detach();
    let released = registry.clone();
    view.update(cx, |_, cx| {
        cx.on_release(move |_, _| released.borrow_mut().stop())
            .detach();
    });
    let closed = registry.clone();
    let weak = view.downgrade();
    let window_id = view.read(cx).document_window_id();
    cx.on_window_closed(move |cx, closed_window| {
        if closed_window == window_id {
            closed.borrow_mut().stop();
            let _ = weak.update(cx, |view, cx| view.shutdown_document_reads(cx));
        }
    })
    .detach();
    let quitting = registry.clone();
    let weak = view.downgrade();
    cx.on_app_quit(move |cx| {
        let jobs = quitting.borrow_mut().stop_and_collect();
        let _ = weak.update(cx, |view, cx| view.shutdown_document_reads(cx));
        async move {
            for job in jobs {
                job.await;
            }
        }
    })
    .detach();
    registry
}

#[cfg(test)]
pub(crate) fn test_read(request: app::ReadRequest) -> PreparedRead {
    read_and_prepare(request, &pdf::PrepareCancellation::default())
}

#[cfg(test)]
pub(crate) mod tests;
