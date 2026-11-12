//! In-app frame only: content and dismissal policy belong to the caller.
use gpui_kit::{prelude::*, *};
use std::rc::Rc;

pub(super) struct DismissRequested;
type Dismiss = dyn Fn(&DismissRequested, &mut Window, &mut App);

#[derive(IntoElement)]
pub(super) struct FloatingPanel {
    content: AnyElement,
    ratio: f32,
    border: Hsla,
    dismiss: Rc<Dismiss>,
}

impl FloatingPanel {
    pub fn new(
        content: impl IntoElement,
        ratio: f32,
        border: Hsla,
        dismiss: impl Fn(&DismissRequested, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            content: content.into_any_element(),
            ratio: ratio.clamp(0., 1.),
            border,
            dismiss: Rc::new(dismiss),
        }
    }
}

impl RenderOnce for FloatingPanel {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        let viewport = window.viewport_size();
        let panel = size(viewport.width * self.ratio, viewport.height * self.ratio);
        let dismiss = self.dismiss;
        div()
            .absolute()
            .inset_0()
            .size_full()
            .child(
                div()
                    .id("selection-backdrop")
                    .debug_selector(|| "selection-backdrop".into())
                    .absolute()
                    .inset_0()
                    .size_full()
                    .occlude()
                    // Retain the shield through mouse-up: never click through.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| {
                        dismiss(&DismissRequested, window, cx);
                        cx.stop_propagation();
                    }),
            )
            .child(
                div()
                    .id("selection-frame")
                    .debug_selector(|| "selection-frame".into())
                    .absolute()
                    .left((viewport.width - panel.width) / 2.)
                    .top((viewport.height - panel.height) / 2.)
                    .w(panel.width)
                    .h(panel.height)
                    .occlude()
                    .border_1()
                    .border_color(self.border)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(self.content),
            )
    }
}
