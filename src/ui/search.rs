use crate::config::Group;
use gpui_kit::{Context, component::text::RangeHighlight};

use super::Viewer;

impl Viewer {
    /// TextView exposes its first displayed block, not the character at the top
    /// of a wrapped line. Use the same upstream parser to map that block's source
    /// span into the rendered-text snapshot; never infer offsets from Markdown lines.
    pub(super) fn search_origin(&mut self, cx: &mut Context<Self>) -> Result<usize, String> {
        let document = self.app.document().ok_or("本文を読み込み中です")?;
        let text = self.document.text.read(cx);
        let snapshot = text.rendered_text();
        if snapshot.source() != document.source {
            return Err("本文を解析中です".into());
        }
        let first = text.list_state().logical_scroll_top().item_ix;
        if first == 0 {
            return Ok(0);
        }
        self.analyze_document(cx)?;
        let analysis = self.document.analysis.as_ref().expect("document analysis");
        Ok(analysis
            .blocks
            .iter()
            .skip(first)
            .filter_map(|span| snapshot.range_for_source(span.clone()?))
            .map(|range| range.start)
            .next()
            .unwrap_or(snapshot.len()))
    }

    pub(super) fn apply_search(&mut self, reveal: bool, cx: &mut Context<Self>) {
        let Some(document) = self.app.document() else {
            return;
        };
        if self.document.text.read(cx).rendered_text().source() != document.source {
            return;
        }
        if let Err(error) = self.analyze_document(cx) {
            self.display_error = Some(error);
            return;
        }
        let highlights = self.range_highlights();
        let target = self
            .app
            .current_match()
            .and_then(|ix| self.app.matches().get(ix))
            .cloned();
        let result = self.document.text.update(cx, |text, cx| {
            text.set_range_highlights(highlights, cx)?;
            if let Some(target) = target.filter(|_| reveal) {
                text.reveal_range(target, cx)?;
            }
            Ok::<_, gpui_kit::component::text::RangeHighlightError>(())
        });
        self.display_error = result.err().map(|error| error.to_string());
    }

    pub(super) fn range_highlights(&self) -> Vec<RangeHighlight> {
        let link = self.palette.get(Group::Link).bg;
        let mut highlights = self
            .document
            .analysis
            .as_ref()
            .expect("current analysis")
            .links
            .iter()
            .cloned()
            .map(|range| RangeHighlight::new(range, link))
            .collect::<Vec<_>>();
        highlights.extend(
            self.app
                .matches()
                .iter()
                .filter(|_| self.app.highlights_visible())
                .enumerate()
                .map(|(ix, range)| {
                    RangeHighlight::new(
                        range.clone(),
                        self.palette
                            .get(if Some(ix) == self.app.current_match() {
                                Group::SearchCurrent
                            } else {
                                Group::Search
                            })
                            .bg,
                    )
                }),
        );
        highlights
    }
}
