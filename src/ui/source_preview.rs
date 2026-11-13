//! Read-only, independently scrolled source. Cache byte positions, shape visible rows only.
use super::theme::Palette;
use crate::{app::Generation, config::Group};
use gpui_kit::{prelude::*, *};
use std::{ops::Range, sync::Arc};

/// Document-lifetime source and original newline index, reused across picker opens.
pub(super) struct SourceSnapshot {
    pub source: Arc<String>,
    pub generation: Generation,
    lines: Vec<Range<usize>>,
}

impl SourceSnapshot {
    pub fn new(source: Arc<String>, generation: Generation) -> Self {
        let lines = crate::core::source_position::SourceLines::new(&source).lines;
        Self {
            source,
            generation,
            lines,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Empty,
    Unavailable,
    Range(Range<usize>),
}

#[derive(PartialEq)]
struct WrapKey {
    width: Pixels,
    font: Font,
    font_size: Pixels,
}

pub(super) struct SourcePreview {
    snapshot: Arc<SourceSnapshot>,
    target: Target,
    rows: Vec<Range<usize>>,
    key: Option<WrapKey>,
    scroll: f32,
    viewport_height: f32,
    row_height: f32,
    reveal_pending: bool,
    palette: Palette,
}

impl SourcePreview {
    pub fn new(snapshot: Arc<SourceSnapshot>, palette: Palette) -> Self {
        Self {
            snapshot,
            target: Target::Empty,
            rows: Vec::new(),
            key: None,
            scroll: 0.,
            viewport_height: 0.,
            row_height: 1.,
            reveal_pending: false,
            palette,
        }
    }

    pub fn reveal_range(
        &mut self,
        range: Range<usize>,
        generation: Generation,
        cx: &mut Context<Self>,
    ) {
        if generation != self.snapshot.generation {
            return;
        }
        let source = &self.snapshot.source;
        if range.start >= range.end || source.get(range.clone()).is_none() {
            self.show_unavailable(cx);
            return;
        }
        self.target = Target::Range(range);
        self.reveal_pending = true;
        cx.notify();
    }

    pub fn clear_target(&mut self, cx: &mut Context<Self>) {
        self.target = Target::Empty;
        self.scroll = 0.;
        self.reveal_pending = false;
        cx.notify();
    }

    pub fn show_unavailable(&mut self, cx: &mut Context<Self>) {
        self.target = Target::Unavailable;
        self.scroll = 0.;
        self.reveal_pending = false;
        cx.notify();
    }

    pub fn scroll_half_page(&mut self, down: bool, cx: &mut Context<Self>) {
        self.scroll_by(self.viewport_height * if down { 0.5 } else { -0.5 }, cx);
    }

    fn scroll_by(&mut self, amount: f32, cx: &mut Context<Self>) {
        if !matches!(self.target, Target::Range(_)) {
            return;
        }
        self.scroll = (self.scroll + amount).clamp(0., self.max_scroll());
        cx.notify();
    }

    fn max_scroll(&self) -> f32 {
        (self.rows.len() as f32 * self.row_height - self.viewport_height).max(0.)
    }

    fn row_for_source(&self, offset: usize) -> usize {
        self.rows
            .partition_point(|row| row.start <= offset)
            .saturating_sub(1)
    }

    fn top_source(&self) -> usize {
        self.rows
            .get((self.scroll / self.row_height).floor() as usize)
            .map_or(0, |row| row.start)
    }

    fn layout(&mut self, bounds: Bounds<Pixels>, window: &mut Window) {
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let key = WrapKey {
            width: bounds.size.width.max(px(1.)),
            font: style.font(),
            font_size,
        };
        let old_anchor = self.top_source();
        let changed = self.key.as_ref() != Some(&key);
        self.viewport_height = f32::from(bounds.size.height).max(0.);
        if changed {
            let mut rows = Vec::new();
            for line in &self.snapshot.lines {
                let text = &self.snapshot.source[line.clone()];
                let mut start = line.start;
                // Use the actual shaper, not character-count or estimated cell widths.
                // Only byte positions survive reflow; shaped text is retained for visible rows only.
                if let Ok(shaped) = window.text_system().shape_text(
                    text.to_owned().into(),
                    font_size,
                    &[style.to_run(text.len())],
                    Some(key.width),
                    None,
                ) {
                    for shaped in shaped {
                        for boundary in shaped.wrap_boundaries() {
                            let glyph = &shaped.runs()[boundary.run_ix].glyphs[boundary.glyph_ix];
                            let end = line.start + glyph.index;
                            if end > start
                                && end <= line.end
                                && self.snapshot.source.is_char_boundary(end)
                            {
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
            self.scroll = self.row_for_source(old_anchor) as f32 * self.row_height;
            self.key = Some(key);
        }
        if self.reveal_pending
            && let Target::Range(range) = &self.target
        {
            let row = self.row_for_source(range.start);
            self.scroll = (row as f32 + 0.5) * self.row_height - self.viewport_height * 0.5;
            self.reveal_pending = false;
        }
        self.scroll = self.scroll.clamp(0., self.max_scroll());
    }

    fn visible_lines(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) -> Vec<(ShapedLine, Point<Pixels>)> {
        self.layout(bounds, window);
        let Target::Range(target) = &self.target else {
            return Vec::new();
        };
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let first = (self.scroll / self.row_height).floor() as usize;
        let count = (self.viewport_height / self.row_height).ceil() as usize + 1;
        self.rows
            .iter()
            .enumerate()
            .skip(first)
            .take(count)
            .map(|(index, row)| {
                let text = &self.snapshot.source[row.clone()];
                let start = target.start.max(row.start).min(row.end) - row.start;
                let end = target.end.min(row.end).max(row.start) - row.start;
                let mut runs = Vec::new();
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
                } else {
                    runs.push(style.to_run(text.len()));
                }
                let line =
                    window
                        .text_system()
                        .shape_line(text.to_owned().into(), font_size, &runs, None);
                (
                    line,
                    bounds.origin + point(px(0.), px(index as f32 * self.row_height - self.scroll)),
                )
            })
            .collect()
    }
}

impl Render for SourcePreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().clone();
        div()
            .id("source-preview")
            .debug_selector(|| "source-preview".into())
            .test_support()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
                this.scroll_by(
                    -f32::from(event.delta.pixel_delta(px(this.row_height)).y),
                    cx,
                );
                cx.stop_propagation();
            }))
            .when(self.target == Target::Unavailable, |element| {
                element.child("Source location unavailable")
            })
            .when(matches!(self.target, Target::Range(_)), |element| {
                element.child(
                    canvas(
                        move |bounds, window, cx| {
                            view.update(cx, |this, _| {
                                let lines = this.visible_lines(bounds, window);
                                (lines, px(this.row_height))
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
                                let _ =
                                    line.paint(origin, height, TextAlign::Left, None, window, cx);
                            }
                        },
                    )
                    .size_full(),
                )
            })
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
