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
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    base::text::{
        TextView,
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

pub(super) struct DocumentView {
    pub text: Entity<TextViewState>,
    text_subscription: Option<Subscription>,
    pub discovered_diagrams: DiscoveredDiagrams,
    pub diagram_images: BTreeMap<usize, Arc<Image>>,
    pub analysis: Option<DocumentAnalysis>,
    layout_serial: u64,
    pane_width: Option<Pixels>,
    resize_distance: Option<f32>,
    heading_reveal: Option<(PositionRequest, u64)>,
    movements: VecDeque<MovementIntent>,
    started: Instant,
    first_paint: Arc<AtomicBool>,
}

pub(super) struct DocumentAnalysis {
    pub generation: Generation,
    pub blocks: Vec<Option<std::ops::Range<usize>>>,
    pub links: Vec<std::ops::Range<usize>>,
    pub candidates: Vec<crate::core::links::LinkCandidate>,
    headings: BTreeMap<String, HeadingLocation>,
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

impl DocumentView {
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

    pub fn scroll(&mut self, lines: f32, pages: f32, cx: &mut Context<Viewer>) {
        let list = self.text.read(cx).list_state();
        list.scroll_by(px(FOOTER_LINE_HEIGHT * lines) + list.viewport_bounds().size.height * pages);
        self.text.update(cx, |_, cx| cx.notify());
    }

    pub fn scroll_edge(&mut self, end: bool, cx: &mut Context<Viewer>) {
        let list = self.text.read(cx).list_state();
        if end {
            list.scroll_to_end();
        } else {
            list.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
        }
        self.text.update(cx, |_, cx| cx.notify());
    }
}

impl Viewer {
    /// Share the same upstream block spans between search and heading links;
    /// heading labels come from TextView's plain rendered-text projection.
    pub(super) fn analyze_document(&mut self, cx: &gpui_kit::App) -> Result<(), String> {
        let document = self.app.document().ok_or("本文を読み込み中です")?;
        if self
            .document
            .analysis
            .as_ref()
            .is_some_and(|analysis| analysis.generation == document.generation)
        {
            return Ok(());
        }
        let snapshot = self.document.text.read(cx).rendered_text();
        if snapshot.source() != document.source {
            return Err("本文を解析中です".into());
        }
        let links = Arc::new(Mutex::new(Vec::new()));
        let extensions = MarkdownExtensions::default()
            .plugin(MermaidPlugin {
                discovered: Default::default(),
                images: Default::default(),
            })
            .plugin(LinkRanges(links.clone()));
        let blocks = parse_markdown(&document.source, extensions.into())
            .map_err(|error| format!("文書の位置を取得できません: {error}"))?;
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
        self.document.analysis = Some(DocumentAnalysis {
            generation: document.generation,
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
            candidates: super::links::extract_candidates(&document.path, &document.source)?,
            headings,
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
        self.document.movements.push_back(MovementIntent {
            generation,
            order,
            movement,
        });
        self.settle_position(cx);
        self.apply_movements_after_layout(cx);
    }

    pub(super) fn settle_position(&mut self, cx: &mut Context<Self>) {
        if let Some(request) = self.app.position_request().cloned() {
            let text = self.document.text.clone();
            self.apply_position_after_layout(&request, &text, cx);
        }
        // Do not consume movement while preparing a new document operation.
        // Its app-produced position policy may still replace that intent.
    }

    /// Accepted movement is a UI position intent, not a read. Keep it behind
    /// Top/Distance/Heading until that generation's layout can consume it once.
    fn apply_movements_after_layout(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.app.document() else {
            return;
        };
        let generation = document.generation;
        self.document
            .movements
            .retain(|intent| intent.generation == generation);
        if self.document.movements.is_empty()
            || self.app.position_request().is_some()
            || self.document.text.read(cx).rendered_text().source() != document.source
            || !self.document.layout_ready(cx)
        {
            return;
        }
        while let Some(intent) = self.document.movements.pop_front() {
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
        if self.app.position_request() != Some(request)
            || self.document.text != *text
            || !self.app.document().is_some_and(|doc| {
                doc.generation == request.generation
                    && text.read(cx).rendered_text().source() == doc.source
            })
        {
            return;
        }
        if matches!(request.target, PositionTarget::Heading(_))
            && let Err(error) = self.analyze_document(cx)
        {
            self.display_error = Some(error);
            return;
        }
        // None also means fully measured content that fits in the viewport.
        // An empty document needs only a positive viewport from this layout.
        if !self.document.layout_ready(cx) {
            return;
        }
        let list = text.read(cx).list_state();
        if let Some((revealed, layout)) = &self.document.heading_reveal
            && revealed == request
        {
            // TextView performs nested line reveals in its next layout. Do not
            // save an enclosing block's temporary offset as a departure distance.
            if *layout == self.document.layout_serial {
                return;
            }
            self.document.heading_reveal = None;
            self.app.acknowledge_position(request);
            cx.notify();
            return;
        }
        match &request.target {
            PositionTarget::Top => list.set_offset_from_scrollbar(point(px(0.), px(0.))),
            PositionTarget::Distance(distance) => {
                list.set_offset_from_scrollbar(point(px(0.), -px(distance.max(0.))))
            }
            PositionTarget::Heading(id) => {
                if let Some(heading) = self
                    .document
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
                        self.document.heading_reveal =
                            Some((request.clone(), self.document.layout_serial));
                        cx.notify();
                        return;
                    }
                } else {
                    list.set_offset_from_scrollbar(point(px(0.), px(0.)));
                    self.message_is_error = false;
                    self.message = Some(format!("見出しが見つかりません: {id}"));
                }
            }
        }
        self.app.acknowledge_position(request);
        text.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    pub(super) fn accept_rendered_text(&mut self, generation: Generation, cx: &mut Context<Self>) {
        let snapshot = self.document.text.read(cx).rendered_text();
        let Some(document) = self.app.document() else {
            return;
        };
        if document.generation != generation || snapshot.source() != document.source {
            return;
        }
        let discovered = std::mem::take(
            &mut *self
                .document
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
        if self
            .document
            .pane_width
            .is_some_and(|width| width != window_width)
            && self.app.position_request().is_none()
        {
            let distance = self.document.distance(cx);
            self.document.resize_distance.get_or_insert(distance);
        }
        self.document.pane_width = Some(window_width);
        let ready = self
            .app
            .document()
            .is_some_and(|doc| self.document.text.read(cx).rendered_text().source() == doc.source);
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
                    {
                        this.document
                            .text
                            .update(cx, |text, cx| text.set_text(&document.source, cx));
                        this.accept_rendered_text(generation, cx);
                    }
                });
            });
        }
        let position = self.app.position_request().cloned();
        let text = self.document.text.clone();
        let layout_view = cx.entity().downgrade();
        let first_paint = self.document.first_paint.clone();
        let started = self.document.started;
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
                TextView::new(&self.document.text)
                    .style(style)
                    .plugin(MermaidPlugin {
                        discovered: self.document.discovered_diagrams.clone(),
                        images: self.document.diagram_images.clone(),
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
            .when(ready, |body| {
                body.child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, _, cx| {
                            // This sibling paints after TextView's measure_all layout.
                            // Defer mutation out of paint, retaining the exact request
                            // and Entity: source equality alone cannot identify a generation.
                            let view = layout_view.clone();
                            let text = text.clone();
                            let position = position.clone();
                            cx.defer(move |cx| {
                                let _ = view.update(cx, |this, cx| {
                                    if this.document.text != text {
                                        return;
                                    }
                                    this.document.layout_serial += 1;
                                    if this.document.layout_ready(cx)
                                        && let Some(distance) = this.document.resize_distance.take()
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
