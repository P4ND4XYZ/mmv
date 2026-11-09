use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{prelude::*, *};

use super::Viewer;
use crate::app::{DocumentOperation, Mode, OperationOrigin, SearchOutcome};

actions!(
    evaluation,
    [
        Find,
        Links,
        Command,
        NextMatch,
        PreviousMatch,
        Reload,
        Back,
        Forward,
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

pub fn bind_keys(cx: &mut gpui_kit::App) {
    cx.bind_keys([
        KeyBinding::new("/", Find, Some("EvaluationNormal")),
        KeyBinding::new("f", Links, Some("EvaluationNormal")),
        KeyBinding::new(":", Command, Some("EvaluationNormal")),
        KeyBinding::new("n", NextMatch, Some("EvaluationNormal")),
        KeyBinding::new("shift-n", PreviousMatch, Some("EvaluationNormal")),
        KeyBinding::new("f5", Reload, Some("EvaluationNormal")),
        KeyBinding::new("ctrl-o", Back, Some("EvaluationNormal")),
        KeyBinding::new("ctrl-i", Forward, Some("EvaluationNormal")),
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

pub(super) struct Prompt {
    pub state: Entity<InputState>,
    _subscription: Subscription,
}

impl Prompt {
    pub fn new(window: &mut Window, cx: &mut Context<Viewer>) -> Self {
        let find = cx.new(|cx| InputState::new(window, cx));
        let input_subscription =
            cx.subscribe_in(&find, window, |this, input, event, window, cx| {
                match event {
                    InputEvent::Change => {
                        if matches!(this.app.mode, Mode::Search) {
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

        Self {
            state: find,
            _subscription: input_subscription,
        }
    }
}

pub(super) fn bind_root_actions(root: Div, cx: &mut Context<Viewer>) -> Div {
    root.on_action(cx.listener(|this, _: &Reload, _, cx| {
        if this.normal_keys_enabled() {
            this.reload(cx);
        }
    }))
    .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
        if event.keystroke.key == "backspace"
            && event.keystroke.modifiers == Modifiers::default()
            && this.app.mode == Mode::Command
            && this.input.state.read(cx).value().is_empty()
        {
            let composing = this.input.state.update(cx, |input, cx| {
                input.marked_text_range(window, cx).is_some()
            });
            if !composing {
                this.cancel_input(window, cx);
                cx.stop_propagation();
            }
        }
    }))
    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
        if event.keystroke.key == "escape" && this.link_panel.is_none() {
            this.cancel_input(window, cx);
            cx.stop_propagation();
        }
    }))
}

pub(super) fn bind_body_actions(body: Div, cx: &mut Context<Viewer>) -> Div {
    body.on_action(cx.listener(|this, _: &Links, window, cx| {
        this.begin_link_selection(window, cx);
    }))
    .on_action(cx.listener(|this, _: &Find, window, cx| {
        this.begin_input(false, window, cx);
    }))
    .on_action(cx.listener(|this, _: &Command, window, cx| this.begin_input(true, window, cx)))
    .on_action(cx.listener(|this, _: &ScrollDown, _, cx| this.scroll_input(1., 0., cx)))
    .on_action(cx.listener(|this, _: &ScrollUp, _, cx| this.scroll_input(-1., 0., cx)))
    .on_action(cx.listener(|this, _: &HalfPageDown, _, cx| this.scroll_input(0., 0.5, cx)))
    .on_action(cx.listener(|this, _: &HalfPageUp, _, cx| this.scroll_input(0., -0.5, cx)))
    .on_action(cx.listener(|this, _: &PageDown, _, cx| this.scroll_input(0., 1., cx)))
    .on_action(cx.listener(|this, _: &PageUp, _, cx| this.scroll_input(0., -1., cx)))
    .on_action(cx.listener(|this, _: &FirstLine, _, cx| this.scroll_edge_input(false, cx)))
    .on_action(cx.listener(|this, _: &LastLine, _, cx| this.scroll_edge_input(true, cx)))
    .on_action(cx.listener(|this, _: &Back, _, cx| {
        this.history_input(DocumentOperation::Back, cx);
    }))
    .on_action(cx.listener(|this, _: &Forward, _, cx| {
        this.history_input(DocumentOperation::Forward, cx);
    }))
    .on_action(cx.listener(|this, _: &NextMatch, _, cx| this.step_match(true, cx)))
    .on_action(cx.listener(|this, _: &PreviousMatch, _, cx| this.step_match(false, cx)))
}

impl Viewer {
    pub(super) fn normal_keys_enabled(&self) -> bool {
        self.app.mode == Mode::Normal && self.link_panel.is_none()
    }

    pub(super) fn begin_input(
        &mut self,
        command: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.normal_keys_enabled() {
            return;
        }
        self.message = None;
        self.display_error = None;
        self.app.clear_error();
        if command {
            self.app.begin_command();
        } else {
            self.app.begin_search();
            self.apply_search(false, cx);
        }
        self.input.state.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn cancel_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.app.cancel_input();
        self.apply_search(false, cx);
        self.body_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn confirm_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.input.state.read(cx).value().to_string();
        match self.app.mode {
            Mode::Search => {
                let from = match self.search_origin(cx) {
                    Ok(from) => from,
                    Err(error) => {
                        self.cancel_input(window, cx);
                        self.message = Some(error);
                        self.message_is_error = true;
                        return;
                    }
                };
                let outcome = self.app.confirm_search(input, from);
                self.apply_search(outcome == SearchOutcome::Found, cx);
                self.message_is_error = false;
                self.message = match outcome {
                    SearchOutcome::Found => None,
                    SearchOutcome::NotFound => Some("一致する文字列がありません".into()),
                    SearchOutcome::NoPreviousQuery => Some("直前の検索語がありません".into()),
                };
            }
            Mode::Command => {
                self.app.cancel_input();
                match input.as_str() {
                    "q" => {
                        window.remove_window();
                        return;
                    }
                    "noh" | "nohlsearch" => {
                        self.app.clear_search_highlights();
                        self.apply_search(false, cx);
                        self.message = None;
                    }
                    _ => {
                        self.message_is_error = false;
                        self.message = Some(format!("未対応のコマンド: :{input}"));
                    }
                }
            }
            Mode::Normal => return,
        }
        self.body_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        if !self.normal_keys_enabled() {
            return;
        }
        self.app.step_match(forward);
        self.apply_search(true, cx);
        cx.notify();
    }

    fn history_input(&mut self, operation: DocumentOperation, cx: &mut Context<Self>) {
        if self.normal_keys_enabled() {
            self.request_operation(operation, OperationOrigin::Keyboard, None, cx);
        }
    }

    fn scroll_input(&mut self, lines: f32, pages: f32, cx: &mut Context<Self>) {
        if self.normal_keys_enabled() {
            self.document.scroll(lines, pages, cx);
        }
    }

    fn scroll_edge_input(&mut self, end: bool, cx: &mut Context<Self>) {
        if self.normal_keys_enabled() {
            self.document.scroll_edge(end, cx);
        }
    }
}
