//! Link-selection extraction adapter. This analysis-only parse uses TextView's
//! upstream parser and public block/paragraph tree; it never changes rendering.
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

use gpui_kit::{
    base::text::editor_host::{BlockNode, Paragraph, parse_markdown},
    component::text::{
        MarkdownExtensions, MarkdownNode, MarkdownParseContext, MarkdownPlugin, markdown_ast,
    },
};

use crate::core::links::{LinkCandidate, candidates};

enum Destination {
    Url(String),
    Reference(String),
}

struct ParsedLink {
    name: String,
    destination: Destination,
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
    let blocks = parse_markdown(source, extensions.into()).map_err(|error| error.to_string())?;
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

/// Root includes HTML's public tree. Candidate extraction deliberately does not
/// share link-background styling's Markdown-only exclusion.
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
    links: &mut Vec<(String, String)>,
) {
    for run in paragraph.children() {
        if let Some(link) = run.as_custom().and_then(|node| node.data::<ParsedLink>()) {
            let destination = match &link.destination {
                Destination::Url(url) => Some(url),
                Destination::Reference(id) => definitions.get(id),
            };
            if let Some(destination) = destination {
                links.push((link.name.clone(), destination.clone()));
            }
            continue;
        }
        // HTML anchors are already parsed by upstream. Their link marks carry
        // decoded labels/URLs, including formatting inside the anchor.
        for (range, mark) in run.mark_runs() {
            if let Some(link) = &mark.link
                && let Some(name) = run.text().get(range.clone())
            {
                links.push((name.to_owned(), link.url.to_string()));
            }
        }
        if let Some(image) = run.as_image()
            && let Some(link) = &image.link
        {
            links.push((
                image.alt.as_deref().unwrap_or_default().to_owned(),
                link.url.to_string(),
            ));
        }
    }
}
