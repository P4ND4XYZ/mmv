use gpui_kit::{
    Context, base::text::editor_host::parse_markdown, component::text::MarkdownExtensions,
};

use super::{Viewer, mermaid::MermaidPlugin};

impl Viewer {
    /// TextView exposes its first displayed block, not the character at the top
    /// of a wrapped line. Use the same upstream parser to map that block's source
    /// span into the rendered-text snapshot; never infer offsets from Markdown lines.
    pub(super) fn search_origin(&mut self, cx: &mut Context<Self>) -> Result<usize, String> {
        let document = self.app.document.as_ref().ok_or("本文を読み込み中です")?;
        let text = self.text.read(cx);
        let snapshot = text.rendered_text();
        if snapshot.source() != document.source {
            return Err("本文を解析中です".into());
        }
        let first = text.list_state().logical_scroll_top().item_ix;
        if first == 0 {
            return Ok(0);
        }
        if self
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
            self.search_blocks = Some((
                document.generation,
                blocks
                    .iter()
                    .map(|block| block.span().map(|span| span.start..span.end))
                    .collect(),
            ));
        }
        let (_, blocks) = self.search_blocks.as_ref().expect("search block spans");
        Ok(blocks
            .iter()
            .skip(first)
            .filter_map(|span| snapshot.range_for_source(span.clone()?))
            .map(|range| range.start)
            .next()
            .unwrap_or(snapshot.len()))
    }
}
