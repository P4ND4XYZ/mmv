use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use gpui_kit::component::{
    ActiveTheme as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    text::{RangeHighlight, TextView, TextViewState},
};
use gpui_kit::{prelude::*, *};

use crate::app::{self, Generation, ImageLocation, ReadRequest, ReadResult, resolve_image};

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

actions!(evaluation, [Find, NextMatch, PreviousMatch, Reload]);

pub fn bind_keys(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        KeyBinding::new("/", Find, Some("EvaluationBody")),
        KeyBinding::new("n", NextMatch, Some("EvaluationBody")),
        KeyBinding::new("shift-n", PreviousMatch, Some("EvaluationBody")),
        KeyBinding::new("f5", Reload, Some("Evaluation")),
    ]);
}

pub struct Viewer {
    app: app::App,
    path: PathBuf,
    text: Entity<TextViewState>,
    text_subscription: Option<Subscription>,
    find: Entity<InputState>,
    _input_subscription: Subscription,
    body_focus: FocusHandle,
    last_link: Option<String>,
    display_error: Option<String>,
    started: Instant,
    first_paint: Arc<AtomicBool>,
}

impl EventEmitter<ReadRequest> for Viewer {}

impl Viewer {
    pub fn new(
        path: PathBuf,
        started: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = cx.new(|cx| TextViewState::markdown("", cx));
        let find = cx
            .new(|cx| InputState::new(window, cx).placeholder("本文検索（大文字・小文字を区別）"));
        let input_subscription =
            cx.subscribe_in(&find, window, |this, input, event, window, cx| {
                match event {
                    InputEvent::Change => {
                        this.app.set_query(input.read(cx).value().to_string());
                        this.apply_search(false, cx);
                    }
                    InputEvent::PressEnter { .. } => {
                        this.body_focus.focus(window, cx);
                        this.apply_search(true, cx);
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
            find,
            _input_subscription: input_subscription,
            body_focus,
            last_link: None,
            display_error: None,
            started,
            first_paint: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn open(&mut self, cx: &mut Context<Self>) {
        let request = self.app.open(self.path.clone());
        cx.emit(request);
        cx.notify();
    }

    pub fn complete(&mut self, result: ReadResult, cx: &mut Context<Self>) {
        if self.app.complete(result) {
            let document = self.app.document.as_ref().expect("accepted document");
            let generation = document.generation;
            self.text = cx.new(|cx| TextViewState::markdown(&document.source, cx));
            self.text_subscription = Some(cx.observe(&self.text, move |this, _, cx| {
                this.accept_rendered_text(generation, cx);
            }));
            self.display_error = None;
            // Small documents can already be parsed when the entity is created.
            self.accept_rendered_text(generation, cx);
        }
        cx.notify();
    }

    fn accept_rendered_text(&mut self, generation: Generation, cx: &mut Context<Self>) {
        let snapshot = self.text.read(cx).rendered_text();
        let Some(document) = self.app.document.as_ref() else {
            return;
        };
        if document.generation != generation || snapshot.source() != document.source {
            return;
        }
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

    fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
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
        let first_paint = self.first_paint.clone();
        let started = self.started;
        let weak = cx.entity().downgrade();
        let matches = match self.app.current_match {
            Some(ix) => format!("{} / {}", ix + 1, self.app.matches.len()),
            None => "0 / 0".into(),
        };
        let status = self
            .app
            .error
            .as_ref()
            .or(self.display_error.as_ref())
            .cloned()
            .or_else(|| {
                self.last_link
                    .as_ref()
                    .map(|url| format!("リンク選択: {url}"))
            })
            .unwrap_or_else(|| {
                if self.app.is_loading() {
                    "読み込み中…".into()
                } else {
                    "V-01 / V-02 検証用。F5: 再読み込み（位置復元は未実装）".into()
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
            .on_action(cx.listener(|this, _: &Reload, _, cx| this.open(cx)))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.body_focus.focus(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(format!("GPUI evaluation — {}", path.display())),
            )
            .child(
                div()
                    .key_context("EvaluationBody")
                    .track_focus(&self.body_focus)
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .on_action(cx.listener(|this, _: &Find, window, cx| {
                        this.find.update(cx, |input, cx| input.focus(window, cx));
                    }))
                    .on_action(cx.listener(|this, _: &NextMatch, _, cx| this.step_match(true, cx)))
                    .on_action(
                        cx.listener(|this, _: &PreviousMatch, _, cx| this.step_match(false, cx)),
                    )
                    .child(
                        TextView::new(&self.text)
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
                            .on_link_click(move |url, event, _, cx| {
                                if event.is_right_click() {
                                    return;
                                }
                                let _ = weak.update(cx, |this, cx| {
                                    this.last_link = Some(url.to_string());
                                    cx.notify();
                                });
                                // Local link routing and history belong to later evaluation stages.
                                if url::Url::parse(url)
                                    .is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
                                {
                                    cx.open_url(url);
                                }
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
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(Input::new(&self.find).id("find").w(px(320.)))
                    .child(matches)
                    .child(
                        Button::new("previous")
                            .ghost()
                            .label("前へ (N)")
                            .on_click(cx.listener(|this, _, _, cx| this.step_match(false, cx))),
                    )
                    .child(
                        Button::new("next")
                            .ghost()
                            .label("次へ (n)")
                            .on_click(cx.listener(|this, _, _, cx| this.step_match(true, cx))),
                    ),
            )
            .child(div().px_4().pb_2().text_sm().child(status))
    }
}
