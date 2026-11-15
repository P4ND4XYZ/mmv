use super::{Align, Motion, Palette, RawSource, Source};
use crate::lua::{
    LuaRuntime,
    document::{DocumentHandle, SourceHandle},
};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _,
    Render, Styled as _, TestAppContext, Window, WindowBounds, WindowOptions, div, point, px, size,
};

struct Harness {
    source: Entity<RawSource>,
    width: f32,
    font: f32,
}
impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(self.width))
            .h(px(150.))
            .text_size(px(self.font))
            .child(self.source.clone())
    }
}
fn model(text: String) -> Source {
    let r = LuaRuntime::from_config(Default::default());
    r.context().documents.borrow_mut().current = Some(DocumentHandle::new(
        "C:/fixtures/raw.md".into(),
        std::sync::Arc::new(text),
        |_, _| Ok(Vec::new()),
    ));
    let d = r
        .lua()
        .load("return require('mmv').document.current():source()")
        .eval::<mlua::AnyUserData>()
        .unwrap();
    let source = d.borrow::<SourceHandle>().unwrap().clone();
    Source {
        source,
        highlight: None,
        motions: Default::default(),
    }
}
fn frame(cx: &mut TestAppContext, w: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(w, |_, w, cx| w.render_frame(cx)).unwrap();
    cx.run_until_parked();
}
#[gpui_kit::test]
fn raw_source_alignment_one_shot_utf8_reflow_anchor_and_full_document(cx: &mut TestAppContext) {
    let text = format!(
        "{}\n{}\n{}",
        "日本語 abcde ".repeat(40),
        "line 日本語\r\n".repeat(100),
        "tail ".repeat(50)
    );
    let model = model(text);
    let bytes = model.source.text();
    let retained = model.source.text();
    assert!(std::sync::Arc::ptr_eq(&bytes, &retained));
    let mut source = None;
    let (w, h) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(400.), px(220.)),
                ))),
                ..Default::default()
            },
            cx,
            |_, cx| {
                let palette =
                    Palette::resolve(&Default::default(), gpui_kit::component::Theme::global(cx));
                let entity = cx.new(|_| RawSource::new(model, palette));
                source = Some(entity.clone());
                cx.new(|_| Harness {
                    source: entity,
                    width: 260.,
                    font: 16.,
                })
            },
        )
        .unwrap()
    });
    let source = source.unwrap();
    frame(cx, w);
    let range = bytes.find("line").unwrap() + "line 日本語\r\n".len() * 50;
    source.update(cx, |s, cx| {
        s.model
            .motions
            .borrow_mut()
            .push(Motion::Range(range..range + 4, Align::Start));
        cx.notify();
    });
    frame(cx, w);
    source.read_with(cx, |s, _| {
        assert_eq!(s.top_source(), range);
        assert!(s.model.motions.borrow().is_empty());
        assert!(
            s.rows
                .iter()
                .all(|r| bytes.is_char_boundary(r.start) && bytes.is_char_boundary(r.end))
        );
    });
    let start = source.read_with(cx, |s, _| s.scroll);
    source.update(cx, |s, cx| {
        s.model.motions.borrow_mut().push(Motion::Pages(0.5));
        cx.notify();
    });
    frame(cx, w);
    let manual = source.read_with(cx, |s, _| {
        assert!((s.scroll - start - s.viewport * 0.5).abs() < 0.1);
        s.scroll
    });
    frame(cx, w);
    source.read_with(cx, |s, _| assert_eq!(s.scroll, manual));
    let anchor = source.read_with(cx, |s, _| s.top_source());
    h.update(cx, |h, cx| {
        h.width = 130.;
        h.font = 22.;
        cx.notify();
    });
    source.update(cx, |_, cx| cx.notify());
    frame(cx, w);
    source.read_with(cx, |s, _| {
        assert_eq!(s.top_source(), anchor);
        assert_ne!(s.scroll, manual);
    });
    source.update(cx, |s, cx| {
        s.model.highlight = Some(0..9);
        cx.notify();
    });
    frame(cx, w);
    source.read_with(cx, |s, _| assert_eq!(s.top_source(), anchor));
    for align in [Align::Start, Align::Center, Align::End, Align::Nearest] {
        source.update(cx, |s, cx| {
            s.model
                .motions
                .borrow_mut()
                .push(Motion::Range(range..range + 4, align));
            cx.notify();
        });
        frame(cx, w);
        source.read_with(cx, |s, _| {
            let top = s.row(range) as f32 * s.row_height;
            let bottom = top + s.row_height;
            match align {
                Align::Start => assert!((s.scroll - top).abs() < 0.1),
                Align::Center => {
                    assert!((s.scroll - (top + bottom - s.viewport) * 0.5).abs() < 0.1)
                }
                Align::End => assert!((s.scroll - (bottom - s.viewport)).abs() < 0.1),
                Align::Nearest => assert!(s.scroll <= top && s.scroll + s.viewport >= bottom),
            }
        });
    }
    source.update(cx, |s, cx| {
        s.model
            .motions
            .borrow_mut()
            .push(Motion::Range(0..bytes.len(), Align::Nearest));
        cx.notify();
    });
    frame(cx, w);
    source.read_with(cx, |s, _| assert_eq!(s.scroll, 0.));
    source.update(cx, |s, cx| {
        s.model.motions.borrow_mut().push(Motion::Pages(f64::MAX));
        cx.notify();
    });
    frame(cx, w);
    source.read_with(cx, |s, _| {
        assert_eq!(s.scroll, s.max());
        assert_eq!(s.rows.last().unwrap().end, bytes.len());
        assert!(s.top_source() > range);
    });
}

fn open_raw(cx: &mut TestAppContext, model: Source) -> (AnyWindowHandle, Entity<RawSource>) {
    let mut source = None;
    let (window, _) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            let palette =
                Palette::resolve(&Default::default(), gpui_kit::component::Theme::global(cx));
            let entity = cx.new(|_| RawSource::new(model, palette));
            source = Some(entity.clone());
            cx.new(|_| Harness {
                source: entity,
                width: 260.,
                font: 16.,
            })
        })
        .unwrap()
    });
    frame(cx, window);
    (window, source.unwrap())
}

#[gpui_kit::test]
fn migrated_preview_long_line_preserves_every_utf8_byte_and_shapes_visible_rows(
    cx: &mut TestAppContext,
) {
    let text = "Wi日本語e\u{301}👩‍💻\t".repeat(5000);
    let total = text.len();
    let (window, source) = open_raw(cx, model(text));
    source.read_with(cx, |s, _| {
        let bytes = s.model.source.text();
        assert_eq!(s.rows.first().unwrap().start, 0);
        assert_eq!(s.rows.last().unwrap().end, total);
        assert!(s.rows.len() > 1000);
        assert!(s.rows.windows(2).all(|r| r[0].end == r[1].start));
        assert!(
            s.rows
                .iter()
                .all(|r| bytes.is_char_boundary(r.start) && bytes.is_char_boundary(r.end))
        );
    });
    cx.update_window(window, |_, w, cx| {
        source.update(cx, |s, _| {
            let lines = s.visible(
                Bounds::new(point(px(0.), px(0.)), size(px(260.), px(150.))),
                w,
            );
            assert!(lines.len() <= 8, "shape only visible rows plus one");
            assert!(lines.iter().all(|(line, _)| line.width() <= px(260.1)));
        })
    })
    .unwrap();
}

#[gpui_kit::test]
fn migrated_preview_link_background_is_painted_and_cleared(cx: &mut TestAppContext) {
    let text = "before [Link](target.md) after";
    let mut m = model(text.into());
    let start = text.find("[Link]").unwrap();
    m.highlight = Some(start..start + "[Link](target.md)".len());
    let (window, source) = open_raw(cx, m);
    let background = source.read_with(cx, |s, _| {
        gpui_kit::Background::from(
            s.palette
                .get(crate::config::Group::PanelSelected)
                .bg
                .opacity(0.35),
        )
    });
    cx.update_window(window, |_, w, _| {
        let quads = w
            .painted_quads()
            .into_iter()
            .filter(|q| q.background == background)
            .collect::<Vec<_>>();
        assert_eq!(quads.len(), 1);
        assert!(quads[0].bounds.origin.x.0 > 0.);
        assert!(quads[0].bounds.size.width.0 > 0.);
        assert!(quads[0].bounds.right().0 < 260. * w.scale_factor());
    })
    .unwrap();
    source.update(cx, |s, cx| {
        s.model.highlight = None;
        cx.notify();
    });
    frame(cx, window);
    cx.update_window(window, |_, w, _| {
        assert!(w.painted_quads().iter().all(|q| q.background != background))
    })
    .unwrap();
}

#[test]
fn migrated_preview_source_indexes_utf8_crlf_without_copying() {
    let m = model("日本語\r\nnext\n".into());
    let source = m.source.text();
    assert!(std::sync::Arc::ptr_eq(&source, &m.source.text()));
    let snapshot = RawSource::new(
        m,
        Palette::resolve(&Default::default(), &gpui_kit::component::Theme::default()),
    );
    assert_eq!(snapshot.lines, vec![0..9, 11..15, 16..16]);
}
