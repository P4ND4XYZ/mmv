use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::{prelude::*, *};

use super::{
    Viewer, input,
    mermaid::{DiscoveredDiagrams, MermaidPlugin},
    status::FOOTER_LINE_HEIGHT,
};
use crate::app::{Generation, ImageLocation, Mode, resolve_image};

pub(super) struct DocumentView {
    pub text: Entity<TextViewState>,
    text_subscription: Option<Subscription>,
    pub discovered_diagrams: DiscoveredDiagrams,
    pub diagram_images: BTreeMap<usize, Arc<Image>>,
    pub search_blocks: Option<(Generation, Vec<Option<std::ops::Range<usize>>>)>,
    started: Instant,
    first_paint: Arc<AtomicBool>,
}

impl DocumentView {
    pub fn new(started: Instant, cx: &mut Context<Viewer>) -> Self {
        Self {
            text: cx.new(|cx| TextViewState::markdown("", cx)),
            text_subscription: None,
            discovered_diagrams: DiscoveredDiagrams::default(),
            diagram_images: BTreeMap::new(),
            search_blocks: None,
            started,
            first_paint: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn replace(&mut self, generation: Generation, cx: &mut Context<Viewer>) {
        self.search_blocks = None;
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

    pub(super) fn render_document(&mut self, path: PathBuf, cx: &mut Context<Self>) -> Div {
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
        let first_paint = self.document.first_paint.clone();
        let started = self.document.started;
        let weak = cx.entity().downgrade();

        input::bind_body_actions(div(), cx)
            .debug_selector(|| "document-body".to_string())
            .key_context(if self.app.mode == Mode::Normal {
                "EvaluationNormal"
            } else {
                "EvaluationEditing"
            })
            .track_focus(&self.body_focus)
            .flex_1()
            .min_h_0()
            .relative()
            .child(
                TextView::new(&self.document.text)
                    .plugin(MermaidPlugin {
                        discovered: self.document.discovered_diagrams.clone(),
                        images: self.document.diagram_images.clone(),
                    })
                    .scrollable(true)
                    .size_full()
                    .px_4()
                    .py_3()
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
                        move |_, _, _, _| {
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
