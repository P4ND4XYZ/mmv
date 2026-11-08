mod mermaid;
mod search;

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use gpui_kit::component::{
    ActiveTheme as _,
    input::{Input, InputEvent, InputState},
    text::{RangeHighlight, TextView, TextViewState},
};
use gpui_kit::{prelude::*, *};

use crate::app::{
    self, DiagramRequest, DiagramResult, Generation, ImageLocation, LinkTarget, Mode, ReadRequest,
    ReadResult, SearchOutcome, resolve_image, resolve_link,
};
use mermaid::{DiscoveredDiagrams, MermaidPlugin};

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

actions!(
    evaluation,
    [
        Find,
        Command,
        NextMatch,
        PreviousMatch,
        Reload,
        ScrollDown,
        ScrollUp,
        HalfPageDown,
        HalfPageUp,
        PageDown,
        PageUp,
        FirstLine,
        LastLine
    ]
);

const FOOTER_LINE_HEIGHT: f32 = 20.;
const MODE_WIDTH: f32 = 96.;

pub fn bind_keys(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        KeyBinding::new("/", Find, Some("EvaluationNormal")),
        KeyBinding::new(":", Command, Some("EvaluationNormal")),
        KeyBinding::new("n", NextMatch, Some("EvaluationNormal")),
        KeyBinding::new("shift-n", PreviousMatch, Some("EvaluationNormal")),
        KeyBinding::new("f5", Reload, Some("EvaluationNormal")),
        KeyBinding::new("j", ScrollDown, Some("EvaluationNormal")),
        KeyBinding::new("k", ScrollUp, Some("EvaluationNormal")),
        KeyBinding::new("ctrl-d", HalfPageDown, Some("EvaluationNormal")),
        KeyBinding::new("ctrl-u", HalfPageUp, Some("EvaluationNormal")),
        KeyBinding::new("ctrl-f", PageDown, Some("EvaluationNormal")),
        KeyBinding::new("ctrl-b", PageUp, Some("EvaluationNormal")),
        KeyBinding::new("g g", FirstLine, Some("EvaluationNormal")),
        KeyBinding::new("shift-g", LastLine, Some("EvaluationNormal")),
    ]);
}

pub struct Viewer {
    app: app::App,
    path: PathBuf,
    text: Entity<TextViewState>,
    text_subscription: Option<Subscription>,
    discovered_diagrams: DiscoveredDiagrams,
    diagram_images: BTreeMap<usize, Arc<Image>>,
    find: Entity<InputState>,
    _input_subscription: Subscription,
    body_focus: FocusHandle,
    message: Option<String>,
    display_error: Option<String>,
    search_blocks: Option<(Generation, Vec<Option<std::ops::Range<usize>>>)>,
    started: Instant,
    first_paint: Arc<AtomicBool>,
}

impl EventEmitter<ReadRequest> for Viewer {}
impl EventEmitter<DiagramRequest> for Viewer {}

impl Viewer {
    pub fn new(
        path: PathBuf,
        started: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = cx.new(|cx| TextViewState::markdown("", cx));
        let find = cx.new(|cx| InputState::new(window, cx));
        let input_subscription =
            cx.subscribe_in(&find, window, |this, input, event, window, cx| {
                match event {
                    InputEvent::Change => {
                        if matches!(this.app.mode, Mode::Search { .. }) {
                            this.app.preview_search(input.read(cx).value().to_string());
                            this.apply_search(false, cx);
                        }
                    }
                    InputEvent::PressEnter { .. } => {
                        // The input emits Enter even while it owns an IME marked range.
                        let composing = input.update(cx, |input, cx| {
                            input.marked_text_range(window, cx).is_some()
                        });
                        if !composing {
                            this.confirm_input(window, cx);
                        }
                    }
                    _ => {}
                }
                cx.notify();
            });
        let body_focus = cx.focus_handle();
        body_focus.focus(window, cx);
        Self {
            app: app::App::default(),
            path,
            text,
            text_subscription: None,
            discovered_diagrams: DiscoveredDiagrams::default(),
            diagram_images: BTreeMap::new(),
            find,
            _input_subscription: input_subscription,
            body_focus,
            message: None,
            display_error: None,
            search_blocks: None,
            started,
            first_paint: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn open(&mut self, cx: &mut Context<Self>) {
        let request = self.app.open(self.path.clone());
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
                let request = self.app.open(path);
                cx.emit(request);
            }
            Ok(LinkTarget::Web(url)) => cx.open_url(&url),
            Ok(LinkTarget::Heading) => self.message = Some("見出しへの移動は未対応です".into()),
            Err(error) => self.display_error = Some(error),
        }
        cx.notify();
    }

    pub fn complete(&mut self, result: ReadResult, cx: &mut Context<Self>) {
        if self.app.complete(result) {
            let document = self.app.document.as_ref().expect("accepted document");
            let generation = document.generation;
            self.path = document.path.clone();
            self.search_blocks = None;
            self.message = None;
            self.discovered_diagrams = DiscoveredDiagrams::default();
            for image in std::mem::take(&mut self.diagram_images).into_values() {
                image.remove_asset(cx);
            }
            // Install the Mermaid extension before parsing any source, including large
            // documents, so raw diagram code is never used as an initial fallback.
            self.text = cx.new(|cx| TextViewState::markdown("", cx));
            self.text_subscription = Some(cx.observe(&self.text, move |this, _, cx| {
                this.accept_rendered_text(generation, cx);
            }));
            self.display_error = None;
            self.accept_rendered_text(generation, cx);
        }
        self.start_next_diagram(cx);
        cx.notify();
    }

    pub fn complete_diagram(&mut self, result: DiagramResult, cx: &mut Context<Self>) {
        if self.app.complete_diagram(&result) {
            match result.png {
                Ok(png) => {
                    self.diagram_images.insert(
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

    fn accept_rendered_text(&mut self, generation: Generation, cx: &mut Context<Self>) {
        let snapshot = self.text.read(cx).rendered_text();
        let Some(document) = self.app.document.as_ref() else {
            return;
        };
        if document.generation != generation || snapshot.source() != document.source {
            return;
        }
        let discovered =
            std::mem::take(&mut *self.discovered_diagrams.lock().expect("diagram inbox"));
        self.app.discover_diagrams(discovered);
        self.start_next_diagram(cx);
        if self.app.rendered(generation, snapshot.as_str()) {
            self.apply_search(false, cx);
            cx.notify();
        }
    }

    fn apply_search(&mut self, reveal: bool, cx: &mut Context<Self>) {
        let Some(document) = self.app.document.as_ref() else {
            return;
        };
        if self.text.read(cx).rendered_text().source() != document.source {
            return;
        }
        let current = self.app.current_match;
        let highlights = self
            .app
            .matches
            .iter()
            .enumerate()
            .map(|(ix, range)| {
                RangeHighlight::new(
                    range.clone(),
                    if Some(ix) == current {
                        rgb(0xffb347).opacity(0.65)
                    } else {
                        rgb(0xffe28a).opacity(0.45)
                    },
                )
            })
            .collect::<Vec<_>>();
        let target = current.and_then(|ix| self.app.matches.get(ix)).cloned();
        let result = self.text.update(cx, |text, cx| {
            text.set_range_highlights(highlights, cx)?;
            if let Some(target) = target.filter(|_| reveal) {
                text.reveal_range(target, cx)?;
            }
            Ok::<_, gpui_kit::component::text::RangeHighlightError>(())
        });
        self.display_error = result.err().map(|error| error.to_string());
    }

    fn begin_input(&mut self, command: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.app.mode != Mode::Normal {
            return;
        }
        self.message = None;
        self.display_error = None;
        self.app.error = None;
        if command {
            self.app.begin_command();
        } else {
            self.app.begin_search();
            self.apply_search(false, cx);
        }
        self.find.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn cancel_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.app.cancel_input();
        self.apply_search(false, cx);
        self.body_focus.focus(window, cx);
        cx.notify();
    }

    fn confirm_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.find.read(cx).value().to_string();
        match self.app.mode {
            Mode::Search { .. } => {
                let from = match self.search_origin(cx) {
                    Ok(from) => from,
                    Err(error) => {
                        self.cancel_input(window, cx);
                        self.message = Some(error);
                        return;
                    }
                };
                let outcome = self.app.confirm_search(input, from);
                self.apply_search(outcome == SearchOutcome::Found, cx);
                self.message = match outcome {
                    SearchOutcome::Found => None,
                    SearchOutcome::NotFound => Some("一致する文字列がありません".into()),
                    SearchOutcome::NoPreviousQuery => Some("直前の検索語がありません".into()),
                };
            }
            Mode::Command => {
                self.app.cancel_input();
                if input == "q" {
                    window.remove_window();
                    return;
                }
                self.message = Some(format!("未対応のコマンド: :{input}"));
            }
            Mode::Normal => return,
        }
        self.body_focus.focus(window, cx);
        cx.notify();
    }

    fn scroll(&mut self, lines: f32, pages: f32, cx: &mut Context<Self>) {
        if self.app.mode != Mode::Normal {
            return;
        }
        let list = self.text.read(cx).list_state();
        list.scroll_by(px(FOOTER_LINE_HEIGHT * lines) + list.viewport_bounds().size.height * pages);
        self.text.update(cx, |_, cx| cx.notify());
    }

    fn scroll_edge(&mut self, end: bool, cx: &mut Context<Self>) {
        if self.app.mode != Mode::Normal {
            return;
        }
        let list = self.text.read(cx).list_state();
        if end {
            list.scroll_to_end();
        } else {
            list.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
        }
        self.text.update(cx, |_, cx| cx.notify());
    }

    fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.app.mode != Mode::Normal {
            return;
        }
        self.app.step_match(forward);
        self.apply_search(true, cx);
        cx.notify();
    }
}

impl Render for Viewer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let path = self
            .app
            .document
            .as_ref()
            .map(|doc| doc.path.clone())
            .unwrap_or_else(|| self.path.clone());
        let ready = self
            .app
            .document
            .as_ref()
            .is_some_and(|doc| self.text.read(cx).rendered_text().source() == doc.source);
        // The child installs extensions during render. Only then hand it the source,
        // and read synchronous results here as well as asynchronous notifications.
        if let Some(generation) = self.app.document.as_ref().map(|doc| doc.generation) {
            let view = cx.entity().downgrade();
            cx.defer(move |cx| {
                let _ = view.update(cx, |this, cx| {
                    if let Some(document) = this
                        .app
                        .document
                        .as_ref()
                        .filter(|doc| doc.generation == generation)
                    {
                        this.text
                            .update(cx, |text, cx| text.set_text(&document.source, cx));
                        this.accept_rendered_text(generation, cx);
                    }
                });
            });
        }
        let first_paint = self.first_paint.clone();
        let started = self.started;
        let weak = cx.entity().downgrade();
        let normal = self.app.mode == Mode::Normal;
        let (mode_background, mode_foreground) = match self.app.mode {
            Mode::Normal => (rgb(0x2563eb), rgb(0xffffff)),
            Mode::Search { .. } => (rgb(0xfbbf24), rgb(0x111827)),
            Mode::Command => (rgb(0x7c3aed), rgb(0xffffff)),
        };
        let filename = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned();
        let status = self
            .app
            .error
            .as_ref()
            .or(self.display_error.as_ref())
            .cloned()
            .or_else(|| self.message.clone())
            .unwrap_or_else(|| {
                if self.app.is_loading() {
                    "読み込み中…".into()
                } else {
                    String::new()
                }
            });

        div()
            .key_context("Evaluation")
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family("Yu Gothic UI")
            .on_action(cx.listener(|this, _: &Reload, _, cx| {
                if this.app.mode == Mode::Normal {
                    this.open(cx);
                }
            }))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "backspace"
                    && event.keystroke.modifiers == Modifiers::default()
                    && this.app.mode == Mode::Command
                    && this.find.read(cx).value().is_empty()
                {
                    let composing = this.find.update(cx, |input, cx| {
                        input.marked_text_range(window, cx).is_some()
                    });
                    if !composing {
                        this.cancel_input(window, cx);
                        cx.stop_propagation();
                    }
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.cancel_input(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .debug_selector(|| "document-body".to_string())
                    .key_context(if normal {
                        "EvaluationNormal"
                    } else {
                        "EvaluationEditing"
                    })
                    .track_focus(&self.body_focus)
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .on_action(cx.listener(|this, _: &Find, window, cx| {
                        this.begin_input(false, window, cx);
                    }))
                    .on_action(cx.listener(|this, _: &Command, window, cx| {
                        this.begin_input(true, window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &ScrollDown, _, cx| this.scroll(1., 0., cx)))
                    .on_action(cx.listener(|this, _: &ScrollUp, _, cx| this.scroll(-1., 0., cx)))
                    .on_action(
                        cx.listener(|this, _: &HalfPageDown, _, cx| this.scroll(0., 0.5, cx)),
                    )
                    .on_action(cx.listener(|this, _: &HalfPageUp, _, cx| this.scroll(0., -0.5, cx)))
                    .on_action(cx.listener(|this, _: &PageDown, _, cx| this.scroll(0., 1., cx)))
                    .on_action(cx.listener(|this, _: &PageUp, _, cx| this.scroll(0., -1., cx)))
                    .on_action(
                        cx.listener(|this, _: &FirstLine, _, cx| this.scroll_edge(false, cx)),
                    )
                    .on_action(cx.listener(|this, _: &LastLine, _, cx| this.scroll_edge(true, cx)))
                    .on_action(cx.listener(|this, _: &NextMatch, _, cx| this.step_match(true, cx)))
                    .on_action(
                        cx.listener(|this, _: &PreviousMatch, _, cx| this.step_match(false, cx)),
                    )
                    .child(
                        TextView::new(&self.text)
                            .plugin(MermaidPlugin {
                                discovered: self.discovered_diagrams.clone(),
                                images: self.diagram_images.clone(),
                            })
                            .scrollable(true)
                            .size_full()
                            .px_4()
                            .py_3()
                            .image_source(move |uri| match resolve_image(&path, uri.as_ref()) {
                                Ok(ImageLocation::File(path)) => path.into(),
                                Ok(ImageLocation::Url(url)) => {
                                    ImageSource::from(SharedUri::from(url))
                                }
                                Err(error) => {
                                    eprintln!("image {uri}: {error}");
                                    // Keep failed/unsupported resources failed; never read them as local paths.
                                    ImageSource::from(uri.clone())
                                }
                            })
                            .on_link_click(move |url, event, window, cx| {
                                if event.is_right_click() {
                                    return;
                                }
                                let _ = weak.update(cx, |this, cx| this.open_link(url, window, cx));
                            }),
                    )
                    .when(ready, |body| {
                        body.child(
                            canvas(
                                |_, _, _| (),
                                move |_, _, _, _| {
                                    if !first_paint.swap(true, Ordering::Relaxed) {
                                        eprintln!(
                                            "first_content_paint_from_main_ms={:.3}",
                                            started.elapsed().as_secs_f64() * 1000.
                                        );
                                    }
                                },
                            )
                            .absolute()
                            .size_0(),
                        )
                    }),
            )
            .child(
                div()
                    .debug_selector(|| "status-line".into())
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .h(px(FOOTER_LINE_HEIGHT + 1.))
                    .overflow_hidden()
                    .font_family("Consolas")
                    .text_size(px(16.))
                    .line_height(px(FOOTER_LINE_HEIGHT))
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .debug_selector(|| "mode-label".into())
                            .w(px(MODE_WIDTH))
                            .h(px(FOOTER_LINE_HEIGHT))
                            .flex_shrink_0()
                            .text_center()
                            .bg(mode_background)
                            .text_color(mode_foreground)
                            .child(self.app.mode.label()),
                    )
                    .child(
                        div()
                            .debug_selector(|| "filename".into())
                            .flex_1()
                            .min_w_0()
                            .px_2()
                            .truncate()
                            .child(filename),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "command-line".into())
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .h(px(FOOTER_LINE_HEIGHT + 1.))
                    .overflow_hidden()
                    .font_family("Consolas")
                    .text_size(px(16.))
                    .line_height(px(FOOTER_LINE_HEIGHT))
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .when(normal, |line| {
                        line.child(div().flex_1().min_w_0().truncate().child(status))
                    })
                    .when(!normal, |line| {
                        line.child(if matches!(self.app.mode, Mode::Search { .. }) {
                            "/"
                        } else {
                            ":"
                        })
                        .child(
                            Input::new(&self.find)
                                .id("prompt")
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false)
                                .h(px(FOOTER_LINE_HEIGHT))
                                .p_0()
                                .flex_1()
                                .min_w_0()
                                .font_family("Consolas")
                                .text_size(px(16.))
                                .line_height(px(FOOTER_LINE_HEIGHT)),
                        )
                    }),
            )
    }
}
