use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use gpui_kit::component::text::{MarkdownNode, MarkdownParseContext, MarkdownPlugin, markdown_ast};
use gpui_kit::{prelude::*, *};

/// The upstream parser can run off-thread. This inbox only collects parsed data;
/// external processes are started later by Viewer, never during parsing/rendering.
pub type DiscoveredDiagrams = Arc<Mutex<BTreeMap<usize, String>>>;

pub struct MermaidPlugin {
    pub discovered: DiscoveredDiagrams,
    pub images: BTreeMap<usize, Arc<Image>>,
}

impl MarkdownPlugin for MermaidPlugin {
    fn is_block(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "evaluation-mermaid"
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::Code(code) = node else {
            return None;
        };
        if code.lang.as_deref() != Some("mermaid") {
            return None;
        }
        let start = cx.offset() + code.position.as_ref()?.start.offset;
        self.discovered
            .lock()
            .expect("diagram inbox")
            .insert(start, code.value.clone());
        // Empty plain text keeps both source and pending/failed diagrams out of search.
        Some(MarkdownNode::new(self.name().to_string(), start).accessibility_label("Mermaid図"))
    }

    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let image = node
            .data::<usize>()
            .and_then(|start| self.images.get(start));
        let loaded = image.and_then(|image| image.clone().use_render_image(window, cx));
        let start = node.data::<usize>().copied().unwrap_or_default();
        let mut element = div().debug_selector(move || format!("mermaid-{start}"));
        if let Some(loaded) = loaded {
            let size = loaded.size(0);
            let width = size.width.0 as f32;
            let height = size.height.0 as f32;
            element = element
                .w_full()
                // mmdc renders at scale 2. Fit narrower windows without stretching.
                .max_w(px(width / 2.))
                .aspect_ratio(width / height)
                .child(img(loaded).size_full());
        }
        element
    }
}
