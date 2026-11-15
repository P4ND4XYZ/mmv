use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use crate::core::HeadingIds;
use gpui_kit::{
    base::TextSelection,
    component::{
        ActiveTheme as _,
        menu::{ContextMenuExt as _, PopupMenuItem},
    },
};
use gpui_kit::{
    base::text::{
        RenderedText, TextRevealHandle, TextRevealId, TextRevealStatus, TextRevealTarget, TextView,
        editor_host::{BlockNode, parse_markdown},
    },
    component::text::{
        MarkdownExtensions, MarkdownNode, MarkdownParseContext, MarkdownPlugin, TextViewState,
        markdown_ast,
    },
};
use gpui_kit::{prelude::*, *};

use super::{
    Viewer, input,
    mermaid::{DiscoveredDiagrams, MermaidPlugin},
    status::FOOTER_LINE_HEIGHT,
};
use crate::app::{
    Generation, ImageLocation, Mode, PendingMovementPolicy, PositionRequest, PositionTarget,
    ScrollDirection, resolve_image,
};

pub(super) enum Movement {
    Scroll(ScrollDirection),
    Edge(bool),
}

struct MovementIntent {
    generation: Generation,
    order: u64,
    movement: Movement,
}

pub(super) enum DocumentView {
    Markdown(Box<MarkdownView>),
    Pdf {
        view: Entity<crate::pdf::PdfView>,
        started: Instant,
        _clicks: Subscription,
    },
}

pub(super) struct MarkdownView {
    pub text: Entity<TextViewState>,
    text_subscription: Option<Subscription>,
    pub discovered_diagrams: DiscoveredDiagrams,
    pub diagram_images: BTreeMap<usize, Arc<Image>>,
    pub analysis: Option<DocumentAnalysis>,
    layout_serial: u64,
    pane_width: Option<Pixels>,
    resize_distance: Option<f32>,
    heading_reveal: Option<(PositionRequest, u64)>,
    source_reveal: Option<SourceReveal>,
    pub(super) source_outcome: Option<SourceLineOutcome>,
    generation: Option<Generation>,
    movement_revision: u64,
    normalization: Option<Normalization>,
    movements: VecDeque<MovementIntent>,
    started: Instant,
    first_paint: Arc<AtomicBool>,
}

#[derive(Clone, Debug, PartialEq)]
struct Normalization {
    generation: Generation,
    revision: u64,
}

struct SourceReveal {
    request: PositionRequest,
    text: Entity<TextViewState>,
    handle: Option<TextRevealHandle>,
    _waiter: Option<Task<()>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SourceLineOutcome {
    Reveal(TextRevealStatus),
    Mapping(super::source_line_mapping::SourceLineMapError),
    PastEnd,
}

pub(super) struct DocumentAnalysis {
    pub generation: Generation,
    snapshot: RenderedText,
    parsed_blocks: Vec<BlockNode>,
    pub blocks: Vec<Option<std::ops::Range<usize>>>,
    pub links: Vec<std::ops::Range<usize>>,
    pub candidates: Vec<crate::core::links::LinkCandidate>,
    pub source: Arc<super::source_preview::SourceSnapshot>,
    headings: BTreeMap<String, HeadingLocation>,
    source_lines: crate::core::source_position::SourceLines,
}

struct LinkRanges(Arc<Mutex<Vec<std::ops::Range<usize>>>>);

impl MarkdownPlugin for LinkRanges {
    fn name(&self) -> &str {
        "mmv-link-ranges"
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        if matches!(
            node,
            markdown_ast::Node::Link(_) | markdown_ast::Node::LinkReference(_)
        ) && let Some(position) = node.position()
        {
            self.0
                .lock()
                .expect("link ranges")
                .push(cx.offset() + position.start.offset..cx.offset() + position.end.offset);
        }
        None
    }
}

struct HeadingLocation {
    block: usize,
    range: std::ops::Range<usize>,
    nested: bool,
}

impl MarkdownView {
    pub fn new(started: Instant, cx: &mut Context<Viewer>) -> Self {
        Self {
            text: cx.new(|cx| TextViewState::markdown("", cx)),
            text_subscription: None,
            discovered_diagrams: DiscoveredDiagrams::default(),
            diagram_images: BTreeMap::new(),
            analysis: None,
            layout_serial: 0,
            pane_width: None,
            resize_distance: None,
            heading_reveal: None,
            source_reveal: None,
            source_outcome: None,
            generation: None,
            movement_revision: 0,
            normalization: None,
            movements: VecDeque::new(),
            started,
            first_paint: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn replace(
        &mut self,
        generation: Generation,
        pending_movements: PendingMovementPolicy,
        cx: &mut Context<Viewer>,
    ) {
        self.analysis = None;
        self.pane_width = None;
        self.resize_distance = None;
        self.heading_reveal = None;
        if let Some(reveal) = self.source_reveal.take()
            && let Some(handle) = reveal.handle
        {
            handle.cancel();
        }
        self.generation = Some(generation);
        self.normalization = None;
        match pending_movements {
            PendingMovementPolicy::Carry => {
                for intent in &mut self.movements {
                    intent.generation = generation;
                }
            }
            PendingMovementPolicy::Replace => self.movements.clear(),
        }
        self.discovered_diagrams = DiscoveredDiagrams::default();
        for image in std::mem::take(&mut self.diagram_images).into_values() {
            image.remove_asset(cx);
        }
        // Install the Mermaid extension before parsing any source, including large
        // documents, so raw diagram code is never used as an initial fallback.
        self.text = cx.new(|cx| TextViewState::markdown("", cx));
        self.text_subscription = Some(cx.observe(&self.text, move |this, _, cx| {
            this.accept_rendered_text(generation, cx);
        }));
    }

    pub fn supersede_movements(&mut self, order: u64) {
        self.movements.retain(|intent| intent.order > order);
    }

    fn layout_ready(&self, cx: &gpui_kit::App) -> bool {
        let list = self.text.read(cx).list_state();
        let viewport = list.viewport_bounds().size;
        viewport.height > px(0.)
            && viewport.width > px(0.)
            && (list.is_scrolled_to_end().is_some()
                || (0..list.item_count()).all(|ix| list.bounds_for_item(ix).is_some()))
    }

    pub fn distance(&self, cx: &gpui_kit::App) -> f32 {
        -f32::from(
            self.text
                .read(cx)
                .list_state()
                .scroll_px_offset_for_scrollbar()
                .y,
        )
    }

    pub(super) fn normalization_pending(&self) -> bool {
        self.normalization.is_some()
    }

    pub fn anchor_changed(&mut self, cx: &mut Context<Viewer>) {
        self.movement_revision = self
            .movement_revision
            .checked_add(1)
            .expect("movement revision");
        self.normalization = self.generation.map(|generation| Normalization {
            generation,
            revision: self.movement_revision,
        });
        cx.notify();
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn source_handle(&self) -> Option<TextRevealHandle> {
        self.source_reveal.as_ref()?.handle.clone()
    }

    pub fn cancel_source_reveal(&self) {
        if let Some(reveal) = &self.source_reveal
            && let Some(handle) = &reveal.handle
        {
            handle.cancel();
        }
    }

    pub fn scroll(&mut self, lines: f32, pages: f32, cx: &mut Context<Viewer>) {
        self.cancel_source_reveal();
        let list = self.text.read(cx).list_state();
        let before = list.logical_scroll_top();
        list.scroll_by(px(FOOTER_LINE_HEIGHT * lines) + list.viewport_bounds().size.height * pages);
        if list.logical_scroll_top() != before {
            self.anchor_changed(cx);
        }
        self.text.update(cx, |_, cx| cx.notify());
    }

    pub fn scroll_edge(&mut self, end: bool, cx: &mut Context<Viewer>) {
        self.cancel_source_reveal();
        let list = self.text.read(cx).list_state();
        let before = list.logical_scroll_top();
        if end {
            list.scroll_to_end();
        } else {
            list.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
        }
        if list.logical_scroll_top() != before {
            self.anchor_changed(cx);
        }
        self.text.update(cx, |_, cx| cx.notify());
    }
}

impl DocumentView {
    pub fn new(started: Instant, cx: &mut Context<Viewer>) -> Self {
        Self::Markdown(Box::new(MarkdownView::new(started, cx)))
    }
    pub fn as_markdown(&self) -> Option<&MarkdownView> {
        match self {
            Self::Markdown(view) => Some(view),
            Self::Pdf { .. } => None,
        }
    }
    pub fn as_markdown_mut(&mut self) -> Option<&mut MarkdownView> {
        match self {
            Self::Markdown(view) => Some(view),
            Self::Pdf { .. } => None,
        }
    }
    fn started(&self) -> Instant {
        match self {
            Self::Markdown(view) => view.started,
            Self::Pdf { started, .. } => *started,
        }
    }
    pub fn release(&mut self, window: Option<&mut Window>, cx: &mut gpui_kit::App) {
        match self {
            Self::Pdf { view, .. } => view.update(cx, |view, cx| view.release(window, cx)),
            Self::Markdown(view) => {
                view.cancel_source_reveal();
                for image in std::mem::take(&mut view.diagram_images).into_values() {
                    image.remove_asset(cx);
                }
            }
        }
    }
    pub fn replace(
        &mut self,
        generation: Generation,
        policy: PendingMovementPolicy,
        cx: &mut Context<Viewer>,
    ) {
        if matches!(self, Self::Pdf { .. }) {
            let started = self.started();
            self.release(None, cx);
            *self = Self::new(started, cx);
        }
        if let Self::Markdown(view) = self {
            view.replace(generation, policy, cx);
        }
    }
    pub fn replace_pdf(&mut self, prepared: crate::pdf::PreparedPdf, cx: &mut Context<Viewer>) {
        let started = self.started();
        // Constructing the new host view cannot fail or reparse after app adoption.
        let view = cx.new(|cx| crate::pdf::PdfView::from_prepared(prepared, cx));
        let clicks = cx.subscribe(&view, |this, _, _: &crate::pdf::BodyClicked, cx| {
            if this.normal_keys_enabled() {
                this.restore_body_focus = true;
                cx.notify();
            }
        });
        self.release(None, cx);
        *self = Self::Pdf {
            view,
            started,
            _clicks: clicks,
        };
    }
    pub fn normalization_pending(&self) -> bool {
        self.as_markdown()
            .is_some_and(|view| view.normalization_pending())
    }
    pub fn distance(&self, cx: &gpui_kit::App) -> f32 {
        self.as_markdown().map_or(0., |view| view.distance(cx))
    }
    pub fn supersede_movements(&mut self, order: u64) {
        if let Some(view) = self.as_markdown_mut() {
            view.supersede_movements(order);
        }
    }
    pub fn cancel_source_reveal(&self) {
        if let Some(view) = self.as_markdown() {
            view.cancel_source_reveal();
        }
    }
    pub fn anchor_changed(&mut self, cx: &mut Context<Viewer>) {
        if let Some(view) = self.as_markdown_mut() {
            view.anchor_changed(cx);
        }
    }
    pub fn scroll(&mut self, lines: f32, pages: f32, cx: &mut Context<Viewer>) {
        match self {
            Self::Markdown(view) => view.scroll(lines, pages, cx),
            Self::Pdf { view, .. } => view.update(cx, |view, cx| {
                let _ = if pages != 0. {
                    view.scroll_viewport(pages, cx)
                } else {
                    view.scroll_by(lines * 40., cx)
                };
            }),
        }
    }
    pub fn scroll_edge(&mut self, end: bool, cx: &mut Context<Viewer>) {
        match self {
            Self::Markdown(view) => view.scroll_edge(end, cx),
            Self::Pdf { view, .. } => view.update(cx, |view, cx| {
                let _ = view.scroll_to_edge(end, cx);
            }),
        }
    }
}

// Existing Markdown fixtures deliberately access their engine's detailed layout.
// Production uses explicit variant checks, never a panicking Deref.
#[cfg(all(test, feature = "ui-tests"))]
impl std::ops::Deref for DocumentView {
    type Target = MarkdownView;
    fn deref(&self) -> &Self::Target {
        self.as_markdown().expect("Markdown fixture")
    }
}
#[cfg(all(test, feature = "ui-tests"))]
impl std::ops::DerefMut for DocumentView {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_markdown_mut().expect("Markdown fixture")
    }
}

impl Viewer {
    /// Share the same upstream block spans between search and heading links;
    /// heading labels come from TextView's plain rendered-text projection.
    pub(super) fn analyze_document(&mut self, cx: &gpui_kit::App) -> Result<(), String> {
        let document = self.app.document().ok_or("Document is loading")?;
        let source = document
            .markdown_source()
            .ok_or("Unable to determine document positions")?;
        let snapshot = self
            .document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .text
            .read(cx)
            .rendered_text();
        if self
            .document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .analysis
            .as_ref()
            .is_some_and(|analysis| {
                analysis.generation == document.generation && analysis.snapshot == snapshot
            })
        {
            return Ok(());
        }
        if snapshot.source() != source.as_str() {
            return Err("Document is being analyzed".into());
        }
        let links = Arc::new(Mutex::new(Vec::new()));
        let extensions = MarkdownExtensions::default()
            .plugin(MermaidPlugin {
                discovered: Default::default(),
                images: Default::default(),
            })
            .plugin(LinkRanges(links.clone()));
        let blocks = parse_markdown(source, extensions.into()).map_err(|error| {
            crate::infra::log_document_failure(&document.path, error.as_ref());
            "Unable to determine document positions".to_string()
        })?;
        let mut ids = HeadingIds::default();
        let mut headings = BTreeMap::new();
        fn collect(
            block: &BlockNode,
            ix: usize,
            nested: bool,
            ids: &mut HeadingIds,
            headings: &mut BTreeMap<String, HeadingLocation>,
            projection: &impl Fn(&BlockNode) -> (String, std::ops::Range<usize>),
        ) {
            match block {
                BlockNode::Heading { .. } => {
                    let (label, range) = projection(block);
                    headings.insert(
                        ids.next(&label),
                        HeadingLocation {
                            block: ix,
                            range,
                            nested,
                        },
                    );
                }
                BlockNode::Root { children, .. }
                | BlockNode::Blockquote { children, .. }
                | BlockNode::List { children, .. }
                | BlockNode::ListItem { children, .. } => {
                    for child in children {
                        collect(child, ix, true, ids, headings, projection);
                    }
                }
                _ => {}
            }
        }
        for (ix, block) in blocks.iter().enumerate() {
            collect(block, ix, false, &mut ids, &mut headings, &|block| {
                let range = block
                    .span()
                    .and_then(|span| snapshot.range_for_source(span.start..span.end))
                    .unwrap_or(0..0);
                (
                    snapshot.as_str()[range.clone()]
                        .trim_end_matches('\n')
                        .to_owned(),
                    range,
                )
            });
        }
        self.document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .analysis = Some(DocumentAnalysis {
            generation: document.generation,
            source_lines: crate::core::source_position::SourceLines::new(source),
            blocks: blocks
                .iter()
                .map(|block| block.span().map(|span| span.start..span.end))
                .collect(),
            links: links
                .lock()
                .expect("link ranges")
                .iter()
                .filter_map(|range| snapshot.range_for_source(range.clone()))
                .collect(),
            candidates: super::links::extract_candidates(&document.path, source)?,
            source: Arc::new(super::source_preview::SourceSnapshot::new(
                source.clone(),
                document.generation,
            )),
            headings,
            snapshot,
            parsed_blocks: blocks,
        });
        Ok(())
    }

    pub(super) fn queue_movement(
        &mut self,
        movement: Movement,
        order: u64,
        cx: &mut Context<Self>,
    ) {
        let generation = self
            .app
            .document()
            .expect("movement requires a document")
            .generation;
        if matches!(self.document, DocumentView::Pdf { .. }) {
            match movement {
                Movement::Scroll(direction) => {
                    let (lines, pages) = match direction {
                        ScrollDirection::Down => (1., 0.),
                        ScrollDirection::Up => (-1., 0.),
                        ScrollDirection::HalfDown => (0., 0.5),
                        ScrollDirection::HalfUp => (0., -0.5),
                        ScrollDirection::PageDown => (0., 1.),
                        ScrollDirection::PageUp => (0., -1.),
                    };
                    self.document.scroll(lines, pages, cx);
                }
                Movement::Edge(end) => self.document.scroll_edge(end, cx),
            }
            return;
        }
        self.document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .movements
            .push_back(MovementIntent {
                generation,
                order,
                movement,
            });
        self.settle_position(cx);
        self.apply_movements_after_layout(cx);
    }

    pub(super) fn settle_position(&mut self, cx: &mut Context<Self>) {
        if matches!(self.document, DocumentView::Pdf { .. }) {
            if let Some(request) = self.app.position_request().cloned() {
                self.document.scroll_edge(false, cx);
                self.app.acknowledge_position(&request);
            }
            return;
        }
        if let Some(request) = self.app.position_request().cloned() {
            let text = self
                .document
                .as_markdown_mut()
                .expect("Markdown variant checked")
                .text
                .clone();
            self.apply_position_after_layout(&request, &text, cx);
        }
        // Do not consume movement while preparing a new document operation.
        // Its app-produced position policy may still replace that intent.
    }

    /// Accepted movement is a UI position intent, not a read. Keep it behind
    /// Top/Distance/Heading until that generation's layout can consume it once.
    fn apply_movements_after_layout(&mut self, cx: &mut Context<Self>) {
        if self.document.as_markdown().is_none() {
            return;
        }
        let Some(document) = self.app.document() else {
            return;
        };
        let generation = document.generation;
        self.document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .movements
            .retain(|intent| intent.generation == generation);
        if self
            .document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .movements
            .is_empty()
            || self.app.position_request().is_some()
            || self.document.normalization_pending()
            || self.read_orders.values().any(|(order, line)| {
                *line
                    && self
                        .document
                        .as_markdown_mut()
                        .expect("Markdown variant checked")
                        .movements
                        .front()
                        .is_some_and(|intent| intent.order > *order)
            })
            || Some(
                self.document
                    .as_markdown_mut()
                    .expect("Markdown variant checked")
                    .text
                    .read(cx)
                    .rendered_text()
                    .source(),
            ) != document.markdown_source().map(|source| source.as_str())
            || !self
                .document
                .as_markdown()
                .is_some_and(|view| view.layout_ready(cx))
        {
            return;
        }
        while let Some(intent) = self
            .document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .movements
            .pop_front()
        {
            match intent.movement {
                Movement::Scroll(direction) => {
                    let (lines, pages) = match direction {
                        ScrollDirection::Down => (1., 0.),
                        ScrollDirection::Up => (-1., 0.),
                        ScrollDirection::HalfDown => (0., 0.5),
                        ScrollDirection::HalfUp => (0., -0.5),
                        ScrollDirection::PageDown => (0., 1.),
                        ScrollDirection::PageUp => (0., -1.),
                    };
                    self.document.scroll(lines, pages, cx);
                }
                Movement::Edge(end) => self.document.scroll_edge(end, cx),
            }
        }
    }

    /// Until layout applies a position, the new TextView's offset is not a
    /// measured departure distance. Carry the existing position intent forward
    /// instead; after acknowledgement, use the live (possibly user-moved) offset.
    pub(super) fn departure_distance(&self, cx: &gpui_kit::App) -> f32 {
        match self.app.position_request().map(|request| &request.target) {
            Some(PositionTarget::Distance(distance)) => *distance,
            Some(PositionTarget::Top) => 0.,
            _ => self.document.distance(cx),
        }
    }

    pub(super) fn apply_position_after_layout(
        &mut self,
        request: &PositionRequest,
        text: &Entity<TextViewState>,
        cx: &mut Context<Self>,
    ) {
        if self.document.as_markdown().is_none() {
            return;
        }
        if self.app.position_request() != Some(request)
            || self
                .document
                .as_markdown_mut()
                .expect("Markdown variant checked")
                .text
                != *text
            || !self.app.document().is_some_and(|doc| {
                doc.generation == request.generation
                    && Some(text.read(cx).rendered_text().source())
                        == doc.markdown_source().map(|source| source.as_str())
            })
        {
            return;
        }
        if matches!(
            request.target,
            PositionTarget::Heading(_) | PositionTarget::SourceLine(_)
        ) && let Err(error) = self.analyze_document(cx)
        {
            if matches!(request.target, PositionTarget::SourceLine(_)) {
                self.document.cancel_source_reveal();
                self.document
                    .as_markdown_mut()
                    .expect("Markdown variant checked")
                    .source_reveal = Some(SourceReveal {
                    request: request.clone(),
                    text: text.clone(),
                    handle: None,
                    _waiter: None,
                });
                self.finish_source_reveal(
                    request,
                    text,
                    None,
                    SourceLineOutcome::Mapping(
                        super::source_line_mapping::SourceLineMapError::InvalidStructure,
                    ),
                    cx,
                );
            } else {
                self.display_error = Some(error);
            }
            return;
        }
        // None also means fully measured content that fits in the viewport.
        // An empty document needs only a positive viewport from this layout.
        if self.document.normalization_pending()
            || (!matches!(request.target, PositionTarget::SourceLine(_))
                && !self
                    .document
                    .as_markdown()
                    .is_some_and(|view| view.layout_ready(cx)))
        {
            return;
        }
        let list = text.read(cx).list_state();
        if let Some((revealed, layout)) = &self
            .document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .heading_reveal
            && revealed == request
        {
            // TextView performs nested line reveals in its next layout. Do not
            // save an enclosing block's temporary offset as a departure distance.
            if *layout
                == self
                    .document
                    .as_markdown_mut()
                    .expect("Markdown variant checked")
                    .layout_serial
            {
                return;
            }
            self.document
                .as_markdown_mut()
                .expect("Markdown variant checked")
                .heading_reveal = None;
            self.app.acknowledge_position(request);
            cx.notify();
            return;
        }
        if !matches!(request.target, PositionTarget::SourceLine(_)) {
            self.document.cancel_source_reveal();
        }
        match &request.target {
            PositionTarget::Top => list.set_offset_from_scrollbar(point(px(0.), px(0.))),
            PositionTarget::Distance(distance) => {
                list.set_offset_from_scrollbar(point(px(0.), -px(distance.max(0.))))
            }
            PositionTarget::SourceLine(line) => {
                if self
                    .document
                    .as_markdown_mut()
                    .expect("Markdown variant checked")
                    .source_reveal
                    .as_ref()
                    .is_some_and(|active| active.request == *request && active.text == *text)
                {
                    return;
                }
                let previous_handle = self
                    .document
                    .as_markdown_mut()
                    .expect("Markdown variant checked")
                    .source_reveal
                    .as_ref()
                    .and_then(|active| active.handle.clone());
                self.document
                    .as_markdown_mut()
                    .expect("Markdown variant checked")
                    .source_reveal = Some(SourceReveal {
                    request: request.clone(),
                    text: text.clone(),
                    handle: None,
                    _waiter: None,
                });
                let analysis = self
                    .document
                    .as_markdown_mut()
                    .expect("Markdown variant checked")
                    .analysis
                    .as_ref()
                    .expect("current analysis");
                use super::source_line_mapping::{MappedLineTarget, map_source_line};
                let mapped = match map_source_line(
                    &analysis.source_lines,
                    &analysis.parsed_blocks,
                    &analysis.snapshot,
                    *line,
                ) {
                    Ok(mapped) => mapped,
                    Err(error) => {
                        if let Some(handle) = previous_handle {
                            handle.cancel();
                        }
                        self.finish_source_reveal(
                            request,
                            text,
                            None,
                            SourceLineOutcome::Mapping(error),
                            cx,
                        );
                        return;
                    }
                };
                let target = match mapped.target() {
                    MappedLineTarget::RenderedPosition(at) => {
                        TextRevealTarget::rendered_position(*at)
                    }
                    MappedLineTarget::CodePosition {
                        source_span,
                        offset,
                    } => TextRevealTarget::code_position(source_span.clone(), *offset),
                    MappedLineTarget::EmptyCode { source_span } => {
                        TextRevealTarget::empty_code(source_span.clone())
                    }
                    MappedLineTarget::Noop => TextRevealTarget::noop(),
                    MappedLineTarget::PastEnd => {
                        if let Some(handle) = previous_handle {
                            handle.cancel();
                        }
                        self.document.scroll_edge(true, cx);
                        self.finish_source_reveal(
                            request,
                            text,
                            None,
                            SourceLineOutcome::PastEnd,
                            cx,
                        );
                        return;
                    }
                };
                let handle = text.update(cx, |state, cx| {
                    state.request_line_reveal(mapped.snapshot(), target, cx)
                });
                let id = handle.id();
                self.document
                    .as_markdown_mut()
                    .expect("Markdown variant checked")
                    .source_reveal
                    .as_mut()
                    .unwrap()
                    .handle = Some(handle.clone());
                let status = handle.status();
                if status != TextRevealStatus::Pending {
                    self.finish_source_reveal(
                        request,
                        text,
                        Some(id),
                        SourceLineOutcome::Reveal(status),
                        cx,
                    );
                } else {
                    let request = request.clone();
                    let text = text.clone();
                    let waiter = cx.spawn(async move |view, cx| {
                        let status = handle.wait().await;
                        let _ = view.update(cx, |this, cx| {
                            this.finish_source_reveal(
                                &request,
                                &text,
                                Some(id),
                                SourceLineOutcome::Reveal(status),
                                cx,
                            );
                        });
                    });
                    self.document
                        .as_markdown_mut()
                        .expect("Markdown variant checked")
                        .source_reveal
                        .as_mut()
                        .unwrap()
                        ._waiter = Some(waiter);
                }
                return;
            }
            PositionTarget::Heading(id) => {
                if let Some(heading) = self
                    .document
                    .as_markdown_mut()
                    .expect("Markdown variant checked")
                    .analysis
                    .as_ref()
                    .and_then(|analysis| analysis.headings.get(id))
                {
                    list.scroll_to(ListOffset {
                        item_ix: heading.block,
                        offset_in_item: px(0.),
                    });
                    // Nested headings share their list row with a container.
                    // Let TextView reveal the exact projected line within it.
                    if heading.nested {
                        let range = heading.range.clone();
                        text.update(cx, |text, cx| text.reveal_range(range, cx))
                            .expect("current heading range");
                        self.document
                            .as_markdown_mut()
                            .expect("Markdown variant checked")
                            .heading_reveal = Some((
                            request.clone(),
                            self.document
                                .as_markdown_mut()
                                .expect("Markdown variant checked")
                                .layout_serial,
                        ));
                        cx.notify();
                        return;
                    }
                } else {
                    list.set_offset_from_scrollbar(point(px(0.), px(0.)));
                    self.message_is_error = false;
                    self.message = Some(format!("Heading not found: {id}"));
                }
            }
        }
        self.app.acknowledge_position(request);
        text.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    pub(super) fn finish_source_reveal(
        &mut self,
        request: &PositionRequest,
        text: &Entity<TextViewState>,
        id: Option<TextRevealId>,
        outcome: SourceLineOutcome,
        cx: &mut Context<Self>,
    ) {
        if self.document.as_markdown().is_none() {
            return;
        }
        if matches!(
            outcome,
            SourceLineOutcome::Reveal(TextRevealStatus::Pending)
        ) || self.app.position_request() != Some(request)
            || self
                .document
                .as_markdown_mut()
                .expect("Markdown variant checked")
                .text
                != *text
            || !self
                .app
                .document()
                .is_some_and(|doc| doc.generation == request.generation)
            || !self
                .document
                .as_markdown_mut()
                .expect("Markdown variant checked")
                .source_reveal
                .as_ref()
                .is_some_and(|active| {
                    active.request == *request
                        && active.text == *text
                        && active.handle.as_ref().map(TextRevealHandle::id) == id
                })
        {
            return;
        }
        self.document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .source_outcome = Some(outcome);
        self.document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .source_reveal = None;
        self.app.acknowledge_position(request);
        self.apply_movements_after_layout(cx);
        cx.notify();
    }

    pub(super) fn accept_rendered_text(&mut self, generation: Generation, cx: &mut Context<Self>) {
        if self.document.as_markdown().is_none() {
            return;
        }
        let snapshot = self
            .document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .text
            .read(cx)
            .rendered_text();
        let Some(document) = self.app.document() else {
            return;
        };
        if document.generation != generation
            || Some(snapshot.source()) != document.markdown_source().map(|source| source.as_str())
        {
            return;
        }
        let discovered = std::mem::take(
            &mut *self
                .document
                .as_markdown_mut()
                .expect("Markdown variant checked")
                .discovered_diagrams
                .lock()
                .expect("diagram inbox"),
        );
        self.app.discover_diagrams(discovered);
        self.start_next_diagram(cx);
        if self.app.rendered(generation, snapshot.as_str()) {
            self.apply_search(false, cx);
            cx.notify();
        }
    }

    pub(super) fn render_document(
        &mut self,
        path: PathBuf,
        window_width: Pixels,
        cx: &mut Context<Self>,
    ) -> Div {
        if let DocumentView::Pdf { view, .. } = &self.document {
            let view = view.clone();
            return input::bind_body_actions(div(), cx)
                .debug_selector(|| "document-body".to_owned())
                .key_context(if self.app.mode == Mode::Normal {
                    "EvaluationNormal"
                } else {
                    "EvaluationEditing"
                })
                .track_focus(&self.body_focus)
                .flex_1()
                .min_w(px(super::pane::BODY_MIN_WIDTH))
                .min_h_0()
                .relative()
                .flex()
                .child(view);
        }
        if self
            .document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .pane_width
            .is_some_and(|width| width != window_width)
            && self.app.position_request().is_none()
            && !self.document.normalization_pending()
        {
            let distance = self.document.distance(cx);
            self.document
                .as_markdown_mut()
                .expect("Markdown variant checked")
                .resize_distance
                .get_or_insert(distance);
        }
        self.document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .pane_width = Some(window_width);
        let ready = self.app.document().is_some_and(|doc| {
            Some(
                self.document
                    .as_markdown_mut()
                    .expect("Markdown variant checked")
                    .text
                    .read(cx)
                    .rendered_text()
                    .source(),
            ) == doc.markdown_source().map(|source| source.as_str())
        });
        // The child installs extensions during render. Only then hand it the source,
        // and read synchronous results here as well as asynchronous notifications.
        if let Some(generation) = self.app.document().map(|doc| doc.generation) {
            let view = cx.entity().downgrade();
            cx.defer(move |cx| {
                let _ = view.update(cx, |this, cx| {
                    if let Some(document) = this
                        .app
                        .document()
                        .filter(|doc| doc.generation == generation)
                        && let Some(source) = document.markdown_source()
                    {
                        this.document
                            .as_markdown_mut()
                            .expect("Markdown variant checked")
                            .text
                            .update(cx, |text, cx| text.set_text(source, cx));
                        this.accept_rendered_text(generation, cx);
                    }
                });
            });
        }
        let position = self.app.position_request().cloned();
        let normalization = self
            .document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .normalization
            .clone();
        let text = self
            .document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .text
            .clone();
        let layout_view = cx.entity().downgrade();
        let first_paint = self
            .document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .first_paint
            .clone();
        let started = self
            .document
            .as_markdown_mut()
            .expect("Markdown variant checked")
            .started;
        let weak = cx.entity().downgrade();
        // Bound the conversion by the actual viewport before converting f64 to f32.
        let max_width = self
            .config
            .max_width
            .min(f64::from(f32::from(window_width))) as f32;
        let centered = self.config.position == crate::config::Position::Center;
        let mut table_style = StyleRefinement::default();
        table_style.overflow.x = Some(Overflow::Scroll);
        let mut code_style = StyleRefinement::default();
        // The vertical-scroll path supplies a tracked viewport and leaves the
        // upstream language label outside it. No max height is imposed.
        // Flex gives the unwrapped text its intrinsic width for horizontal scroll.
        code_style.overflow.x = Some(Overflow::Scroll);
        code_style.overflow.y = Some(Overflow::Scroll);
        code_style.display = Some(Display::Flex);
        code_style.flex_direction = Some(FlexDirection::Row);
        code_style.text.white_space = Some(WhiteSpace::Nowrap);
        let style = self.palette.text_style(cx.theme(), table_style, code_style);

        input::bind_body_actions(div(), cx)
            .debug_selector(|| "document-body".to_string())
            .key_context(if self.app.mode == Mode::Normal {
                "EvaluationNormal"
            } else {
                "EvaluationEditing"
            })
            .track_focus(&self.body_focus)
            .flex_1()
            .min_w(px(super::pane::BODY_MIN_WIDTH))
            .min_h_0()
            .relative()
            .flex()
            .px_4()
            .py_3()
            .when(centered, |body| body.justify_center())
            .child(
                div()
                    .size_full()
                    .max_w(px(max_width))
                    .child(
                        TextView::new(
                            &self
                                .document
                                .as_markdown_mut()
                                .expect("Markdown variant checked")
                                .text,
                        )
                        .style(style)
                        .plugin(MermaidPlugin {
                            discovered: self
                                .document
                                .as_markdown_mut()
                                .expect("Markdown variant checked")
                                .discovered_diagrams
                                .clone(),
                            images: self
                                .document
                                .as_markdown_mut()
                                .expect("Markdown variant checked")
                                .diagram_images
                                .clone(),
                        })
                        .scrollable(true)
                        .size_full()
                        .max_w(px(max_width))
                        .image_source(move |uri| match resolve_image(&path, uri.as_ref()) {
                            Ok(ImageLocation::File(path)) => path.into(),
                            Ok(ImageLocation::Url(url)) => ImageSource::from(SharedUri::from(url)),
                            Err(error) => {
                                eprintln!("image {uri}: {error}");
                                // Keep failed/unsupported resources failed; never read them as local paths.
                                ImageSource::from(uri.clone())
                            }
                        })
                        .on_link_click(move |url, event, window, cx| {
                            if event.is_right_click() {
                                return;
                            }
                            let _ = weak.update(cx, |this, cx| this.open_link(url, window, cx));
                        }),
                    )
                    .context_menu(|menu, window, cx| {
                        // Snapshot before the popup takes focus or its click clears selection.
                        let selected = TextSelection::selected_text(window, cx).trim().to_owned();
                        menu.item(
                            PopupMenuItem::new("Copy")
                                .disabled(selected.is_empty())
                                .on_click(move |_, _, cx| {
                                    if !selected.is_empty() {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            selected.clone(),
                                        ));
                                    }
                                }),
                        )
                    }),
            )
            .when(ready, |body| {
                body.child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, _window, cx| {
                            // This sibling paints after TextView's measure_all layout.
                            // Defer mutation out of paint, retaining the exact request
                            // and Entity: source equality alone cannot identify a generation.
                            let view = layout_view.clone();
                            let text = text.clone();
                            let position = position.clone();
                            // List prepaint has normalized this anchor. The
                            // paint-captured marker is checked after the frame,
                            // never inferred from a serial read in this defer.
                            let normalization = normalization.clone();
                            cx.defer(move |cx| {
                                let _ = view.update(cx, |this, cx| {
                                    if this.document.as_markdown().is_none() {
                                        return;
                                    }
                                    if this
                                        .document
                                        .as_markdown_mut()
                                        .expect("Markdown variant checked")
                                        .text
                                        != text
                                    {
                                        return;
                                    }
                                    this.document
                                        .as_markdown_mut()
                                        .expect("Markdown variant checked")
                                        .layout_serial += 1;
                                    if normalization.is_some()
                                        && this
                                            .document
                                            .as_markdown_mut()
                                            .expect("Markdown variant checked")
                                            .normalization
                                            == normalization
                                    {
                                        this.document
                                            .as_markdown_mut()
                                            .expect("Markdown variant checked")
                                            .normalization = None;
                                        cx.notify();
                                    }
                                    if this
                                        .document
                                        .as_markdown()
                                        .is_some_and(|view| view.layout_ready(cx))
                                        && let Some(distance) = this
                                            .document
                                            .as_markdown_mut()
                                            .expect("Markdown variant checked")
                                            .resize_distance
                                            .take()
                                    {
                                        text.read(cx).list_state().set_offset_from_scrollbar(
                                            point(px(0.), -px(distance.max(0.))),
                                        );
                                        text.update(cx, |_, cx| cx.notify());
                                    }
                                    if let Some(request) = position {
                                        this.apply_position_after_layout(&request, &text, cx);
                                    }
                                    this.apply_movements_after_layout(cx);
                                });
                            });
                            if !first_paint.swap(true, Ordering::Relaxed) {
                                eprintln!(
                                    "first_content_paint_from_main_ms={:.3}",
                                    started.elapsed().as_secs_f64() * 1000.
                                );
                            }
                        },
                    )
                    .absolute()
                    .size_0(),
                )
            })
    }
}
