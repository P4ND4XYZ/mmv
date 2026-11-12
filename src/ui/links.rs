//! Link-selection caller and extraction adapter. The analysis-only parse uses
//! TextView's upstream parser and public tree; it never changes rendering.
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    sync::{Arc, Mutex},
};

use gpui_kit::{
    base::text::editor_host::{BlockNode, Paragraph, parse_markdown},
    component::text::{
        MarkdownExtensions, MarkdownNode, MarkdownParseContext, MarkdownPlugin, markdown_ast,
    },
};

use gpui_kit::{prelude::*, *};

use super::{
    Viewer,
    floating_panel::FloatingPanel,
    selection_panel::{self, SelectionPanel},
    source_preview::SourcePreview,
    theme::Palette,
};
use crate::{
    app::Generation,
    core::links::{CandidateId, LinkCandidate, LinkKind, LinkMatcher, candidates},
};

pub(super) struct LinkSelection {
    pub generation: Generation,
    pub panel: Entity<SelectionPanel>,
    pub preview: Entity<SourcePreview>,
    matcher: LinkMatcher,
    _subscription: Subscription,
}

impl LinkSelection {
    pub fn overlay(&self, palette: &Palette) -> FloatingPanel {
        let panel = self.panel.downgrade();
        FloatingPanel::new(
            self.panel.clone(),
            0.8,
            palette.get(crate::config::Group::Border).fg,
            move |_, _, cx| {
                let _ = panel.update(cx, |_, cx| cx.emit(selection_panel::Event::Cancel));
            },
        )
    }
}

impl Viewer {
    pub(super) fn begin_link_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.normal_keys_enabled() || self.app.document().is_none() {
            return;
        }
        if let Err(error) = self.analyze_document(cx) {
            self.display_error = Some(error);
            cx.notify();
            return;
        }
        let analysis = self.document.analysis.as_ref().expect("current analysis");
        let generation = analysis.generation;
        let mut matcher = LinkMatcher::default();
        let items = panel_items(&analysis.candidates, &mut matcher, "");
        let palette = self.palette.clone();
        let source = analysis.source.clone();
        let preview = cx.new(|_| SourcePreview::new(source, palette.clone()));
        let panel =
            cx.new(|cx| SelectionPanel::new(items, "No matching links", palette, window, cx));
        panel.update(cx, |panel, _| panel.set_preview(preview.clone().into()));
        let subscription = cx.subscribe_in(&panel, window, |this, panel, event, window, cx| {
            if !this.link_panel.as_ref().is_some_and(|current| {
                current.panel == *panel
                    && this
                        .app
                        .document()
                        .is_some_and(|doc| doc.generation == current.generation)
            }) {
                return;
            }
            match event {
                selection_panel::Event::Query(query) => {
                    let selection = this.link_panel.as_mut().expect("open panel");
                    let Some(analysis) = this
                        .document
                        .analysis
                        .as_ref()
                        .filter(|analysis| analysis.generation == selection.generation)
                    else {
                        return;
                    };
                    let items = panel_items(&analysis.candidates, &mut selection.matcher, query);
                    panel.update(cx, |panel, cx| panel.set_items(items, query, cx));
                }
                selection_panel::Event::SelectionChanged(id) => {
                    let selection = this.link_panel.as_ref().expect("open panel");
                    let Some(analysis) = this
                        .document
                        .analysis
                        .as_ref()
                        .filter(|analysis| analysis.generation == selection.generation)
                    else {
                        return;
                    };
                    let range = id
                        .and_then(|id| {
                            analysis
                                .candidates
                                .iter()
                                .find(|candidate| candidate.id == CandidateId(id))
                        })
                        .and_then(|candidate| candidate.source_range.clone());
                    selection
                        .preview
                        .update(cx, |preview, cx| match (id, range) {
                            (None, _) => preview.clear_target(cx),
                            (Some(_), Some(range)) => {
                                preview.reveal_range(range, selection.generation, cx)
                            }
                            (Some(_), None) => preview.show_unavailable(cx),
                        });
                }
                selection_panel::Event::PreviewScrollRequested(down) => {
                    this.link_panel
                        .as_ref()
                        .expect("open panel")
                        .preview
                        .update(cx, |preview, cx| preview.scroll_half_page(*down, cx));
                }
                selection_panel::Event::Cancel => this.close_link_selection(window, cx),
                selection_panel::Event::Confirm(id) => {
                    let generation = this.link_panel.as_ref().expect("open panel").generation;
                    let target = this
                        .document
                        .analysis
                        .as_ref()
                        .filter(|analysis| {
                            analysis.generation == generation
                                && this
                                    .app
                                    .document()
                                    .is_some_and(|document| document.generation == generation)
                        })
                        .and_then(|analysis| {
                            analysis
                                .candidates
                                .iter()
                                .find(|candidate| candidate.id == CandidateId(*id))
                        })
                        .map(|candidate| candidate.target.clone());
                    this.close_link_selection(window, cx);
                    if let Some(target) = target {
                        this.next_operation_order += 1;
                        this.follow_link(Ok(target), this.next_operation_order, window, cx);
                    }
                }
            }
        });
        self.link_panel = Some(LinkSelection {
            generation,
            panel: panel.clone(),
            preview,
            matcher,
            _subscription: subscription,
        });
        panel.update(cx, |panel, cx| panel.activate(cx));
        cx.notify();
    }

    fn close_link_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.link_panel = None;
        self.body_focus.focus(window, cx);
        cx.notify();
    }
}

fn panel_items(
    candidates: &[LinkCandidate],
    matcher: &mut LinkMatcher,
    query: &str,
) -> Vec<selection_panel::Item> {
    let by_id: HashMap<_, _> = candidates
        .iter()
        .map(|candidate| (candidate.id, candidate))
        .collect();
    matcher
        .rank(candidates, query)
        .into_iter()
        .filter_map(|matched| {
            let candidate = by_id.get(&matched.id)?;
            Some(selection_panel::Item {
                id: candidate.id.0,
                primary: candidate.display_name.clone(),
                secondary: candidate.destination.clone(),
                badge: match candidate.kind {
                    LinkKind::Heading => "Heading",
                    LinkKind::LocalMarkdown => "Markdown",
                    LinkKind::LocalMarkdownHeading => "Markdown heading",
                    LinkKind::Web => "Web",
                },
                primary_ranges: matched.name_ranges,
                secondary_ranges: matched.destination_ranges,
            })
        })
        .collect()
}

enum Destination {
    Url(String),
    Reference(String),
}

struct ParsedLink {
    name: String,
    destination: Destination,
    source_range: Option<std::ops::Range<usize>>,
}

struct CandidatePlugin;

impl MarkdownPlugin for CandidatePlugin {
    fn name(&self) -> &str {
        "mmv-link-candidate"
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let destination = match node {
            markdown_ast::Node::Link(link) => Destination::Url(link.url.clone()),
            markdown_ast::Node::LinkReference(link) => {
                Destination::Reference(link.identifier.clone())
            }
            _ => return None,
        };
        let mut name = String::new();
        label(node, cx, &mut name);
        Some(
            MarkdownNode::new(
                self.name(),
                ParsedLink {
                    name: name.clone(),
                    destination,
                    source_range: node.position().map(|position| {
                        cx.offset() + position.start.offset..cx.offset() + position.end.offset
                    }),
                },
            )
            .text(name),
        )
    }
}

/// Labels use upstream AST values and upstream's inline HTML projection.
/// Capture the complete Markdown link as one analysis node so a link wrapping
/// text AND images remains one candidate despite the renderer's split runs.
fn label(node: &markdown_ast::Node, cx: &MarkdownParseContext<'_>, text: &mut String) {
    use markdown_ast::Node;
    match node {
        Node::Text(value) => {
            text.push_str(&value.value.replace("\r\n", " ").replace(['\r', '\n'], " "))
        }
        Node::InlineCode(value) => text.push_str(&value.value),
        // TextView retains unclaimed inline math as literal source text.
        Node::InlineMath(value) => text.push_str(cx.node_source(node).unwrap_or(&value.value)),
        Node::Image(value) => text.push_str(&value.alt),
        Node::ImageReference(value) => text.push_str(&value.alt),
        Node::Break(_) => text.push('\n'),
        Node::Html(value) => html_label(&value.value, text),
        _ => {
            if let Some(children) = node.children() {
                for child in children {
                    label(child, cx, text);
                }
            }
        }
    }
}

/// Let upstream interpret the HTML node in inline-link context. In particular,
/// a <br> must remain a newline, not be trimmed as a standalone HTML block.
/// This parse uses no candidate plugin and cannot recurse back into this helper.
fn html_label(html: &str, text: &mut String) {
    let source = format!("[{html}](#)");
    let Ok(blocks) = parse_markdown(&source, MarkdownExtensions::default().into()) else {
        text.push_str(html);
        return;
    };
    visit(&blocks, &mut |block| {
        if let BlockNode::Paragraph(paragraph) = block {
            for run in paragraph.children() {
                text.push_str(run.text());
                if let Some(image) = run.as_image() {
                    text.push_str(image.alt.as_deref().unwrap_or_default());
                }
            }
        }
    });
}

/// Observe definitions before upstream flattens containers such as footnotes
/// into the public block tree. Match upstream's document-wide first-wins rule.
fn collect_definitions(node: &markdown_ast::Node, definitions: &mut BTreeMap<String, String>) {
    if let markdown_ast::Node::Definition(definition) = node {
        definitions
            .entry(definition.identifier.clone())
            .or_insert_with(|| definition.url.clone());
    }
    if let Some(children) = node.children() {
        for child in children {
            collect_definitions(child, definitions);
        }
    }
}

pub(super) fn extract_candidates(
    document: &Path,
    source: &str,
) -> Result<Vec<LinkCandidate>, String> {
    let definitions = Arc::new(Mutex::new(BTreeMap::new()));
    let observed = definitions.clone();
    let extensions = MarkdownExtensions::default()
        .block_parser(move |node, _| {
            collect_definitions(node, &mut observed.lock().expect("link definitions"));
            None // Observe only; keep upstream's normal conversion unchanged.
        })
        .plugin(CandidatePlugin);
    let blocks = parse_markdown(source, extensions.into()).map_err(|error| {
        crate::infra::log_document_failure(document, error.as_ref());
        "Unable to analyze document links".to_string()
    })?;
    let definitions = definitions.lock().expect("link definitions");
    let mut links = Vec::new();
    visit(&blocks, &mut |block| match block {
        BlockNode::Paragraph(paragraph)
        | BlockNode::Heading {
            children: paragraph,
            ..
        } => {
            collect_paragraph(paragraph, &definitions, &mut links);
        }
        BlockNode::Table(table) => {
            for row in table.rows() {
                for paragraph in row {
                    collect_paragraph(paragraph, &definitions, &mut links);
                }
            }
        }
        _ => {}
    });
    Ok(candidates(document, links))
}

/// Visit the public tree, accepting only Markdown nodes claimed by our plugin.
/// HTML marks remain available to the body renderer, not the picker.
fn visit(blocks: &[BlockNode], f: &mut impl FnMut(&BlockNode)) {
    for block in blocks {
        f(block);
        match block {
            BlockNode::Root { children, .. }
            | BlockNode::Blockquote { children, .. }
            | BlockNode::List { children, .. }
            | BlockNode::ListItem { children, .. } => visit(children, f),
            _ => {}
        }
    }
}

fn collect_paragraph(
    paragraph: &Paragraph,
    definitions: &BTreeMap<String, String>,
    links: &mut Vec<(String, String, Option<std::ops::Range<usize>>)>,
) {
    for run in paragraph.children() {
        if let Some(link) = run.as_custom().and_then(|node| node.data::<ParsedLink>()) {
            let destination = match &link.destination {
                Destination::Url(url) => Some(url),
                Destination::Reference(id) => definitions.get(id),
            };
            if let Some(destination) = destination {
                links.push((
                    link.name.clone(),
                    destination.clone(),
                    link.source_range.clone(),
                ));
            }
            continue;
        }
    }
}
