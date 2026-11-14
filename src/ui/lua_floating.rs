//! GPUI adapter for the resident Lua UI model. No GPUI references enter Lua.
use super::{Viewer, theme::Palette};
use crate::{
    config::Group,
    lua::{
        callbacks::{CloseReason, OwnerId},
        ui::{
            FocusTarget,
            tree::{Align, Kind, Node},
        },
    },
};
use gpui_kit::{prelude::*, *};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct FloatingUi {
    handles: BTreeMap<(OwnerId, Option<u64>), FocusHandle>,
    previous_external: Option<FocusHandle>,
    focus_revision: u64,
}

impl Viewer {
    fn sync_lua_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let context = self.lua.context();
        let mut ui = context.ui.borrow_mut();
        for w in &ui.windows {
            self.floating_ui
                .handles
                .entry((w.owner, None))
                .or_insert_with(|| cx.focus_handle());
            for id in &w.focus_order {
                self.floating_ui
                    .handles
                    .entry((w.owner, Some(*id)))
                    .or_insert_with(|| cx.focus_handle());
            }
        }
        let actual = self
            .floating_ui
            .handles
            .iter()
            .find(|(_, h)| h.is_focused(window))
            .map(|((owner, identity), _)| FocusTarget {
                owner: *owner,
                identity: *identity,
            });
        let modal = ui.windows.iter().any(|w| w.modal);
        let actual_accessible = actual.filter(|f| ui.accessible(f.owner) && ui.valid_focus(*f));
        if self.floating_ui.focus_revision != ui.focus_revision {
            // The model can close a window before its host has observed a
            // background focus change. Restore only if a removed window really
            // held focus, not merely because the model's last target said so.
            let closed_was_focused =
                actual.is_some_and(|f| !ui.windows.iter().any(|w| w.owner == f.owner));
            if ui.restoring_focus && !closed_was_focused && !modal {
                ui.set_focus(actual_accessible);
            } else if let Some(focus) = ui.focus {
                if actual.is_none() {
                    self.floating_ui.previous_external = window.focused(cx);
                }
                if let Some(handle) = self.floating_ui.handles.get(&(focus.owner, focus.identity)) {
                    handle.focus(window, cx);
                }
            } else if closed_was_focused
                && let Some(handle) = self.floating_ui.previous_external.take()
            {
                handle.focus(window, cx);
            }
            ui.restoring_focus = false;
        } else if actual_accessible != ui.focus {
            if modal && actual_accessible.is_none() {
                // Logical GPUI focus only; this never activates the OS window.
                if let Some(focus) = ui.focus
                    && let Some(handle) =
                        self.floating_ui.handles.get(&(focus.owner, focus.identity))
                {
                    handle.focus(window, cx);
                }
            } else {
                ui.set_focus(actual_accessible);
                if actual_accessible.is_none() {
                    self.floating_ui.previous_external = window.focused(cx);
                }
            }
        }
        self.floating_ui.focus_revision = ui.focus_revision;
        self.floating_ui.handles.retain(|(owner, identity), _| {
            ui.windows
                .iter()
                .any(|w| w.owner == *owner && identity.is_none_or(|id| w.focus_order.contains(&id)))
        });
    }

    pub(super) fn lua_float_key(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_lua_focus(window, cx);
        let context = self.lua.context();
        // This interceptor runs before GPUI keybindings, including native Tab.
        // D adds its input-owned composition check before calling ui_key. Behind
        // a nonmodal, leave the existing prompt/IME path completely untouched.
        if context.ui.borrow().focus.is_none() {
            return;
        }
        let stroke = &event.keystroke;
        let mut key = String::new();
        for (active, modifier) in [
            (stroke.modifiers.control, "ctrl-"),
            (stroke.modifiers.shift, "shift-"),
            (stroke.modifiers.alt, "alt-"),
            (stroke.modifiers.platform, "super-"),
        ] {
            if active {
                key.push_str(modifier);
            }
        }
        key.push_str(&stroke.key);
        if crate::lua::keymap::normalize(&key).is_err() {
            return;
        }
        match context.ui_key(self.lua.lua(), &key) {
            Ok(true) => {
                cx.stop_propagation();
                self.pump_lua(cx);
                self.sync_lua_focus(window, cx);
                cx.notify();
            }
            Ok(false) => (),
            Err(error) => {
                context.report_error(
                    None,
                    crate::lua::ErrorInfo::new(error, crate::lua::Phase::Event, None),
                );
                self.pump_lua(cx);
                cx.stop_propagation();
                cx.notify();
            }
        }
    }

    pub(super) fn render_lua_floats(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        self.sync_lua_focus(window, cx);
        let context = self.lua.context();
        let ui = context.ui.borrow();
        let viewport = window.viewport_size();
        let mut result = Vec::new();
        for (index, w) in ui.windows.iter().enumerate() {
            let owner = w.owner;
            let width = w
                .width
                .resolve(f32::from(viewport.width))
                .unwrap()
                .min(f32::from(viewport.width));
            let height = w
                .height
                .resolve(f32::from(viewport.height))
                .unwrap()
                .min(f32::from(viewport.height));
            let handle = self.floating_ui.handles[&(owner, None)].clone();
            let content = render_node(
                &w.root,
                owner,
                &w.components,
                &self.floating_ui.handles,
                &self.palette,
                Group::Panel,
                cx,
            );
            let mut layer = div().absolute().inset_0().size_full();
            if w.modal || (index + 1 == ui.windows.len() && w.outside_click) {
                let outside = w.outside_click;
                layer = layer.child(
                    div()
                        .id(("lua-backdrop", index))
                        .debug_selector(move || format!("lua-backdrop-{index}"))
                        .absolute()
                        .inset_0()
                        .size_full()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if outside {
                                let _ = this
                                    .lua
                                    .context()
                                    .close_owner(owner, CloseReason::OutsideClick);
                                this.pump_lua(cx);
                                this.sync_lua_focus(window, cx);
                                cx.notify();
                            }
                            cx.stop_propagation();
                        })),
                );
            }
            let colors = self.palette.get(Group::Panel);
            layer = layer.child(
                div()
                    .id(("lua-window", index))
                    .debug_selector(move || format!("lua-window-{index}"))
                    .absolute()
                    .left((viewport.width - px(width)) / 2.)
                    .top((viewport.height - px(height)) / 2.)
                    .w(px(width))
                    .h(px(height))
                    .overflow_hidden()
                    .occlude()
                    .track_focus(&handle)
                    .key_context("LuaFloat")
                    .bg(colors.bg)
                    .text_color(colors.fg)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            let context = this.lua.context();
                            let mut ui = context.ui.borrow_mut();
                            if ui.accessible(owner) {
                                ui.set_focus(Some(FocusTarget {
                                    owner,
                                    identity: None,
                                }));
                            }
                            drop(ui);
                            this.sync_lua_focus(window, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(content),
            );
            result.push(layer.into_any_element());
        }
        result
    }
}

fn render_node(
    node: &Node,
    owner: OwnerId,
    components: &BTreeMap<String, crate::lua::ui::Component>,
    handles: &BTreeMap<(OwnerId, Option<u64>), FocusHandle>,
    palette: &Palette,
    inherited: Group,
    cx: &mut Context<Viewer>,
) -> AnyElement {
    let group = node.hl.unwrap_or(inherited);
    let colors = palette.get(group);
    // GPUI percentages use the parent's content box, excluding padding,
    // including when the parent's size is resolved by flex grow or auto.
    let length = |dimension| match dimension {
        crate::lua::ui::tree::Dimension::Auto => Length::Auto,
        crate::lua::ui::tree::Dimension::Pixels(n) => px(n.min(f64::from(f32::MAX)) as f32).into(),
        crate::lua::ui::tree::Dimension::Ratio(n) => relative(n as f32).into(),
    };
    let selector = node
        .key
        .as_ref()
        .map(|key| format!("lua-component-{key}"))
        .unwrap_or_else(|| format!("lua-node-{:p}", node));
    let mut el = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .min_w_0()
        .min_h_0()
        .flex_shrink_1()
        .p(px(node.padding))
        .bg(colors.bg)
        .text_color(colors.fg);
    el = el.w(length(node.width)).h(length(node.height));
    if node.grow > 0. {
        el = el.flex_grow(node.grow).flex_basis(px(0.));
    }
    match node.kind {
        Kind::Row | Kind::Column => {
            el = el.flex().gap(px(node.gap));
            if node.kind == Kind::Column {
                el = el.flex_col();
            }
            el = match node.align {
                Align::Start => el.items_start(),
                Align::Center => el.items_center(),
                Align::End => el.items_end(),
                Align::Stretch => el.items_stretch(),
            };
            for child in &node.children {
                el = el.child(render_node(
                    child, owner, components, handles, palette, group, cx,
                ));
            }
        }
        Kind::Text => {
            if node.wrap {
                el = el.whitespace_normal();
            } else {
                el = el.whitespace_nowrap();
            }
            el = el.child(
                StyledText::new(node.text.clone()).with_highlights(text_highlights(node, palette)),
            );
        }
        Kind::Button => {
            let component = &components[node.key.as_ref().unwrap()];
            if node.enabled {
                let identity = component.identity;
                let handle = &handles[&(owner, Some(identity))];
                el = el
                    .track_focus(handle)
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            let context = this.lua.context();
                            if context.ui.borrow().accessible(owner) {
                                context.ui.borrow_mut().set_focus(Some(FocusTarget {
                                    owner,
                                    identity: Some(identity),
                                }));
                            }
                            this.sync_lua_focus(window, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let _ = this
                            .lua
                            .context()
                            .activate_button(this.lua.lua(), owner, identity);
                        this.pump_lua(cx);
                        this.sync_lua_focus(window, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }));
            }
            el = el.child(node.text.clone());
        }
    }
    el.into_any_element()
}

/// Produce non-overlapping runs. Later ranges replace only explicitly supplied
/// attributes; false decorations reset inherited/earlier styling.
fn text_highlights(
    node: &Node,
    palette: &Palette,
) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    let mut boundaries = vec![0, node.text.len()];
    for h in &node.highlights {
        boundaries.extend([h.range.start, h.range.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
        .windows(2)
        .filter(|w| w[0] < w[1])
        .map(|w| {
            let range = w[0]..w[1];
            let mut style = HighlightStyle::default();
            for h in &node.highlights {
                if h.range.start <= range.start && h.range.end >= range.end {
                    if let Some(bold) = h.bold {
                        style.font_weight = Some(if bold {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        });
                    }
                    if let Some(underline) = h.underline {
                        style.underline = Some(UnderlineStyle {
                            thickness: px(if underline { 1. } else { 0. }),
                            ..Default::default()
                        });
                    }
                    if let Some(group) = h.hl {
                        let colors = palette.get(group);
                        style.color = Some(colors.fg);
                        style.background_color = Some(colors.bg);
                    }
                }
            }
            (range, style)
        })
        .collect()
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
