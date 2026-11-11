use gpui_kit::{prelude::*, *};

use super::Viewer;
use crate::app::Mode;

pub(super) const TREE_WIDTH: f32 = 240.;
pub(super) const BODY_MIN_WIDTH: f32 = 240.;
pub(super) const BORDER_WIDTH: f32 = 1.;
pub(crate) const MIN_WINDOW_WIDTH: f32 = TREE_WIDTH + BODY_MIN_WIDTH + BORDER_WIDTH;

impl Viewer {
    pub(super) fn pane_keys_enabled(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.app.mode == Mode::Normal
            && self.link_panel.is_none()
            && !self.input.state.update(cx, |input, cx| {
                input.marked_text_range(window, cx).is_some()
            })
    }

    pub(super) fn toggle_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pane_keys_enabled(window, cx) {
            return;
        }
        self.tree.visible = !self.tree.visible;
        if self.tree.visible {
            let requests = self.app.tree.show(&self.config.tree.root_markers);
            self.emit_tree_requests(requests, cx);
        } else if self.tree.focus.is_focused(window) {
            self.body_focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn focus_pane(&mut self, tree: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pane_keys_enabled(window, cx) {
            return;
        }
        if tree {
            if self.tree.visible && !self.tree.focus.is_focused(window) {
                self.tree.focus.focus(window, cx);
            }
        } else if !self.body_focus.is_focused(window) {
            self.body_focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn render_panes(
        &mut self,
        body: Div,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .debug_selector(|| "pane-layout".into())
            .flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .when(self.tree.visible, |row| {
                row.child(self.render_tree(window, cx)).child(
                    div()
                        .w(px(BORDER_WIDTH))
                        .h_full()
                        .flex_shrink_0()
                        .bg(self.palette.get(crate::config::Group::Border).fg),
                )
            })
            .child(body)
    }
}
