use std::path::Path;

use gpui_kit::component::{
    ActiveTheme as _,
    input::{Input, InputState},
};
use gpui_kit::{prelude::*, *};

use super::Viewer;
use crate::app::Mode;

pub(super) const FOOTER_LINE_HEIGHT: f32 = 20.;
pub(super) const MODE_WIDTH: f32 = 96.;

pub(super) fn render_status_line(mode: &Mode, path: &Path, cx: &mut Context<Viewer>) -> Div {
    let (mode_background, mode_foreground) = match mode {
        Mode::Normal => (rgb(0x2563eb), rgb(0xffffff)),
        Mode::Search => (rgb(0xfbbf24), rgb(0x111827)),
        Mode::Command => (rgb(0x7c3aed), rgb(0xffffff)),
    };
    let filename = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned();

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
                .child(mode.label()),
        )
        .child(
            div()
                .debug_selector(|| "filename".into())
                .flex_1()
                .min_w_0()
                .px_2()
                .truncate()
                .child(filename),
        )
}

pub(super) fn render_command_line(
    mode: &Mode,
    input: &Entity<InputState>,
    status: String,
    cx: &mut Context<Viewer>,
) -> Div {
    let normal = *mode == Mode::Normal;
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
            line.child(if matches!(*mode, Mode::Search) {
                "/"
            } else {
                ":"
            })
            .child(
                Input::new(input)
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
        })
}
