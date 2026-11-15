use super::{super::Viewer, FloatingUi};
use crate::lua::{callbacks::OwnerId, ui::tree::Kind};
use gpui_kit::{
    component::input::{InputEvent, InputState},
    prelude::*,
    *,
};

pub(super) struct InputEntry {
    pub state: Entity<InputState>,
    pub bound_text: String,
    pub _subscription: Subscription,
}
impl Viewer {
    pub(super) fn sync_lua_resources(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let context = self.lua.context();
        let components = context
            .ui
            .borrow()
            .windows
            .iter()
            .flat_map(|w| w.components.values().cloned().map(move |c| (w.owner, c)))
            .collect::<Vec<_>>();
        self.floating_ui.inputs.retain(|(owner, id), _| {
            components
                .iter()
                .any(|(o, c)| o == owner && c.identity == *id && c.kind == Kind::Input)
        });
        self.floating_ui.sources.retain(|(owner, id), _| {
            components
                .iter()
                .any(|(o, c)| o == owner && c.identity == *id && c.kind == Kind::Source)
        });
        self.floating_ui.lists.retain(|(owner, id), _| {
            components
                .iter()
                .any(|(o, c)| o == owner && c.identity == *id && c.kind == Kind::List)
        });
        for (owner, c) in components {
            let id = c.identity;
            if let Some(input) = &c.node.input {
                if !self.floating_ui.inputs.contains_key(&(owner, id)) {
                    let value = crate::lua::ui::tree::string(
                        input.state.get(self.lua.lua()).unwrap(),
                        "input",
                    )
                    .unwrap();
                    let state =
                        cx.new(|cx| InputState::new(window, cx).default_value(value.clone()));
                    let subscription =
                        cx.subscribe_in(&state, window, move |this, input, event, window, cx| {
                            if matches!(event, InputEvent::Change) {
                                let composing = input.update(cx, |input, cx| {
                                    input.marked_text_range(window, cx).is_some()
                                });
                                if !composing {
                                    this.commit_lua_input(
                                        owner,
                                        id,
                                        input.read(cx).value().to_string(),
                                        window,
                                        cx,
                                    );
                                }
                            }
                        });
                    self.floating_ui.inputs.insert(
                        (owner, id),
                        InputEntry {
                            state,
                            bound_text: value,
                            _subscription: subscription,
                        },
                    );
                }
                let entry = self.floating_ui.inputs.get_mut(&(owner, id)).unwrap();
                let state = &entry.state;
                let value =
                    crate::lua::ui::tree::string(input.state.get(self.lua.lua()).unwrap(), "input")
                        .unwrap();
                state.update(cx, |s, cx| {
                    let composing = s.marked_text_range(window, cx).is_some();
                    // A native edit can precede its deferred Change event. Only
                    // a changed bind value may overwrite that pending edit.
                    if !composing && entry.bound_text != value {
                        if s.value().as_ref() != value {
                            s.set_value(value.clone(), window, cx);
                        }
                        entry.bound_text = value;
                    }
                    if s.presentation().placeholder().as_ref() != input.placeholder {
                        s.set_placeholder(input.placeholder.clone(), window, cx);
                    }
                });
                self.floating_ui
                    .handles
                    .insert((owner, Some(id)), state.focus_handle(cx));
            }
            if let Some(source) = &c.node.source {
                let replace = self
                    .floating_ui
                    .sources
                    .get(&(owner, id))
                    .is_none_or(|v| !v.read(cx).model.source.same_source(&source.source));
                if replace {
                    let model = source.clone();
                    let palette = self.palette.clone();
                    let entity = cx.new(|_| super::source::RawSource::new(model, palette));
                    self.floating_ui.sources.insert((owner, id), entity);
                } else {
                    self.floating_ui.sources[&(owner, id)].update(cx, |s, cx| {
                        let changed = s.model.highlight != source.highlight
                            || !source.motions.borrow().is_empty();
                        s.model = source.clone();
                        if changed {
                            cx.notify();
                        }
                    });
                }
            }
            if let Some(list) = &c.node.list {
                let entry = self.floating_ui.lists.entry((owner, id)).or_default();
                let synchronized = context
                    .ui
                    .borrow()
                    .windows
                    .iter()
                    .any(|w| w.owner == owner && w.synchronized);
                let reveal = synchronized && list.reveal.replace(false);
                if !entry.initialized || entry.selected != list.selected || reveal {
                    if let Some(index) = list
                        .selected
                        .as_ref()
                        .and_then(|id| list.ids.iter().position(|i| i == id))
                    {
                        entry.scroll.scroll_to_item(index);
                    }
                    entry.selected = list.selected.clone();
                    entry.initialized = true;
                }
            }
        }
    }
    pub(super) fn gate_pending_lua_inputs(&self, window: &mut Window, cx: &mut App) {
        let mut pending = std::collections::BTreeSet::new();
        for ((owner, _), entry) in &self.floating_ui.inputs {
            let composing = entry
                .state
                .update(cx, |s, cx| s.marked_text_range(window, cx).is_some());
            if composing || entry.state.read(cx).value().as_ref() != entry.bound_text {
                pending.insert(*owner);
            }
        }
        for w in &mut self.lua.context().ui.borrow_mut().windows {
            // Render dependencies can cross windows; protect all lists until commit.
            w.native_pending = !pending.is_empty();
        }
    }

    fn commit_lua_input(
        &mut self,
        owner: OwnerId,
        id: u64,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = self.lua.context();
        let c = context
            .ui
            .borrow()
            .windows
            .iter()
            .find(|w| w.owner == owner)
            .and_then(|w| w.components.values().find(|c| c.identity == id))
            .cloned();
        let Some(c) = c else { return };
        let Some(i) = c.node.input.clone() else {
            return;
        };
        // Native commit and an undelivered native edit are different states.
        // The callback may restore the previously rendered bind value.
        if let Some(entry) = self.floating_ui.inputs.get_mut(&(owner, id)) {
            entry.bound_text = text.clone();
        }
        self.gate_pending_lua_inputs(window, cx);
        let lua = self.lua.lua();
        let result = (|| -> mlua::Result<()> {
            let value = mlua::Value::String(lua.create_string(&text)?);
            if i.state.get(lua)? == value {
                return Ok(());
            }
            i.state.set(lua, value)?;
            if let Some(f) = i.changed {
                let callback = crate::lua::ui::tree::load_function(lua, &f)?;
                let ctx = context.clone();
                *ctx.ui.borrow_mut().defer_changes.entry(owner).or_default() += 1;
                let wrapper = lua.create_function(move |_, text: String| {
                    let result = callback.call::<()>(text);
                    if let Some(count) = ctx.ui.borrow_mut().defer_changes.get_mut(&owner) {
                        *count = count.saturating_sub(1);
                    }
                    result
                })?;
                let f = std::rc::Rc::new(lua.create_registry_value(wrapper)?);
                context.queue_component(lua, owner, c.node.key.clone(), &f, text)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            context.report_error(
                Some(owner),
                crate::lua::ErrorInfo::new(error, crate::lua::Phase::Event, None),
            );
        }
        self.pump_lua(cx);
        cx.notify();
    }
}
impl FloatingUi {
    pub(super) fn composing(
        &self,
        owner: OwnerId,
        id: u64,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        self.inputs.get(&(owner, id)).is_some_and(|i| {
            i.state
                .update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
        })
    }
}
