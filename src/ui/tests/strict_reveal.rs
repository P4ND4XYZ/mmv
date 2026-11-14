//! Bounded feasibility checks using the existing renderer, without production bootstrap.
use gpui_kit::base::text::{
    TextRevealCancellation, TextRevealFailure, TextRevealHandle, TextRevealStatus,
    TextRevealTarget, TextView, TextViewState,
};
use gpui_kit::base::text::{
    TextViewStyle,
    editor_host::{
        BlockNode, CodeBlock, ColumnAlign, FlowPosition, InlineNode, LeafGeometry, Paragraph,
        RenderContext, Span, Table, collect_leaf_geometry,
    },
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, AvailableSpace, Bounds, Context, Element, ElementId,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, ParentElement as _, Pixels, Render,
    Styled as _, TestAppContext, Window, div, px, size,
};
use gpui_kit::{
    Entity, SharedString, WindowBounds, WindowOptions, point, relative, test::TestWindowExt as _,
};
use std::{cell::RefCell, collections::HashMap, rc::Rc};

type Geometry = Rc<RefCell<HashMap<u64, LeafGeometry>>>;
struct Collect {
    child: AnyElement,
    measurement_blocks: Vec<BlockNode>,
    context: RenderContext,
    output: Geometry,
}
impl IntoElement for Collect {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for Collect {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let (_, geometry) =
            collect_leaf_geometry(Default::default(), || self.child.prepaint(window, cx));
        let previous_autoscroll = window.take_autoscroll();
        let measured = window.with_element_namespace("strict-feasibility-measurement", |window| {
            let mut element = render_blocks(&self.measurement_blocks, &self.context, window, cx);
            element.layout_as_root(
                size(
                    AvailableSpace::Definite(px(400.)),
                    AvailableSpace::MinContent,
                ),
                window,
                cx,
            );
            let result: Result<(), _> = window.transact(|window| {
                let (_, measured) = collect_leaf_geometry(Default::default(), || {
                    element.prepaint_at(bounds.origin, window, cx);
                });
                Err(measured)
            });
            result.unwrap_err()
        });
        assert!(
            window.take_autoscroll().is_none(),
            "measurement requested an autoscroll"
        );
        if let Some(previous) = previous_autoscroll {
            window.request_autoscroll(previous);
        }
        for key in 1..=4 {
            assert_eq!(
                geometry[&key].bounds(),
                measured[&key].bounds(),
                "cloned renderer leaf {key}"
            );
        }
        *self.output.borrow_mut() = geometry;
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}
struct Gate {
    blocks: Vec<BlockNode>,
    output: Geometry,
}
impl Render for Gate {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let context = RenderContext::new(TextViewStyle::default()).with_body_font_size(px(16.));
        Collect {
            child: render_blocks(&self.blocks, &context, window, cx),
            measurement_blocks: detached_plain_fixture(),
            context,
            output: self.output.clone(),
        }
    }
}
fn paragraph(text: &str, key: u64) -> Paragraph {
    let mut p = Paragraph::default();
    p.push(InlineNode::new(text.to_owned()));
    p.with_leaf_key(key)
}
fn render_blocks(
    blocks: &[BlockNode],
    context: &RenderContext,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let refs: Vec<_> = blocks.iter().collect();
    let children: Vec<_> = refs
        .iter()
        .enumerate()
        .map(|(ix, block)| {
            block.render(
                ix,
                FlowPosition::at(&refs, ix, ix + 1 == refs.len()),
                context,
                window,
                cx,
            )
        })
        .collect();
    div().w(px(400.)).children(children).into_any_element()
}

// Deliberately rebuilt runtime state for this bounded plain-text fixture. This
// does not establish isolation of arbitrary cloned ASTs or inline-flow caches.
fn detached_plain_fixture() -> Vec<BlockNode> {
    vec![
        BlockNode::Blockquote {
            children: vec![BlockNode::List {
                children: vec![BlockNode::ListItem {
                    children: vec![BlockNode::Paragraph(paragraph("nested body", 1))],
                    spread: false,
                    checked: None,
                    span: None,
                }],
                ordered: false,
                start: None,
                span: None,
            }],
            span: None,
        },
        BlockNode::Table(Table::new(
            vec![vec![paragraph("head", 2)], vec![paragraph("cell body", 3)]],
            vec![ColumnAlign::Left],
        )),
        BlockNode::CodeBlock(
            CodeBlock::new("".into(), Some("rust".into()), None::<Span>).with_leaf_key(4),
        ),
    ]
}

pub(super) fn geometry_gate(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let output = Geometry::default();
    let blocks = detached_plain_fixture();
    let (window, _) = cx.update(|cx| {
        gpui_kit::open_window(Default::default(), cx, |_, cx| {
            cx.new(|_| Gate {
                blocks,
                output: output.clone(),
            })
        })
        .expect("offscreen gate window")
    });
    let visual = &mut gpui_kit::VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear(cx));
    visual.run_until_parked();
    let geometry = output.borrow();
    for key in 1..=4 {
        assert!(geometry.contains_key(&key), "missing leaf {key}");
    }
    assert!(geometry[&1].bounds().top() < geometry[&2].bounds().top());
    assert!(geometry[&2].bounds().top() < geometry[&3].bounds().top());
    assert!(geometry[&4].is_empty());
    assert!(geometry[&4].bounds().size.height > px(0.));
    eprintln!("GEOMETRY_GATE nested=true table=true empty_line_box=true");
}
#[gpui_kit::test]
fn strict_reveal_noop_geometry_gate(cx: &mut TestAppContext) {
    geometry_gate(cx);
}

struct StrictRoot {
    scrollable: bool,
    clamp: Option<usize>,
    text: Entity<TextViewState>,
    style: TextViewStyle,
    frames: Rc<RefCell<Vec<gpui_kit::Point<Pixels>>>>,
}
impl Render for StrictRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let text = self.text.clone();
        let frames = self.frames.clone();
        div()
            .size_full()
            .relative()
            .text_size(px(16.))
            .line_height(relative(1.4))
            .child(
                TextView::new(&self.text)
                    .selectable(true)
                    .scrollable(self.scrollable)
                    .when_some(self.clamp, |view, lines| view.max_lines(lines))
                    .style(self.style.clone())
                    .size_full(),
            )
            .child(
                gpui_kit::canvas(
                    move |_, _, cx| {
                        frames
                            .borrow_mut()
                            .push(text.read(cx).list_state().scroll_px_offset_for_scrollbar());
                    },
                    |_, _, _, _| (),
                )
                .absolute()
                .size_0(),
            )
    }
}
fn strict_open(
    cx: &mut TestAppContext,
    source: &str,
    style: TextViewStyle,
) -> (
    gpui_kit::AnyWindowHandle,
    Entity<StrictRoot>,
    Entity<TextViewState>,
) {
    cx.update(gpui_kit::init);
    let (window, root) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(480.), px(260.)),
                ))),
                ..Default::default()
            },
            cx,
            |_, cx| {
                cx.new(|cx| StrictRoot {
                    scrollable: true,
                    clamp: None,
                    text: cx.new(|cx| TextViewState::markdown(source, cx)),
                    style,
                    frames: Default::default(),
                })
            },
        )
        .unwrap()
    });
    let text = root.read_with(cx, |root, _| root.text.clone());
    strict_frames(cx, window.into(), 2);
    (window.into(), root, text)
}
fn strict_frames(cx: &mut TestAppContext, window: gpui_kit::AnyWindowHandle, count: usize) {
    for _ in 0..count {
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.simulate_next_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
}
fn strict_request(
    cx: &mut TestAppContext,
    text: &Entity<TextViewState>,
    target: TextRevealTarget,
) -> TextRevealHandle {
    cx.update(|cx| {
        text.update(cx, |state, cx| {
            let snapshot = state.rendered_text();
            state.request_line_reveal(&snapshot, target, cx)
        })
    })
}
fn position(cx: &TestAppContext, text: &Entity<TextViewState>, needle: &str) -> usize {
    text.read_with(cx, |state, _| {
        state.rendered_text().as_str().find(needle).unwrap()
    })
}
fn offset(cx: &TestAppContext, text: &Entity<TextViewState>) -> gpui_kit::Point<Pixels> {
    text.read_with(cx, |state, _| {
        state.list_state().scroll_px_offset_for_scrollbar()
    })
}
fn code_spans(block: &BlockNode, out: &mut Vec<(std::ops::Range<usize>, SharedString)>) {
    match block {
        BlockNode::Root { children, .. }
        | BlockNode::Blockquote { children, .. }
        | BlockNode::List { children, .. }
        | BlockNode::ListItem { children, .. } => {
            for child in children {
                code_spans(child, out);
            }
        }
        BlockNode::CodeBlock(code) => {
            let span = code.span.unwrap();
            out.push((span.start..span.end, code.code()));
        }
        _ => {}
    }
}
fn codes(source: &str) -> Vec<(std::ops::Range<usize>, SharedString)> {
    let blocks =
        gpui_kit::base::text::editor_host::parse_markdown(source, Default::default()).unwrap();
    let mut out = Vec::new();
    for block in &blocks {
        code_spans(block, &mut out);
    }
    out
}
pub(super) fn strict_contract_cases(cx: &mut TestAppContext) {
    let source = format!(
        "visible first\n\n{}\n\n> - nested target\n\n| head |\n| --- |\n| table target |\n\n```rust\n{}\n```\n\n```\n```\n\nend",
        (0..30)
            .map(|i| format!("paragraph {i}\n\n"))
            .collect::<String>(),
        (0..180)
            .map(|i| format!("code line {i}\n"))
            .collect::<String>()
    );
    let (window, root, text) = strict_open(cx, &source, TextViewStyle::default());
    let before = offset(cx, &text);
    let handle = strict_request(
        cx,
        &text,
        TextRevealTarget::rendered_position(position(cx, &text, "visible first")),
    );
    strict_frames(cx, window, 3);
    assert_eq!(handle.status(), TextRevealStatus::Shown, "already visible");
    assert_eq!(offset(cx, &text), before);
    let code = codes(&source);
    let target = code[0].1.find("code line 150").unwrap();
    mark(cx, &text, position(cx, &text, "code line 150"));
    root.read_with(cx, |root, _| root.frames.borrow_mut().clear());
    let handle = strict_request(
        cx,
        &text,
        TextRevealTarget::code_position(code[0].0.clone(), target),
    );
    strict_frames(cx, window, 4);
    assert_eq!(handle.status(), TextRevealStatus::Shown, "direct long code");
    let final_offset = offset(cx, &text);
    assert!(final_offset.y < before.y);
    assert_row_visible(cx, window, &text);
    root.read_with(cx, |root, _| {
        assert!(
            root.frames
                .borrow()
                .iter()
                .all(|offset| *offset == before || *offset == final_offset),
            "intermediate scroll frame"
        )
    });
    for needle in ["nested target", "table target"] {
        mark(cx, &text, position(cx, &text, needle));
        let handle = strict_request(
            cx,
            &text,
            TextRevealTarget::rendered_position(position(cx, &text, needle)),
        );
        strict_frames(cx, window, 4);
        assert_eq!(handle.status(), TextRevealStatus::Shown, "{needle}");
        assert_row_visible(cx, window, &text);
    }
    let handle = strict_request(cx, &text, TextRevealTarget::empty_code(code[1].0.clone()));
    strict_frames(cx, window, 4);
    assert_eq!(
        handle.status(),
        TextRevealStatus::Shown,
        "empty code border box"
    );

    let mut style = gpui_kit::StyleRefinement::default();
    style.overflow.y = Some(gpui_kit::Overflow::Scroll);
    style.overflow.x = Some(gpui_kit::Overflow::Scroll);
    style.display = Some(gpui_kit::Display::Flex);
    style.flex_direction = Some(gpui_kit::FlexDirection::Row);
    style.max_size.height = Some(px(100.).into());
    style.text.white_space = Some(gpui_kit::WhiteSpace::Nowrap);
    let (window, _, text) =
        strict_open(cx, &source, TextViewStyle::default().with_code_block(style));
    let handle = strict_request(
        cx,
        &text,
        TextRevealTarget::code_position(code[0].0.clone(), target),
    );
    strict_frames(cx, window, 4);
    assert_eq!(
        handle.status(),
        TextRevealStatus::Shown,
        "fixed-height inner code scroller"
    );
    let handle = strict_request(
        cx,
        &text,
        TextRevealTarget::code_position(code[0].0.clone(), 0),
    );
    strict_frames(cx, window, 4);
    assert_eq!(
        handle.status(),
        TextRevealStatus::Shown,
        "inner offset is preserved in measurement, then minimally changed"
    );
    let empty = strict_request(cx, &text, TextRevealTarget::empty_code(code[1].0.clone()));
    strict_frames(cx, window, 4);
    assert_eq!(
        empty.status(),
        TextRevealStatus::Shown,
        "empty border box is not clipped by its own content viewport"
    );
}
#[gpui_kit::test]
fn strict_reveal_contract(cx: &mut TestAppContext) {
    strict_contract_cases(cx);
}

#[gpui_kit::test]
fn strict_reveal_results_cancel_timeout_stale_and_legacy(cx: &mut TestAppContext) {
    let (window, _, text) = strict_open(cx, "first\n\nsecond", TextViewStyle::default());
    let first = strict_request(cx, &text, TextRevealTarget::rendered_position(0));
    let second = strict_request(cx, &text, TextRevealTarget::rendered_position(0));
    assert_eq!(
        first.status(),
        TextRevealStatus::Cancelled(TextRevealCancellation::Superseded)
    );
    assert!(second.cancel());
    assert!(!second.cancel());
    assert_eq!(
        second.status(),
        TextRevealStatus::Cancelled(TextRevealCancellation::Explicit)
    );
    let pending = strict_request(cx, &text, TextRevealTarget::rendered_position(0));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(
        pending.status(),
        TextRevealStatus::Failed(TextRevealFailure::TimedOut),
        "no-frame deadline"
    );
    let pending = strict_request(cx, &text, TextRevealTarget::rendered_position(0));
    cx.update(|cx| {
        text.update(cx, |state, cx| {
            assert!(state.reveal_range(usize::MAX..usize::MAX, cx).is_err());
            assert_eq!(pending.status(), TextRevealStatus::Pending);
            state.reveal_range(0..1, cx).unwrap();
        })
    });
    assert_eq!(
        pending.status(),
        TextRevealStatus::Cancelled(TextRevealCancellation::Superseded)
    );
    let snapshot = text.read_with(cx, |state, _| state.rendered_text());
    let pending = strict_request(cx, &text, TextRevealTarget::rendered_position(0));
    cx.update(|cx| text.update(cx, |state, cx| state.set_text("replacement", cx)));
    assert_eq!(
        pending.status(),
        TextRevealStatus::Cancelled(TextRevealCancellation::ContentChanged)
    );
    let stale = cx.update(|cx| {
        text.update(cx, |state, cx| {
            state.request_line_reveal(&snapshot, TextRevealTarget::rendered_position(0), cx)
        })
    });
    assert_eq!(
        stale.status(),
        TextRevealStatus::Failed(TextRevealFailure::StaleSnapshot)
    );
    strict_frames(cx, window, 3);
    assert_eq!(
        pending.status(),
        TextRevealStatus::Cancelled(TextRevealCancellation::ContentChanged)
    );
}
pub(super) fn strict_private_case(cx: &mut TestAppContext) {
    let Some(path) = std::env::var_os("MMV_PRIVATE_SOURCE_FIXTURE") else {
        return;
    };
    let source = std::fs::read_to_string(path).expect("private source fixture read");
    let code = codes(&source)
        .into_iter()
        .max_by_key(|(_, code)| code.len())
        .expect("private code block");
    let target = code
        .1
        .char_indices()
        .nth(code.1.chars().count() / 2)
        .unwrap()
        .0;
    let mut app_code = gpui_kit::StyleRefinement::default();
    app_code.overflow.x = Some(gpui_kit::Overflow::Scroll);
    app_code.overflow.y = Some(gpui_kit::Overflow::Scroll);
    app_code.display = Some(gpui_kit::Display::Flex);
    app_code.flex_direction = Some(gpui_kit::FlexDirection::Row);
    app_code.text.white_space = Some(gpui_kit::WhiteSpace::Nowrap);
    for style in [
        TextViewStyle::default(),
        TextViewStyle::default().with_code_block(app_code),
    ] {
        let (window, _, text) = strict_open(cx, &source, style);
        let handle = strict_request(
            cx,
            &text,
            TextRevealTarget::code_position(code.0.clone(), target),
        );
        strict_frames(cx, window, 4);
        assert_eq!(
            handle.status(),
            TextRevealStatus::Shown,
            "private native code target"
        );
    }
    eprintln!("PRIVATE_NATIVE_CODE verified=true");
}

#[gpui_kit::test]
fn strict_reveal_nested_code_blank_end_offsets_and_opaque_failures(cx: &mut TestAppContext) {
    let source = format!(
        "{}\n\n> - quoted list\n>\n>   ```lua\n>   first\n>\n>   last\n>\n>   ```\n\nend",
        "before\n\n".repeat(30)
    );
    let code = codes(&source);
    assert_eq!(code.len(), 1);
    assert!(
        code[0].1.ends_with('\n'),
        "fixture retains its final empty code row"
    );
    let (window, _, text) = strict_open(cx, &source, TextViewStyle::default());
    for at in [
        0,
        code[0].1.find('\n').unwrap(),
        code[0].1.find("\n\n").unwrap() + 1,
        code[0].1.len(),
    ] {
        let handle = strict_request(
            cx,
            &text,
            TextRevealTarget::code_position(code[0].0.clone(), at),
        );
        strict_frames(cx, window, 4);
        assert_eq!(
            handle.status(),
            TextRevealStatus::Shown,
            "exact nested code offset {at}"
        );
    }
    let bad = strict_request(
        cx,
        &text,
        TextRevealTarget::code_position(code[0].0.clone(), code[0].1.len() + 1),
    );
    assert_eq!(
        bad.status(),
        TextRevealStatus::Failed(TextRevealFailure::InvalidTarget)
    );

    let source = "![opaque](data:image/svg+xml;utf8,<svg/>)";
    let (window, _, text) = strict_open(cx, source, TextViewStyle::default());
    let before = offset(cx, &text);
    let handle = strict_request(cx, &text, TextRevealTarget::rendered_position(0));
    assert!(matches!(handle.status(), TextRevealStatus::Failed(_)));
    strict_frames(cx, window, 3);
    assert_eq!(offset(cx, &text), before);
}

#[gpui_kit::test]
fn strict_reveal_actual_thumb_down_cancels_latest_without_motion(cx: &mut TestAppContext) {
    let source = "body paragraph\n\n".repeat(40);
    let (window, _, text) = strict_open(cx, &source, TextViewStyle::default());
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    let body = point(px(40.), px(40.));
    visual.simulate_event(gpui_kit::MouseMoveEvent {
        position: body,
        pressed_button: None,
        modifiers: Default::default(),
    });
    for delta in [-20., 20.] {
        visual.simulate_event(gpui_kit::ScrollEvent {
            position: body,
            delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(delta))),
            touch_phase: gpui_kit::TouchPhase::Moved,
            ..Default::default()
        });
        visual.update(|window, cx| window.render_frame(cx));
    }
    let viewport = text.read_with(&visual, |state, _| state.list_state().viewport_bounds());
    let thumb = point(viewport.right() - px(8.), viewport.top() + px(12.));
    visual.simulate_event(gpui_kit::MouseMoveEvent {
        position: thumb,
        pressed_button: None,
        modifiers: Default::default(),
    });
    visual.update(|window, cx| window.render_frame(cx));
    let before = offset(cx, &text);
    let pending = strict_request(cx, &text, TextRevealTarget::rendered_position(0));
    assert_eq!(pending.status(), TextRevealStatus::Pending);
    // The scrollbar element was painted before this new request. Its real
    // thumb-down handler must reach the latest shared pending record.
    visual.simulate_mouse_down(thumb, gpui_kit::MouseButton::Left, Default::default());
    assert_eq!(
        offset(cx, &text),
        before,
        "no thumb motion or offset change"
    );
    assert_eq!(
        pending.status(),
        TextRevealStatus::Cancelled(TextRevealCancellation::UserScroll)
    );
    let during_drag = strict_request(cx, &text, TextRevealTarget::rendered_position(0));
    assert_eq!(
        during_drag.status(),
        TextRevealStatus::Cancelled(TextRevealCancellation::UserScroll)
    );
    strict_frames(cx, window, 3);
    assert_eq!(offset(cx, &text), before);
    assert_eq!(
        pending.status(),
        TextRevealStatus::Cancelled(TextRevealCancellation::UserScroll)
    );
    visual.simulate_mouse_up(thumb, gpui_kit::MouseButton::Left, Default::default());
    let after_drag = strict_request(cx, &text, TextRevealTarget::rendered_position(0));
    strict_frames(cx, window, 3);
    assert_eq!(after_drag.status(), TextRevealStatus::Shown);
}

#[gpui_kit::test]
fn strict_reveal_approved_scope_rejects_fit_content_and_clamped(cx: &mut TestAppContext) {
    for clamp in [None, Some(1)] {
        let (window, root, text) =
            strict_open(cx, "visible target\n\nsecond", TextViewStyle::default());
        cx.update(|cx| {
            root.update(cx, |root, cx| {
                root.scrollable = false;
                root.clamp = clamp;
                cx.notify();
            })
        });
        strict_frames(cx, window, 2);
        let before = offset(cx, &text);
        let handle = strict_request(cx, &text, TextRevealTarget::rendered_position(0));
        assert_eq!(
            handle.status(),
            TextRevealStatus::Failed(TextRevealFailure::UnsupportedGeometry)
        );
        assert_eq!(offset(cx, &text), before);
        cx.update(|cx| text.update(cx, |state, cx| state.reveal_range(0..1, cx).unwrap()));
        strict_frames(cx, window, 2);
        assert_eq!(
            handle.status(),
            TextRevealStatus::Failed(TextRevealFailure::UnsupportedGeometry)
        );
    }
}
pub(super) fn strict_native_complex_text(cx: &mut TestAppContext) {
    let source = format!(
        "{}\n\n日本語 e\u{301} 👩‍💻 שלום العربية target",
        "before\n\n".repeat(25)
    );
    let (window, _, text) = strict_open(cx, &source, TextViewStyle::default());
    for needle in ["日本語", "e\u{301}", "👩‍💻", "שלום", "العربية"] {
        let at = position(cx, &text, needle);
        let handle = strict_request(cx, &text, TextRevealTarget::rendered_position(at));
        strict_frames(cx, window, 4);
        assert_eq!(
            handle.status(),
            TextRevealStatus::Shown,
            "native representative cluster/RTL position"
        );
    }
}
const MARK: u32 = 0x13579b;
fn mark(cx: &mut TestAppContext, text: &Entity<TextViewState>, at: usize) {
    cx.update(|cx| {
        text.update(cx, |state, cx| {
            state
                .set_range_highlights(
                    [gpui_kit::base::text::RangeHighlight::new(
                        at..at + 1,
                        gpui_kit::rgb(MARK),
                    )],
                    cx,
                )
                .unwrap()
        })
    });
}
fn row(cx: &mut TestAppContext, window: gpui_kit::AnyWindowHandle) -> Bounds<Pixels> {
    cx.update_window(window, |_, window, _| {
        let color: gpui_kit::Background = gpui_kit::rgb(MARK).into();
        let quad = window
            .painted_quads()
            .into_iter()
            .find(|q| q.background == color)
            .expect("actual painted target highlight");
        let scale = window.scale_factor();
        Bounds::new(
            point(
                px(quad.bounds.origin.x.as_f32() / scale),
                px(quad.bounds.origin.y.as_f32() / scale),
            ),
            size(
                px(quad.bounds.size.width.as_f32() / scale),
                px(quad.bounds.size.height.as_f32() / scale),
            ),
        )
    })
    .unwrap()
}
fn assert_row_visible(
    cx: &mut TestAppContext,
    window: gpui_kit::AnyWindowHandle,
    text: &Entity<TextViewState>,
) {
    let row = row(cx, window);
    let viewport = text.read_with(cx, |state, _| state.list_state().viewport_bounds());
    assert!(
        row.top() >= viewport.top() - px(0.5) && row.bottom() <= viewport.bottom() + px(0.5),
        "full painted row, not intersection: {row:?} / {viewport:?}"
    );
}
#[gpui_kit::test]
fn strict_reveal_partial_rows_are_minimal(cx: &mut TestAppContext) {
    strict_partial_row_cases(cx);
}

pub(super) fn strict_partial_row_cases(cx: &mut TestAppContext) {
    let source = (0..40)
        .map(|ix| format!("paragraph number {ix}\n\n"))
        .collect::<String>();
    let (window, _, text) = strict_open(cx, &source, TextViewStyle::default());
    let at = position(cx, &text, "paragraph number 20");
    mark(cx, &text, at);
    let shown = strict_request(cx, &text, TextRevealTarget::rendered_position(at));
    strict_frames(cx, window, 4);
    assert_eq!(shown.status(), TextRevealStatus::Shown);
    let initial = row(cx, window);
    let viewport = text.read_with(cx, |state, _| state.list_state().viewport_bounds());
    assert!((initial.bottom() - viewport.bottom()).abs() <= px(0.5));
    cx.update(|cx| text.read(cx).list_state().scroll_by(px(-5.)));
    strict_frames(cx, window, 1);
    let before = offset(cx, &text);
    let partial = row(cx, window);
    assert!((partial.bottom() - viewport.bottom() - px(5.)).abs() <= px(0.5));
    let handle = strict_request(cx, &text, TextRevealTarget::rendered_position(at));
    strict_frames(cx, window, 4);
    assert_eq!(handle.status(), TextRevealStatus::Shown);
    assert!((offset(cx, &text).y - before.y + px(5.)).abs() <= px(0.5));
    assert_row_visible(cx, window, &text);
    let line = row(cx, window);
    cx.update(|cx| {
        text.read(cx)
            .list_state()
            .scroll_by(line.top() - viewport.top() + px(5.))
    });
    strict_frames(cx, window, 1);
    let before = offset(cx, &text);
    let handle = strict_request(cx, &text, TextRevealTarget::rendered_position(at));
    strict_frames(cx, window, 4);
    assert_eq!(handle.status(), TextRevealStatus::Shown);
    assert!((offset(cx, &text).y - before.y - px(5.)).abs() <= px(0.5));
    assert_row_visible(cx, window, &text);
}
#[gpui_kit::test]
fn strict_reveal_failures_do_not_scroll_and_horizontal_offset_is_retained(cx: &mut TestAppContext) {
    let source = format!("```\n{}tail\n\nlast\n```", "W".repeat(200));
    let mut code_style = gpui_kit::StyleRefinement::default();
    code_style.overflow.x = Some(gpui_kit::Overflow::Scroll);
    code_style.overflow.y = Some(gpui_kit::Overflow::Scroll);
    code_style.display = Some(gpui_kit::Display::Flex);
    code_style.flex_direction = Some(gpui_kit::FlexDirection::Row);
    code_style.max_size.height = Some(px(100.).into());
    code_style.text.white_space = Some(gpui_kit::WhiteSpace::Nowrap);
    let (window, _, text) = strict_open(
        cx,
        &source,
        TextViewStyle::default().with_code_block(code_style),
    );
    let code = codes(&source);
    let before = offset(cx, &text);
    let clipped = strict_request(
        cx,
        &text,
        TextRevealTarget::code_position(code[0].0.clone(), code[0].1.find("tail").unwrap()),
    );
    strict_frames(cx, window, 3);
    assert_eq!(
        clipped.status(),
        TextRevealStatus::Failed(TextRevealFailure::HorizontallyClipped)
    );
    assert_eq!(offset(cx, &text), before);
    {
        let visual = &mut gpui_kit::VisualTestContext::from_window(window, cx);
        visual.simulate_event(gpui_kit::MouseMoveEvent {
            position: point(px(40.), px(40.)),
            pressed_button: None,
            modifiers: Default::default(),
        });
        visual.simulate_event(gpui_kit::ScrollEvent {
            position: point(px(40.), px(40.)),
            delta: gpui_kit::ScrollDelta::Pixels(point(px(-400.), px(0.))),
            touch_phase: gpui_kit::TouchPhase::Moved,
            ..Default::default()
        });
    }
    strict_frames(cx, window, 2);
    let clipped = strict_request(
        cx,
        &text,
        TextRevealTarget::code_position(code[0].0.clone(), 0),
    );
    strict_frames(cx, window, 3);
    assert_eq!(
        clipped.status(),
        TextRevealStatus::Failed(TextRevealFailure::HorizontallyClipped),
        "nonzero inner x offset must not reset to zero in measurement"
    );
    assert_eq!(offset(cx, &text), before);

    let (window, _, text) = strict_open(cx, " \n ", TextViewStyle::default());
    let noop = strict_request(cx, &text, TextRevealTarget::noop());
    assert_eq!(noop.status(), TextRevealStatus::Shown);
    strict_frames(cx, window, 2);
    let code_source = "```\n```";
    let mut code_style = gpui_kit::StyleRefinement::default();
    code_style.size.height = Some(px(300.).into());
    let (window, _, text) = strict_open(
        cx,
        code_source,
        TextViewStyle::default().with_code_block(code_style),
    );
    let before = offset(cx, &text);
    let invalid_noop = strict_request(cx, &text, TextRevealTarget::noop());
    assert_eq!(
        invalid_noop.status(),
        TextRevealStatus::Failed(TextRevealFailure::InvalidTarget)
    );
    let too_large = strict_request(
        cx,
        &text,
        TextRevealTarget::empty_code(codes(code_source)[0].0.clone()),
    );
    strict_frames(cx, window, 3);
    assert_eq!(
        too_large.status(),
        TextRevealStatus::Failed(TextRevealFailure::TargetTooLarge)
    );
    assert_eq!(offset(cx, &text), before);
}
#[gpui_kit::test]
async fn strict_reveal_waiters_drop_manual_scroll_resize_and_old_callbacks(
    cx: &mut TestAppContext,
) {
    let source = (0..30)
        .map(|ix| format!("line {ix}\n\n"))
        .collect::<String>();
    let (window, root, text) = strict_open(cx, &source, TextViewStyle::default());
    let handle = strict_request(cx, &text, TextRevealTarget::rendered_position(0));
    let clone = handle.clone();
    let one =
        cx.update(|cx| text.update(cx, |_, cx| cx.spawn(async move |_, _| clone.wait().await)));
    let clone = handle.clone();
    let two =
        cx.update(|cx| text.update(cx, |_, cx| cx.spawn(async move |_, _| clone.wait().await)));
    cx.run_until_parked();
    assert!(handle.cancel());
    let expected = TextRevealStatus::Cancelled(TextRevealCancellation::Explicit);
    assert_eq!(one.await, expected);
    assert_eq!(two.await, expected);

    let pending = strict_request(cx, &text, TextRevealTarget::rendered_position(0));
    {
        let visual = &mut gpui_kit::VisualTestContext::from_window(window, cx);
        visual.simulate_event(gpui_kit::MouseMoveEvent {
            position: point(px(40.), px(40.)),
            pressed_button: None,
            modifiers: Default::default(),
        });
        visual.simulate_event(gpui_kit::ScrollEvent {
            position: point(px(40.), px(40.)),
            delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-10.))),
            touch_phase: gpui_kit::TouchPhase::Moved,
            ..Default::default()
        });
    }
    assert_eq!(
        pending.status(),
        TextRevealStatus::Cancelled(TextRevealCancellation::UserScroll)
    );
    let at = position(cx, &text, "line 20");
    let first = strict_request(cx, &text, TextRevealTarget::rendered_position(at));
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert_eq!(
        first.status(),
        TextRevealStatus::Pending,
        "paint itself must not finalize Shown"
    );
    let second = strict_request(cx, &text, TextRevealTarget::rendered_position(at));
    assert_ne!(first.id(), second.id());
    strict_frames(cx, window, 4);
    assert_eq!(
        first.status(),
        TextRevealStatus::Cancelled(TextRevealCancellation::Superseded)
    );
    assert_eq!(second.status(), TextRevealStatus::Shown);
    cx.simulate_window_resize(window, size(px(400.), px(230.)));
    strict_frames(cx, window, 2);
    assert_eq!(
        second.status(),
        TextRevealStatus::Shown,
        "terminal is immutable after resize"
    );

    // Context updates can synchronously draw and commit. Resize before the
    // request here; the separate case below deliberately resizes after commit.
    cx.simulate_window_resize(window, size(px(410.), px(240.)));
    let before_resize = strict_request(cx, &text, TextRevealTarget::rendered_position(0));
    strict_frames(cx, window, 5);
    assert_eq!(
        before_resize.status(),
        TextRevealStatus::Shown,
        "request after resized layout remeasures"
    );
    let after_commit = strict_request(cx, &text, TextRevealTarget::rendered_position(at));
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.simulate_window_resize(window, size(px(420.), px(250.)));
    strict_frames(cx, window, 3);
    assert_eq!(
        after_commit.status(),
        TextRevealStatus::Cancelled(TextRevealCancellation::LayoutChangedAfterScroll)
    );

    cx.update_window(window, |_, window, cx| {
        text.read(cx).focus_handle().clone().focus(window, cx);
        window.press("ctrl-a", cx);
    })
    .unwrap();
    let selection = text.read_with(cx, |state, _| {
        (state.selected_text(), state.selected_source_range())
    });
    let request = strict_request(cx, &text, TextRevealTarget::rendered_position(at));
    strict_frames(cx, window, 4);
    assert_eq!(request.status(), TextRevealStatus::Shown);
    assert_eq!(
        text.read_with(cx, |state, _| (
            state.selected_text(),
            state.selected_source_range()
        )),
        selection,
        "measurement preserves live selection/source identity"
    );

    let orphan =
        cx.update(|cx| cx.new(|cx| TextViewState::markdown("orphan", cx).scrollable(true)));
    cx.run_until_parked();
    let dropped = strict_request(cx, &orphan, TextRevealTarget::rendered_position(0));
    drop(orphan);
    cx.update(|_| ());
    cx.run_until_parked();
    assert_eq!(
        dropped.wait().await,
        TextRevealStatus::Cancelled(TextRevealCancellation::ViewDropped)
    );
    drop(root);
}
