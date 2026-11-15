//! Wrapped raw source, sharing snapshot bytes and retaining a source-byte reflow anchor.
use super::super::theme::Palette;
use crate::{
    config::Group,
    lua::ui::source::{Align, Motion, Source},
};
use gpui_kit::{prelude::*, *};
use std::ops::Range;

pub(super) struct RawSource {
    pub model: Source,
    lines: Vec<Range<usize>>,
    rows: Vec<Range<usize>>,
    key: Option<(Pixels, Font, Pixels)>,
    pub scroll: f32,
    pub viewport: f32,
    row_height: f32,
    palette: Palette,
}
impl RawSource {
    pub fn new(model: Source, palette: Palette) -> Self {
        let lines = crate::core::source_position::SourceLines::new(&model.source.text()).lines;
        Self {
            model,
            lines,
            rows: Vec::new(),
            key: None,
            scroll: 0.,
            viewport: 0.,
            row_height: 1.,
            palette,
        }
    }
    fn row(&self, byte: usize) -> usize {
        self.rows
            .partition_point(|r| r.start <= byte)
            .saturating_sub(1)
    }
    pub(super) fn top_source(&self) -> usize {
        self.rows
            .get((self.scroll / self.row_height).floor() as usize)
            .map_or(0, |r| r.start)
    }
    fn max(&self) -> f32 {
        (self.rows.len() as f32 * self.row_height - self.viewport).max(0.)
    }
    fn layout(&mut self, bounds: Bounds<Pixels>, window: &mut Window) {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let key = (bounds.size.width.max(px(1.)), style.font(), font_size);
        let source = self.model.source.text();
        let anchor = self.top_source();
        self.viewport = f32::from(bounds.size.height).max(0.);
        if self.key.as_ref() != Some(&key) {
            let mut rows = Vec::new();
            for line in &self.lines {
                let text = &source[line.clone()];
                let mut start = line.start;
                if let Ok(shaped) = window.text_system().shape_text(
                    text.to_owned().into(),
                    font_size,
                    &[style.to_run(text.len())],
                    Some(key.0),
                    None,
                ) {
                    for shaped in shaped {
                        for boundary in shaped.wrap_boundaries() {
                            let glyph = &shaped.runs()[boundary.run_ix].glyphs[boundary.glyph_ix];
                            let end = line.start + glyph.index;
                            if end > start && end <= line.end && source.is_char_boundary(end) {
                                rows.push(start..end);
                                start = end;
                            }
                        }
                    }
                }
                rows.push(start..line.end);
            }
            self.rows = rows;
            self.row_height = f32::from(font_size) * 1.5;
            self.scroll = self.row(anchor) as f32 * self.row_height;
            self.key = Some(key);
        }
        let motions = std::mem::take(&mut *self.model.motions.borrow_mut());
        for motion in motions {
            match motion {
                Motion::Pages(n) => {
                    self.scroll = (f64::from(self.scroll) + n * f64::from(self.viewport))
                        .clamp(0., f64::from(self.max())) as f32
                }
                Motion::Lines(n) => {
                    self.scroll = (f64::from(self.scroll) + n * f64::from(self.row_height))
                        .clamp(0., f64::from(self.max())) as f32
                }
                Motion::Edge(end) => self.scroll = if end { self.max() } else { 0. },
                Motion::Range(r, align) => {
                    let start = self.row(r.start) as f32 * self.row_height;
                    let end = (self.row(r.end.saturating_sub(usize::from(r.end > r.start))) + 1)
                        as f32
                        * self.row_height;
                    self.scroll = match align {
                        Align::Start => start,
                        Align::Center => (start + end - self.viewport) * 0.5,
                        Align::End => end - self.viewport,
                        Align::Nearest if end - start > self.viewport => start,
                        Align::Nearest if start < self.scroll => start,
                        Align::Nearest if end > self.scroll + self.viewport => end - self.viewport,
                        Align::Nearest => self.scroll,
                    };
                }
            }
            self.scroll = self.scroll.clamp(0., self.max());
        }
        self.scroll = self.scroll.clamp(0., self.max());
    }
    fn visible(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) -> Vec<(ShapedLine, Point<Pixels>)> {
        self.layout(bounds, window);
        let source = self.model.source.text();
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let first = (self.scroll / self.row_height).floor() as usize;
        let count = (self.viewport / self.row_height).ceil() as usize + 1;
        self.rows
            .iter()
            .enumerate()
            .skip(first)
            .take(count)
            .map(|(i, row)| {
                let text = &source[row.clone()];
                let mut runs = Vec::new();
                if let Some(target) = &self.model.highlight {
                    let start = target.start.max(row.start).min(row.end) - row.start;
                    let end = target.end.min(row.end).max(row.start) - row.start;
                    if start < end {
                        if start > 0 {
                            runs.push(style.to_run(start));
                        }
                        let mut selected = style.to_run(end - start);
                        selected.background_color =
                            Some(self.palette.get(Group::PanelSelected).bg.opacity(0.35));
                        runs.push(selected);
                        if end < text.len() {
                            runs.push(style.to_run(text.len() - end));
                        }
                    }
                }
                if runs.is_empty() {
                    runs.push(style.to_run(text.len()));
                }
                (
                    window
                        .text_system()
                        .shape_line(text.to_owned().into(), font_size, &runs, None),
                    bounds.origin + point(px(0.), px(i as f32 * self.row_height - self.scroll)),
                )
            })
            .collect()
    }
}
#[cfg(all(test, feature = "ui-tests"))]
mod tests;

impl Render for RawSource {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().clone();
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                this.scroll = (this.scroll
                    - f32::from(event.delta.pixel_delta(px(this.row_height)).y))
                .clamp(0., this.max());
                cx.stop_propagation();
                cx.notify();
            }))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        view.update(cx, |this, _| {
                            (this.visible(bounds, window), px(this.row_height))
                        })
                    },
                    |_, (lines, height), window, cx| {
                        for (line, origin) in &lines {
                            let _ = line.paint_background(
                                *origin,
                                height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                        for (line, origin) in lines {
                            let _ = line.paint(origin, height, TextAlign::Left, None, window, cx);
                        }
                    },
                )
                .size_full(),
            )
    }
}
