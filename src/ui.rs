mod document;
mod input;
mod mermaid;
mod search;
mod status;

use std::{path::PathBuf, sync::Arc, time::Instant};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{prelude::*, *};

use crate::app::{
    self, DiagramRequest, DiagramResult, ExternalRequestId, LinkTarget, ReadCompletion,
    ReadRequest, ReadResult, resolve_link,
};
use document::DocumentView;
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
    message: Option<String>,
    display_error: Option<String>,
}

impl EventEmitter<ReadRequest> for Viewer {}
impl EventEmitter<ReadCompletion> for Viewer {}
impl EventEmitter<DiagramRequest> for Viewer {}

impl Viewer {
    pub fn new(
        path: PathBuf,
        started: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
            message: None,
            display_error: None,
        }
    }

    pub fn open(&mut self, cx: &mut Context<Self>) {
        let request = self.app.open(self.path.clone());
        cx.emit(request);
        cx.notify();
    }

    pub fn request_document(
        &mut self,
        path: PathBuf,
        response_to: Option<ExternalRequestId>,
        cx: &mut Context<Self>,
    ) {
        let request = self.app.open_request(path, response_to);
        cx.emit(request);
        cx.notify();
    }

    fn open_link(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.message = None;
        self.display_error = None;
        match resolve_link(&self.path, url) {
            Ok(LinkTarget::File(path)) => {
                self.cancel_input(window, cx);
                self.body_focus.focus(window, cx);
                self.request_document(path, None, cx);
            }
            Ok(LinkTarget::Web(url)) => cx.open_url(&url),
            Ok(LinkTarget::Heading) => self.message = Some("見出しへの移動は未対応です".into()),
            Err(error) => self.display_error = Some(error),
        }
        cx.notify();
    }

    pub fn complete(&mut self, result: ReadResult, cx: &mut Context<Self>) {
        let Some(completion) = self.app.complete(result) else {
            return;
        };
        if completion.adopted {
            let document = self.app.document().expect("accepted document");
            let generation = document.generation;
            self.path = document.path.clone();
            self.message = None;
            self.document.replace(generation, cx);
            self.display_error = None;
            self.accept_rendered_text(generation, cx);
        }
        cx.emit(completion);
        self.start_next_diagram(cx);
        cx.notify();
    }

    pub fn complete_diagram(&mut self, result: DiagramResult, cx: &mut Context<Self>) {
        if self.app.complete_diagram(&result) {
            match result.png {
                Ok(png) => {
                    self.document.diagram_images.insert(
                        result.request.start,
                        Arc::new(Image::from_bytes(ImageFormat::Png, png)),
                    );
                }
                Err(error) => {
                    eprintln!(
                        "Mermaid {} byte {}: {error}",
                        result.request.path.display(),
                        result.request.start
                    );
                    self.message = Some(format!("Mermaidの生成に失敗しました: {error}"));
                }
            }
        }
        self.start_next_diagram(cx);
        cx.notify();
    }

    fn start_next_diagram(&mut self, cx: &mut Context<Self>) {
        if let Some(request) = self.app.next_diagram() {
            cx.emit(request);
        }
    }
}

impl Render for Viewer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let path = self
            .app
            .document()
            .map(|doc| doc.path.clone())
            .unwrap_or_else(|| self.path.clone());
        let status = self
            .app
            .error()
            .or(self.display_error.as_deref())
            .map(str::to_owned)
            .or_else(|| self.message.clone())
            .unwrap_or_else(|| {
                if self.app.is_loading() {
                    "読み込み中…".into()
                } else {
                    String::new()
                }
            });

        let body = self.render_document(path.clone(), cx);
        let status_line = status::render_status_line(&self.app.mode, &path, cx);
        let command_line =
            status::render_command_line(&self.app.mode, &self.input.state, status, cx);

        input::bind_root_actions(div(), cx)
            .key_context("Evaluation")
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family("Yu Gothic UI")
            .child(body)
            .child(status_line)
            .child(command_line)
    }
}
