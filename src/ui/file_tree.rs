use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{prelude::*, *};

use super::{Viewer, input, pane::TREE_WIDTH};
use crate::app::file_tree::{Action, Completion, Request};

pub(super) struct TreePane {
    pub visible: bool,
    pub focus: FocusHandle,
    pub scroll: ScrollHandle,
}

impl TreePane {
    pub fn new(visible: bool, cx: &mut Context<Viewer>) -> Self {
        Self {
            visible,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }
}

impl EventEmitter<Request> for Viewer {}

impl Viewer {
    pub(super) fn emit_tree_requests(&self, requests: Vec<Request>, cx: &mut Context<Self>) {
        for request in requests {
            cx.emit(request);
        }
    }

    pub fn complete_tree(&mut self, completion: Completion, cx: &mut Context<Self>) {
        let old_root = self.app.tree.root.clone();
        let old_cursor = self.app.tree.cursor.clone();
        let requests = self.app.tree.complete(completion);
        if self.app.tree.root != old_root {
            self.tree.scroll.set_offset(point(px(0.), px(0.)));
        } else if self.app.tree.cursor != old_cursor {
            self.reveal_tree_cursor();
        }
        self.emit_tree_requests(requests, cx);
        cx.notify();
    }

    pub(super) fn tree_action(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.tree.visible
            || !self.tree.focus.is_focused(window)
            || self.app.tree.resolving_root()
            || !self.pane_keys_enabled(window, cx)
        {
            return;
        }
        let requests = self.app.tree.act(action);
        if !matches!(action, Action::Refresh) {
            self.reveal_tree_cursor();
        }
        self.emit_tree_requests(requests, cx);
        cx.notify();
    }

    pub(super) fn tree_open(&mut self, toggle: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tree.visible
            || !self.tree.focus.is_focused(window)
            || !self.pane_keys_enabled(window, cx)
            || self.app.tree.resolving_root()
        {
            return;
        }
        if let Some((path, root)) = self.app.tree.open_candidate() {
            self.message = None;
            self.display_error = None;
            self.request_operation(
                crate::app::DocumentOperation::TreeOpen { path, root },
                crate::app::OperationOrigin::Keyboard,
                None,
                cx,
            );
        } else if self
            .app
            .tree
            .cursor
            .as_ref()
            .and_then(|path| self.app.tree.node(path))
            .is_some_and(|node| {
                node.entry.directory && node.entry.expandable && !node.entry.reparse
            })
        {
            self.tree_action(
                if toggle {
                    Action::Toggle
                } else {
                    Action::Right
                },
                window,
                cx,
            );
        }
    }

    fn reveal_tree_cursor(&self) {
        if let Some(index) = self
            .app
            .tree
            .rows()
            .iter()
            .position(|(path, _)| Some(path) == self.app.tree.cursor.as_ref())
        {
            self.tree.scroll.scroll_to_item(index);
        }
    }

    pub(super) fn render_tree(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.tree.focus.is_focused(window);
        // Standard theme colors, not another Lua highlight API. Inactive cursor
        // background is subtle, but text always keeps the normal foreground.
        let background = if active {
            cx.theme().selection
        } else {
            cx.theme().muted
        };
        let accent = cx.theme().link;
        let normal = self.palette.get(crate::config::Group::Normal);
        let rows = self
            .app
            .tree
            .rows()
            .into_iter()
            .map(|(path, depth)| {
                let node = self.app.tree.node(&path).expect("visible node");
                let selected = self.app.tree.cursor.as_ref() == Some(&path);
                let symbol = if node.entry.expandable {
                    if node.expanded { "▾" } else { "▸" }
                } else {
                    " "
                };
                let name = node.entry.name.to_string_lossy().into_owned();
                let row = div()
                    .debug_selector(move || format!("tree-row-{depth}-{name}"))
                    .flex()
                    .items_center()
                    .w_full()
                    .h(px(24.))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .text_color(normal.fg)
                    .when(selected, |row| row.bg(background))
                    .child(
                        div()
                            .w(px(2.))
                            .h_full()
                            .flex_shrink_0()
                            .when(selected && active, |line| line.bg(accent)),
                    )
                    .child(
                        div()
                            .pl(px(6. + depth as f32 * 16.))
                            .flex()
                            .min_w_0()
                            .child(format!("{symbol} {}", node.entry.name.to_string_lossy())),
                    );
                // Error text belongs to the tree, never to the body/footer input.
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .child(row)
                    .when(node.error, |row| {
                        row.child(div().h(px(24.)).px_2().child("Unable to access directory."))
                    })
            })
            .collect::<Vec<_>>();
        input::bind_tree_actions(div(), cx)
            .id("file-tree")
            .debug_selector(|| "file-tree".into())
            .key_context("EvaluationTree")
            .track_focus(&self.tree.focus)
            .track_scroll(&self.tree.scroll)
            .when(!self.app.tree.resolving_root(), |tree| {
                tree.overflow_y_scroll()
            })
            .when(self.app.tree.resolving_root(), |tree| {
                tree.overflow_y_hidden()
            })
            .overflow_x_hidden()
            .w(px(TREE_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .text_size(px(13.))
            // Each row is a direct scroll child for minimal scroll_to_item.
            .children(rows)
            .into_any_element()
    }
}
