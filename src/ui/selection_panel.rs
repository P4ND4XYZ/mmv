//! Reusable selection UI. Callers supply already-ranked rows and own execution.
use std::{cell::Cell, ops::Range, rc::Rc};

use gpui_kit::component::{
    ElementExt as _, IndexPath,
    command::{Command, CommandItem, CommandState},
    dialog::{Cancel, Confirm},
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{
    base::actions::{SelectDown, SelectUp},
    prelude::*,
    *,
};

use super::theme::{Colors, Palette};
use crate::config::Group;

#[derive(Clone)]
pub(super) struct Item {
    pub id: usize,
    pub primary: String,
    pub secondary: String,
    pub badge: &'static str,
    pub primary_ranges: Vec<Range<usize>>,
    pub secondary_ranges: Vec<Range<usize>>,
}

pub(super) enum Event {
    Query(String),
    Confirm(usize),
    SelectionChanged(Option<usize>),
    PreviewScrollRequested(bool),
    Cancel,
}

pub(super) struct SelectionPanel {
    pub input: Entity<InputState>,
    pub command: Entity<CommandState>,
    pub items: Vec<Item>,
    query: String,
    items_query: String,
    selected: Rc<Cell<Option<usize>>>,
    enter_was_composing: bool,
    reset_selection: bool,
    empty: SharedString,
    palette: Palette,
    preview: Option<AnyView>,
    list_height: Pixels,
    _subscription: Subscription,
    _keystroke_subscription: Subscription,
}

impl EventEmitter<Event> for SelectionPanel {}

impl SelectionPanel {
    pub fn new(
        items: Vec<Item>,
        empty: impl Into<SharedString>,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let command = cx.new(|cx| CommandState::new(window, cx));
        let subscription = cx.subscribe_in(&input, window, |this, input, event, window, cx| {
            match event {
                InputEvent::Change => {
                    let query = input.read(cx).value().to_string();
                    if query != this.query {
                        this.query = query.clone();
                        cx.emit(Event::Query(query));
                    }
                }
                InputEvent::PressEnter { .. } => {
                    // Capture precedes the input's action: the input/platform may
                    // commit marked text before this deferred event is delivered.
                    let composing = input.update(cx, |input, cx| {
                        input.marked_text_range(window, cx).is_some()
                    });
                    if !this.enter_was_composing && !composing {
                        this.confirm(cx);
                    }
                    this.enter_was_composing = false;
                }
                _ => {}
            }
            cx.notify();
        });
        // Raw key listeners run AFTER key bindings in GPUI. Snapshot before
        // either path so a platform-consumed composition Enter cannot leave a
        // stale guard on the next independent Enter. Scope to this input only.
        let weak = cx.entity().downgrade();
        let query = input.clone();
        let keystroke_subscription = cx.intercept_keystrokes(move |event, window, cx| {
            if event.keystroke.key == "enter" && query.focus_handle(cx).is_focused(window) {
                let composing = query.update(cx, |input, cx| {
                    input.marked_text_range(window, cx).is_some()
                });
                let _ = weak.update(cx, |this, _| this.enter_was_composing = composing);
            }
        });
        // Only opening moves focus. Rendering and query updates never do.
        input.focus_handle(cx).focus(window, cx);
        Self {
            selected: Rc::new(Cell::new((!items.is_empty()).then_some(0))),
            input,
            command,
            items,
            query: String::new(),
            items_query: String::new(),
            enter_was_composing: false,
            reset_selection: true,
            empty: empty.into(),
            palette,
            preview: None,
            list_height: px(0.),
            _subscription: subscription,
            _keystroke_subscription: keystroke_subscription,
        }
    }

    pub fn set_items(&mut self, items: Vec<Item>, query: &str, cx: &mut Context<Self>) {
        let previous = self.selected_id();
        self.items_query = query.to_owned();
        self.selected.set((!items.is_empty()).then_some(0));
        self.items = items;
        self.reset_selection = true;
        self.emit_selection_change(previous, cx);
        cx.notify();
    }

    pub fn set_preview(&mut self, preview: AnyView) {
        self.preview = Some(preview);
    }

    /// Called by the feature only after its subscription and lifetime are installed.
    pub fn activate(&self, cx: &mut Context<Self>) {
        cx.emit(Event::SelectionChanged(self.selected_id()));
    }

    fn selected_id(&self) -> Option<usize> {
        self.selected
            .get()
            .and_then(|ix| self.items.get(ix))
            .map(|item| item.id)
    }

    fn emit_selection_change(&self, previous: Option<usize>, cx: &mut Context<Self>) {
        let selected = self.selected_id();
        if previous != selected {
            cx.emit(Event::SelectionChanged(selected));
        }
    }

    fn select_index(&mut self, index: usize, cx: &mut Context<Self>) {
        let previous = self.selected_id();
        self.selected
            .set((index < self.items.len()).then_some(index));
        self.emit_selection_change(previous, cx);
        cx.notify();
    }

    fn cancel_or_unmark(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let composing = self.input.update(cx, |input, cx| {
            let composing = input.marked_text_range(window, cx).is_some();
            if composing {
                input.unmark_text(window, cx);
            }
            composing
        });
        if !composing {
            cx.emit(Event::Cancel);
        }
        cx.stop_propagation();
    }

    fn confirm(&self, cx: &mut Context<Self>) {
        // A query callback must have supplied its new rows before confirmation.
        if self.input.read(cx).value().as_ref() != self.items_query {
            return;
        }
        if let Some(item) = self.selected.get().and_then(|ix| self.items.get(ix)) {
            cx.emit(Event::Confirm(item.id));
        }
    }

    fn move_selection(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.items.len();
        if count == 0 {
            return;
        }
        let current = self.selected.get().unwrap_or(0);
        let next = if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        self.select_index(next, cx);
        self.command.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::new(next)), window, cx);
        });
        cx.notify();
    }

    fn capture_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let composing = self.input.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        });
        let key = event.keystroke.key.as_str();
        if key == "enter" {
            self.enter_was_composing = composing;
        }
        if key == "escape" && !composing {
            cx.emit(Event::Cancel);
            cx.stop_propagation();
        } else if !composing {
            let modifiers = event.keystroke.modifiers;
            let plain = modifiers == Modifiers::default();
            let control = modifiers
                == Modifiers {
                    control: true,
                    ..Default::default()
                };
            if control && matches!(key, "u" | "d") {
                cx.emit(Event::PreviewScrollRequested(key == "d"));
                cx.stop_propagation();
            } else if (plain && matches!(key, "down" | "up"))
                || (control && matches!(key, "n" | "p"))
            {
                self.move_selection(matches!(key, "down" | "n"), window, cx);
                cx.stop_propagation();
            }
        }
    }
}

fn match_style() -> HighlightStyle {
    // Emphasis must inherit both selected and unselected palette colors.
    HighlightStyle {
        font_weight: Some(FontWeight::BOLD),
        ..Default::default()
    }
}

fn highlighted(text: String, ranges: Vec<Range<usize>>) -> StyledText {
    StyledText::new(text).with_highlights(ranges.into_iter().map(|range| (range, match_style())))
}

fn item_element(item: &Item, selected: bool, palette: &Palette) -> Div {
    let colors = palette.get(if selected {
        Group::PanelSelected
    } else {
        Group::Panel
    });
    let id = item.id;
    div()
        .debug_selector(move || format!("selection-item-{id}"))
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        // Cover Command's native row insets with the resolved colors, in rems
        // just like its own padding, including at a different UI scale.
        .mx(rems(-0.5))
        .my(rems(-0.375))
        .px_2()
        .py_1p5()
        .bg(colors.bg)
        .text_color(colors.fg)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().flex_shrink_0().child(item.badge))
                .child(div().min_w_0().truncate().child(highlighted(
                    item.primary.clone(),
                    item.primary_ranges.clone(),
                ))),
        )
        .child(div().truncate().child(highlighted(
            item.secondary.clone(),
            item.secondary_ranges.clone(),
        )))
}

pub(super) fn input_element(input: &Entity<InputState>, colors: Colors) -> Input {
    Input::new(input)
        .id("selection-query")
        .appearance(false)
        .bordered(false)
        .focus_bordered(false)
        .bg(colors.bg)
        .text_color(colors.fg)
}

impl Render for SelectionPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.palette.get(Group::Panel);
        let items = self
            .items
            .iter()
            .enumerate()
            .map(|(ix, item)| {
                let item = item.clone();
                let selected = self.selected.clone();
                let palette = self.palette.clone();
                CommandItem::new()
                    .label(item.primary.clone())
                    .child(move |_, _| item_element(&item, selected.get() == Some(ix), &palette))
            })
            .collect::<Vec<_>>();
        let empty = self.empty.clone();
        let weak = cx.entity().downgrade();
        let confirmed_items = self.items.iter().map(|item| item.id).collect::<Vec<_>>();
        let command = Command::new(&self.command)
            .searchable(false)
            .filterable(false)
            .bordered(false)
            .max_h(self.list_height)
            .bg(colors.bg)
            .text_color(colors.fg)
            .items(items)
            .empty(move |_, _, _| {
                div()
                    .id("selection-empty")
                    .test_support()
                    .aria_label(empty.clone())
                    .p_3()
                    .bg(colors.bg)
                    .text_color(colors.fg)
                    .child(empty.clone())
            })
            .on_select(move |index, _, cx| {
                let _ = weak.update(cx, |panel, cx| panel.select_index(index.row, cx));
            })
            .on_confirm({
                let weak = cx.entity().downgrade();
                move |index, _, cx| {
                    if let Some(&id) = confirmed_items.get(index.row) {
                        let _ = weak.update(cx, |panel, cx| {
                            // Deferred mouse confirmations must still refer to this query/list.
                            if panel.items.get(index.row).is_some_and(|item| item.id == id) {
                                panel.select_index(index.row, cx);
                                panel.confirm(cx);
                            }
                        });
                    }
                }
            });
        let reset = std::mem::take(&mut self.reset_selection);
        let state = self.command.clone();
        div()
            .debug_selector(|| "selection-panel".into())
            .key_context("SelectionPanel")
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .min_w_0()
            .bg(colors.bg)
            .text_color(colors.fg)
            .capture_key_down(cx.listener(Self::capture_key))
            // Command's default Esc clears the query; its default Enter has
            // no IME guard. Input events above own these product semantics.
            .capture_action(
                cx.listener(|this, _: &gpui_kit::base::input::Enter, window, cx| {
                    this.enter_was_composing |= this.input.update(cx, |input, cx| {
                        input.marked_text_range(window, cx).is_some()
                    });
                }),
            )
            .capture_action(|_: &Confirm, _, cx| cx.stop_propagation())
            .capture_action(cx.listener(|this, _: &SelectDown, window, cx| {
                if this.input.update(cx, |input, cx| {
                    input.marked_text_range(window, cx).is_some()
                }) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &SelectUp, window, cx| {
                if this.input.update(cx, |input, cx| {
                    input.marked_text_range(window, cx).is_some()
                }) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(
                cx.listener(|this, _: &gpui_kit::base::input::Escape, window, cx| {
                    this.cancel_or_unmark(window, cx)
                }),
            )
            .capture_action(
                cx.listener(|this, _: &Cancel, window, cx| this.cancel_or_unmark(window, cx)),
            )
            .on_prepaint(move |_, window, cx| {
                if reset {
                    state.update(cx, |state, cx| {
                        let first = (state.matched_count() > 0).then_some(IndexPath::new(0));
                        state.set_selected_index(first, window, cx);
                    });
                }
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .debug_selector(|| "selection-list".into())
                            .on_prepaint({
                                let panel = cx.entity().downgrade();
                                move |bounds, _, cx| {
                                    let _ = panel.update(cx, |panel, cx| {
                                        if panel.list_height != bounds.size.height {
                                            panel.list_height = bounds.size.height;
                                            cx.notify();
                                        }
                                    });
                                }
                            })
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .overflow_hidden()
                            .child(command),
                    )
                    .child(
                        div()
                            .debug_selector(|| "selection-preview".into())
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .h_full()
                            .border_l_1()
                            .border_color(self.palette.get(Group::Border).fg)
                            .children(self.preview.clone()),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "selection-input".into())
                    .w_full()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(self.palette.get(Group::Border).fg)
                    .child(input_element(&self.input, colors)),
            )
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
