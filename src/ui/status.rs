use std::path::Path;

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{prelude::*, *};

use super::theme::Palette;
use crate::{app::Mode, config::Group};

pub(super) const FOOTER_LINE_HEIGHT: f32 = 20.;
pub(super) const MODE_WIDTH: f32 = 96.;

pub(super) fn render_mode(mode: &Mode, palette: &Palette) -> Div {
    let colors = palette.get(match mode {
        Mode::Normal => Group::ModeNormal,
        Mode::Search => Group::ModeSearch,
        Mode::Command => Group::ModeCommand,
    });
    div()
        .debug_selector(|| "mode-label".into())
        .w(px(MODE_WIDTH))
        .h(px(FOOTER_LINE_HEIGHT))
        .flex_shrink_0()
        .text_center()
        .bg(colors.bg)
        .text_color(colors.fg)
        .child(mode.label())
}

pub(super) fn render_prompt(input: &Entity<InputState>, foreground: Hsla) -> Input {
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
        .text_color(foreground)
        .text_size(px(16.))
        .line_height(px(FOOTER_LINE_HEIGHT))
}

pub(super) fn render_status_line(mode: &Mode, path: &Path, palette: &Palette) -> Div {
    let line_color = palette.get(Group::StatusLine);
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
        .border_color(palette.get(Group::Border).fg)
        .bg(line_color.bg)
        .text_color(line_color.fg)
        .child(render_mode(mode, palette))
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
    severity: Option<Group>,
    palette: &Palette,
) -> Div {
    let normal = *mode == Mode::Normal;
    let colors = palette.get(if normal {
        severity.unwrap_or(Group::CommandLine)
    } else {
        Group::CommandLine
    });
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
        .border_color(palette.get(Group::Border).fg)
        .bg(colors.bg)
        .text_color(colors.fg)
        .when(normal, |line| {
            line.child(div().flex_1().min_w_0().truncate().child(status))
        })
        .when(!normal, |line| {
            line.child(if matches!(*mode, Mode::Search) {
                "/"
            } else {
                ":"
            })
            .child(render_prompt(input, colors.fg))
        })
}
