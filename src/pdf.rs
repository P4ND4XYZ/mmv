//! Thin PDF boundary. Loading, request adoption and host input policy live outside this module.
use std::{collections::VecDeque, fmt, rc::Rc, sync::Arc};

use crate::lua::pdf::{Operation as ZoomOperation, Target as ZoomTarget};
use gpui_kit::{canvas, div, prelude::*};

use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render, Subscription,
    Window,
};

#[derive(Clone, Default)]
pub(crate) struct PrepareCancellation {
    inner: gpui_pdf::PrepareCancellation,
}

impl PrepareCancellation {
    pub fn cancel(&self) {
        self.inner.cancel();
    }
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }
}

/// Only `prepare` can construct this uniquely owned, validated document.
pub(crate) struct PreparedPdf {
    inner: gpui_pdf::PreparedPdf,
}

impl PreparedPdf {
    pub fn page_dimensions(&self) -> &[(f32, f32)] {
        self.inner.page_dimensions()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrepareErrorKind {
    Locked,
    InvalidDocument,
    NoPages,
    InvalidDimensions { page: usize },
    Abandoned,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PrepareError {
    kind: PrepareErrorKind,
}

impl PrepareError {
    pub fn kind(&self) -> PrepareErrorKind {
        self.kind
    }
}

impl fmt::Display for PrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.kind {
            PrepareErrorKind::Locked => "PDF requires a password",
            PrepareErrorKind::InvalidDocument => "Unable to parse PDF",
            PrepareErrorKind::NoPages => "PDF has no pages",
            PrepareErrorKind::InvalidDimensions { .. } => "PDF has invalid page dimensions",
            PrepareErrorKind::Abandoned => "PDF preparation abandoned",
        })
    }
}
impl std::error::Error for PrepareError {}

// Test-only instrumentation on the deterministic executor's test thread; parallel
// test contexts must not count each other's independent preparations.
#[cfg(test)]
thread_local! { static PREPARE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
pub(crate) fn prepare_calls() -> usize {
    PREPARE_CALLS.with(std::cell::Cell::get)
}

/// CPU-only: the fork parses shared bytes once and validates every page's geometry.
pub(crate) fn prepare(
    bytes: Arc<Vec<u8>>,
    cancel: &PrepareCancellation,
) -> Result<PreparedPdf, PrepareError> {
    #[cfg(test)]
    PREPARE_CALLS.with(|count| count.set(count.get() + 1));
    gpui_pdf::prepare(bytes, &cancel.inner)
        .map(|inner| PreparedPdf { inner })
        .map_err(|error| {
            let kind = match error {
                gpui_pdf::PdfPrepareError::Locked => PrepareErrorKind::Locked,
                gpui_pdf::PdfPrepareError::InvalidDocument(_) => PrepareErrorKind::InvalidDocument,
                gpui_pdf::PdfPrepareError::NoPages => PrepareErrorKind::NoPages,
                gpui_pdf::PdfPrepareError::InvalidDimensions { page } => {
                    PrepareErrorKind::InvalidDimensions { page }
                }
                gpui_pdf::PdfPrepareError::Abandoned => PrepareErrorKind::Abandoned,
            };
            PrepareError { kind }
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperationStatus {
    Applied,
    Unchanged,
    DeferredUntilLayout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OperationOutcome {
    status: OperationStatus,
}
impl OperationOutcome {
    pub fn status(self) -> OperationStatus {
        self.status
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OperationErrorKind {
    Released,
    InvalidArgument,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OperationError {
    kind: OperationErrorKind,
}
impl OperationError {
    pub fn kind(self) -> OperationErrorKind {
        self.kind
    }
}

fn operation_result(
    result: Result<gpui_pdf::OperationOutcome, gpui_pdf::OperationError>,
) -> Result<OperationOutcome, OperationError> {
    result
        .map(|outcome| OperationOutcome {
            status: match outcome {
                gpui_pdf::OperationOutcome::Applied => OperationStatus::Applied,
                gpui_pdf::OperationOutcome::Unchanged => OperationStatus::Unchanged,
                gpui_pdf::OperationOutcome::DeferredUntilLayout => {
                    OperationStatus::DeferredUntilLayout
                }
            },
        })
        .map_err(|error| OperationError {
            kind: match error {
                gpui_pdf::OperationError::Released => OperationErrorKind::Released,
                gpui_pdf::OperationError::NonFinite => OperationErrorKind::InvalidArgument,
            },
        })
}

/// A notification only: the host chooses focus according to its current mode.
pub(crate) struct BodyClicked;

pub(crate) struct PdfView {
    inner: Entity<gpui_pdf::PdfView>,
    clicks: Option<Subscription>,
    zoom_target: ZoomTarget,
    pending_zoom: VecDeque<ZoomOperation>,
    layout_established: bool,
}
impl EventEmitter<BodyClicked> for PdfView {}

impl PdfView {
    /// Move prepared state directly into a host-controlled view: no I/O or parsing.
    pub fn from_prepared(prepared: PreparedPdf, cx: &mut Context<Self>) -> Self {
        let inner = cx.new(|cx| {
            gpui_pdf::PdfView::from_prepared(
                prepared.inner,
                gpui_pdf::Controls::Host,
                Rc::new(gpui_pdf::PdfStyle::default),
                Rc::new(|| 1.0),
                cx,
            )
        });
        let clicks = cx.subscribe(&inner, |_, _, event, cx| {
            if matches!(event, gpui_pdf::PdfEvent::BodyClicked) {
                cx.emit(BodyClicked);
            }
        });
        cx.on_release(|this, cx| this.release(None, cx)).detach();
        Self {
            inner,
            clicks: Some(clicks),
            zoom_target: ZoomTarget::default(),
            pending_zoom: VecDeque::new(),
            layout_established: false,
        }
    }

    pub(crate) fn target(&self) -> ZoomTarget {
        self.zoom_target.clone()
    }

    /// Keep relative operations symbolic until the preceding operation has applied.
    pub(crate) fn request_zoom(&mut self, operation: ZoomOperation, cx: &mut Context<Self>) {
        if self.is_released(cx) {
            return;
        }
        self.pending_zoom.push_back(operation);
        if self.layout_established {
            self.apply_zoom_requests(cx);
        }
        cx.notify();
    }

    fn apply_zoom_requests(&mut self, cx: &mut Context<Self>) {
        // A public zero-distance operation probes the child's actual scroll bounds.
        // If unlaid, only that harmless no-op enters the dependency's own queue;
        // our relative zooms stay here, preserving FIFO and applied-value reads.
        let ready = self.inner.update(cx, |view, cx| {
            matches!(view.scroll_by(0.0, cx), Ok(outcome) if outcome != gpui_pdf::OperationOutcome::DeferredUntilLayout)
        });
        if !ready {
            return;
        }
        if !self.layout_established {
            self.inner.update(cx, |view, cx| {
                let _ = view.set_fit_width(cx);
            });
            self.layout_established = true;
        }
        while let Some(operation) = self.pending_zoom.pop_front() {
            self.inner.update(cx, |view, cx| match operation {
                ZoomOperation::SetZoom(factor) => view.set_zoom(factor, cx),
                ZoomOperation::ZoomIn => view.set_zoom(view.zoom() * 1.1, cx),
                ZoomOperation::ZoomOut => view.set_zoom(view.zoom() / 1.1, cx),
                ZoomOperation::FitWidth => {
                    let _ = view.set_fit_width(cx);
                }
                ZoomOperation::FitPage => view.fit_page(cx),
            });
        }
        self.zoom_target.publish(self.zoom(cx));
    }

    pub fn scroll_by(
        &mut self,
        logical_y: f32,
        cx: &mut Context<Self>,
    ) -> Result<OperationOutcome, OperationError> {
        operation_result(
            self.inner
                .update(cx, |view, cx| view.scroll_by(logical_y, cx)),
        )
    }
    pub fn scroll_viewport(
        &mut self,
        fraction: f32,
        cx: &mut Context<Self>,
    ) -> Result<OperationOutcome, OperationError> {
        operation_result(
            self.inner
                .update(cx, |view, cx| view.scroll_viewport(fraction, cx)),
        )
    }
    pub fn scroll_to_edge(
        &mut self,
        end: bool,
        cx: &mut Context<Self>,
    ) -> Result<OperationOutcome, OperationError> {
        operation_result(self.inner.update(cx, |view, cx| {
            if end {
                view.scroll_to_end(cx)
            } else {
                view.scroll_to_start(cx)
            }
        }))
    }
    pub fn set_zoom(
        &mut self,
        factor: f32,
        cx: &mut Context<Self>,
    ) -> Result<OperationOutcome, OperationError> {
        operation_result(self.inner.update(cx, |view, cx| view.zoom_to(factor, cx)))
    }
    pub fn fit_width(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<OperationOutcome, OperationError> {
        operation_result(self.inner.update(cx, |view, cx| view.set_fit_width(cx)))
    }
    pub fn zoom(&self, cx: &App) -> f32 {
        self.inner.read(cx).zoom()
    }
    pub fn is_released(&self, cx: &App) -> bool {
        self.inner.read(cx).is_released()
    }

    /// Terminal and idempotent, including replacement or shutdown without a Window.
    pub fn release(&mut self, window: Option<&mut Window>, cx: &mut App) {
        self.clicks.take();
        self.pending_zoom.clear();
        self.zoom_target.release();
        self.inner
            .update(cx, |view, cx| view.release_resources(window, cx));
    }
}

impl Focusable for PdfView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.inner.read(cx).focus_handle(cx)
    }
}
impl Render for PdfView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        div()
            .size_full()
            .relative()
            .flex()
            .child(self.inner.clone())
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, _, cx| {
                        if f32::from(bounds.size.width) > 1.0 && f32::from(bounds.size.height) > 1.0
                        {
                            let weak = weak.clone();
                            cx.defer(move |cx| {
                                let _ = weak.update(cx, |view, cx| {
                                    if !view.is_released(cx) {
                                        view.apply_zoom_requests(cx);
                                    }
                                });
                            });
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
    }
}

#[cfg(test)]
pub(crate) mod tests;
