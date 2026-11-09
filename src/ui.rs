mod document;
mod input;
mod links;
mod mermaid;
mod search;
mod selection_panel;
mod status;
mod theme;

use std::{path::PathBuf, sync::Arc, time::Instant};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{prelude::*, *};

use crate::app::{
    self, DiagramRequest, DiagramResult, DocumentKey, DocumentOperation, ExternalOperation,
    ExternalRequestId, LinkTarget, OperationError, OperationOrigin, OperationResult,
    PendingMovementPolicy, ReadCompletion, ReadRequest, ReadResult, WatchTarget, resolve_link,
};
use document::{DocumentView, Movement};
use input::Prompt;
pub use input::bind_keys;
#[cfg(all(test, feature = "ui-tests"))]
use status::MODE_WIDTH;

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

pub struct Viewer {
    app: app::App,
    path: PathBuf,
    document: DocumentView,
    input: Prompt,
    body_focus: FocusHandle,
    link_panel: Option<links::LinkSelection>,
    restore_body_focus: bool,
    message: Option<String>,
    message_is_error: bool,
    display_error: Option<String>,
    config: crate::config::Config,
    palette: theme::Palette,
    config_notice: bool,
    // Links wait for an unresolved heading's layout so history records a
    // measured departure distance, not the new TextView's temporary zero.
    deferred_links: std::collections::VecDeque<DeferredLink>,
    next_operation_order: u64,
    latest_navigation_order: u64,
}

struct DeferredLink {
    target: Result<LinkTarget, String>,
    // A key survives reload generations, but a fragment must never be rebound
    // to a different document. Order also rejects newer requests to the same key.
    document: DocumentKey,
    order: u64,
}

impl EventEmitter<WatchTarget> for Viewer {}
impl EventEmitter<ReadRequest> for Viewer {}
impl EventEmitter<ReadCompletion> for Viewer {}
impl EventEmitter<DiagramRequest> for Viewer {}

impl Viewer {
    #[cfg(all(test, feature = "ui-tests"))]
    pub fn new(
        path: PathBuf,
        started: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_config(path, started, Default::default(), false, window, cx)
    }

    pub fn with_config(
        path: PathBuf,
        started: Instant,
        config: crate::config::Config,
        config_notice: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mode = match config.colorscheme {
            crate::config::ColorScheme::Dark => gpui_kit::component::ThemeMode::Dark,
            crate::config::ColorScheme::Light => gpui_kit::component::ThemeMode::Light,
        };
        gpui_kit::component::Theme::change(mode, None, cx);
        let palette = theme::Palette::resolve(&config, cx.theme());
        let document = DocumentView::new(started, cx);
        let input = Prompt::new(window, cx);
        let body_focus = cx.focus_handle();
        body_focus.focus(window, cx);
        Self {
            app: app::App::default(),
            path,
            document,
            input,
            body_focus,
            link_panel: None,
            restore_body_focus: false,
            message: None,
            message_is_error: false,
            display_error: None,
            config,
            palette,
            config_notice,
            deferred_links: Default::default(),
            next_operation_order: 0,
            latest_navigation_order: 0,
        }
    }

    #[cfg_attr(
        not(all(test, feature = "ui-tests")),
        expect(
            dead_code,
            reason = "Convenience adapter used by UI tests; production IPC uses request_external."
        )
    )]
    pub fn request_document(
        &mut self,
        path: PathBuf,
        response_to: Option<ExternalRequestId>,
        cx: &mut Context<Self>,
    ) -> OperationResult {
        self.request_operation(
            DocumentOperation::Open(path),
            OperationOrigin::External,
            response_to,
            cx,
        )
    }

    /// IPC receives Complete synchronously, or waits for ReadCompletion keyed by
    /// the returned ReadRequestId (external IDs can collide across connections).
    /// This adapter never changes input focus.
    pub fn request_external(
        &mut self,
        operation: ExternalOperation,
        response_to: ExternalRequestId,
        cx: &mut Context<Self>,
    ) -> OperationResult {
        let operation = match operation {
            ExternalOperation::Open(path) if self.app.document().is_none() => {
                DocumentOperation::Initial(path)
            }
            ExternalOperation::Open(path) => DocumentOperation::Open(path),
            ExternalOperation::Back => DocumentOperation::Back,
            ExternalOperation::Forward => DocumentOperation::Forward,
            movement => {
                let outcome = if self.app.document().is_none() {
                    Err(OperationError::DocumentUnavailable)
                } else {
                    let movement = match movement {
                        ExternalOperation::Scroll(direction) => Movement::Scroll(direction),
                        ExternalOperation::Top => Movement::Edge(false),
                        ExternalOperation::Bottom => Movement::Edge(true),
                        _ => unreachable!("document operations dispatched separately"),
                    };
                    self.next_operation_order += 1;
                    self.latest_navigation_order = self.next_operation_order;
                    self.deferred_links.clear();
                    self.queue_movement(movement, self.next_operation_order, cx);
                    Ok(())
                };
                cx.notify();
                return OperationResult::Complete {
                    response_to: Some(response_to),
                    outcome,
                };
            }
        };
        self.request_operation(operation, OperationOrigin::External, Some(response_to), cx)
    }

    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        self.request_operation(
            DocumentOperation::Reload,
            OperationOrigin::Keyboard,
            None,
            cx,
        );
    }

    /// Connection boundaries can respond immediately to Complete, or await the
    /// emitted read's ReadCompletion when Pending is returned.
    pub fn request_operation(
        &mut self,
        operation: DocumentOperation,
        origin: OperationOrigin,
        response_to: Option<ExternalRequestId>,
        cx: &mut Context<Self>,
    ) -> OperationResult {
        self.next_operation_order += 1;
        self.request_operation_in_order(
            operation,
            origin,
            response_to,
            self.next_operation_order,
            cx,
        )
    }

    fn request_operation_in_order(
        &mut self,
        operation: DocumentOperation,
        origin: OperationOrigin,
        response_to: Option<ExternalRequestId>,
        order: u64,
        cx: &mut Context<Self>,
    ) -> OperationResult {
        // Reload carries the current position intent across generations. Other
        // accepted operations supersede older clicks at acceptance, not adoption.
        let supersedes_links = !matches!(operation, DocumentOperation::Reload);
        self.settle_position(cx);
        let previous_position = self.app.position_request().map(|request| request.serial);
        let distance = self.departure_distance(cx);
        let result = self.app.operate(operation, origin, distance, response_to);
        // A synchronous position can replace earlier movement without a read.
        // Pending/failed reads leave the app's position intent unchanged.
        if self.app.position_request().is_some_and(|request| {
            Some(request.serial) != previous_position
                && request.pending_movements == PendingMovementPolicy::Replace
        }) {
            self.document.supersede_movements(order);
        }
        if supersedes_links
            && matches!(
                &result,
                OperationResult::Pending(_)
                    | OperationResult::Complete {
                        outcome: Ok(()),
                        ..
                    }
            )
        {
            self.latest_navigation_order = self.latest_navigation_order.max(order);
            self.deferred_links
                .retain(|link| link.order > self.latest_navigation_order);
        }
        if let OperationResult::Pending(request) = &result {
            cx.emit(request.clone());
        }
        cx.notify();
        result
    }

    fn open_link(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        // Resolve against the clicked document, even if execution waits for layout.
        self.next_operation_order += 1;
        self.follow_link(
            resolve_link(&self.path, url),
            self.next_operation_order,
            window,
            cx,
        );
    }

    fn follow_link(
        &mut self,
        target: Result<LinkTarget, String>,
        order: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settle_position(cx);
        if self
            .app
            .position_request()
            .is_some_and(|request| matches!(request.target, app::PositionTarget::Heading(_)))
        {
            self.deferred_links.push_back(DeferredLink {
                target,
                document: self.app.document().expect("heading document").key.clone(),
                order,
            });
            cx.notify();
            return;
        }
        self.message = None;
        self.display_error = None;
        match target {
            Ok(LinkTarget::File { path, fragment }) => {
                self.cancel_input(window, cx);
                self.body_focus.focus(window, cx);
                self.request_operation_in_order(
                    match fragment {
                        Some(id) => DocumentOperation::Heading {
                            path: Some(path),
                            id,
                        },
                        None => DocumentOperation::Open(path),
                    },
                    OperationOrigin::Link,
                    None,
                    order,
                    cx,
                );
            }
            Ok(LinkTarget::Web(url)) => cx.open_url(&url),
            Ok(LinkTarget::Heading(id)) => {
                self.cancel_input(window, cx);
                self.body_focus.focus(window, cx);
                self.request_operation_in_order(
                    DocumentOperation::Heading { path: None, id },
                    OperationOrigin::Link,
                    None,
                    order,
                    cx,
                );
            }
            Err(error) => self.display_error = Some(error),
        }
        cx.notify();
    }

    pub fn watch_changed(&mut self, target: &WatchTarget, cx: &mut Context<Self>) {
        if let Some(request) = self.app.watch_changed(target) {
            cx.emit(request);
            cx.notify();
        }
    }

    pub fn complete(&mut self, result: ReadResult, cx: &mut Context<Self>) {
        let previous_watch = self.app.watch_target().cloned();
        self.settle_position(cx);
        let distance = self.departure_distance(cx);
        let Some(completion) = self.app.complete_at(result, distance) else {
            return;
        };
        if completion.adopted {
            let panel_closed = self.link_panel.take().is_some();
            self.restore_body_focus |= panel_closed;
            let document = self.app.document().expect("accepted document");
            let generation = document.generation;
            let pending_movements = self
                .app
                .position_request()
                .expect("adoption produces a position request")
                .pending_movements;
            self.path = document.path.clone();
            self.message =
                panel_closed.then(|| "本文が更新されたためリンク選択を閉じました".into());
            self.message_is_error = false;
            self.document.replace(generation, pending_movements, cx);
            self.display_error = None;
            self.accept_rendered_text(generation, cx);
        }
        if self.app.watch_target() != previous_watch.as_ref()
            && let Some(target) = self.app.watch_target()
        {
            cx.emit(target.clone());
        }
        cx.emit(completion);
        if let Some(request) = self.app.take_automatic_reload() {
            cx.emit(request);
        }
        self.start_next_diagram(cx);
        cx.notify();
    }

    pub fn complete_diagram(&mut self, result: DiagramResult, cx: &mut Context<Self>) {
        if self.app.complete_diagram(&result)
            && let Ok(png) = result.png
        {
            self.document.diagram_images.insert(
                result.request.start,
                Arc::new(Image::from_bytes(ImageFormat::Png, png)),
            );
        }
        self.start_next_diagram(cx);
        cx.notify();
    }

    fn start_next_diagram(&mut self, cx: &mut Context<Self>) {
        if let Some(request) = self.app.next_diagram() {
            cx.emit(request);
        }
    }

    fn status_severity(&self) -> Option<crate::config::Group> {
        use crate::config::Group;
        if self.app.error().is_some() || self.display_error.is_some() {
            Some(Group::Error)
        } else if self.message.is_some() {
            Some(if self.message_is_error {
                Group::Error
            } else {
                Group::Warning
            })
        } else if self.config_notice {
            Some(Group::Error)
        } else {
            None
        }
    }

    fn status_text(&self) -> String {
        self.app
            .error()
            .or(self.display_error.as_deref())
            .map(str::to_owned)
            .or_else(|| self.message.clone())
            .or_else(|| {
                self.config_notice
                    .then(|| "設定を読み込めませんでした".into())
            })
            .unwrap_or_else(|| {
                if self.app.is_loading() {
                    "読み込み中…".into()
                } else {
                    String::new()
                }
            })
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.restore_body_focus) {
            // Logical focus only: never activate the OS window on external adoption.
            self.body_focus.focus(window, cx);
        }
        self.deferred_links.retain(|link| {
            link.order > self.latest_navigation_order
                && self
                    .app
                    .document()
                    .is_some_and(|document| document.key == link.document)
        });
        if self.app.position_request().is_none()
            && let Some(link) = self.deferred_links.pop_front()
        {
            self.follow_link(link.target, link.order, window, cx);
        }
        let path = self
            .app
            .document()
            .map(|doc| doc.path.clone())
            .unwrap_or_else(|| self.path.clone());
        let status = self.status_text();
        let severity = self.status_severity();
        let normal = self.palette.get(crate::config::Group::Normal);
        let body = self.render_document(path.clone(), window.viewport_size().width, cx);
        let status_line = status::render_status_line(&self.app.mode, &path, &self.palette);
        let command_line = status::render_command_line(
            &self.app.mode,
            &self.input.state,
            status,
            severity,
            &self.palette,
        );

        let overlay = self
            .link_panel
            .as_ref()
            .map(|selection| selection.panel.clone());
        input::bind_root_actions(div(), cx)
            .relative()
            .key_context("Evaluation")
            .size_full()
            .flex()
            .flex_col()
            .bg(normal.bg)
            .text_color(normal.fg)
            .font_family("Yu Gothic UI")
            .child(body)
            .child(status_line)
            .child(command_line)
            .children(overlay)
    }
}
