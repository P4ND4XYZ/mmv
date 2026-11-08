use gpui_kit::{
    Context,
    base::text::editor_host::parse_markdown,
    component::text::{MarkdownExtensions, RangeHighlight},
    rgb,
};

use super::{Viewer, mermaid::MermaidPlugin};

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
        if self
            .document
            .search_blocks
            .as_ref()
            .is_none_or(|(generation, _)| *generation != document.generation)
        {
            // Match TextView's extensions, but keep discovery isolated from the
            // live diagram queue. Cache only spans, not another rendering tree.
            let extensions = MarkdownExtensions::default().plugin(MermaidPlugin {
                discovered: Default::default(),
                images: Default::default(),
            });
            let blocks = parse_markdown(&document.source, extensions.into())
                .map_err(|error| format!("検索開始位置を取得できません: {error}"))?;
            self.document.search_blocks = Some((
                document.generation,
                blocks
                    .iter()
                    .map(|block| block.span().map(|span| span.start..span.end))
                    .collect(),
            ));
        }
        let (_, blocks) = self
            .document
            .search_blocks
            .as_ref()
            .expect("search block spans");
        Ok(blocks
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
        let current = self.app.current_match();
        let highlights = self
            .app
            .matches()
            .iter()
            .enumerate()
            .map(|(ix, range)| {
                RangeHighlight::new(
                    range.clone(),
                    if Some(ix) == current {
                        rgb(0xffb347).opacity(0.65)
                    } else {
                        rgb(0xffe28a).opacity(0.45)
                    },
                )
            })
            .collect::<Vec<_>>();
        let target = current.and_then(|ix| self.app.matches().get(ix)).cloned();
        let result = self.document.text.update(cx, |text, cx| {
            text.set_range_highlights(highlights, cx)?;
            if let Some(target) = target.filter(|_| reveal) {
                text.reveal_range(target, cx)?;
            }
            Ok::<_, gpui_kit::component::text::RangeHighlightError>(())
        });
        self.display_error = result.err().map(|error| error.to_string());
    }
}
