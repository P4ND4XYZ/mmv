use super::{SourcePreview, SourceSnapshot, Target};
use crate::{app, ui::theme::Palette};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _,
    Render, Styled as _, TestAppContext, Window, WindowBounds, WindowOptions, div, point, px, size,
    test::TestWindowExt as _,
};
use std::{path::PathBuf, sync::Arc};

fn snapshot(source: String) -> Arc<SourceSnapshot> {
    let mut app = app::App::default();
    let path = PathBuf::from(r"C:\fixtures\source.md");
    let request = app.open(path.clone());
    app.complete(app::ReadResult {
        request,
        content: Ok(app::LoadedDocument {
            key: app::DocumentKey(path),
            source,
        }),
    });
    let document = app.document().unwrap();
    let snapshot = Arc::new(SourceSnapshot::new(
        document.source.clone(),
        document.generation,
    ));
    assert!(Arc::ptr_eq(&document.source, &snapshot.source));
    snapshot
}

#[test]
fn shared_source_indexes_utf8_crlf_without_copying() {
    let snapshot = snapshot("日本語\r\nnext\n".into());
    assert_eq!(snapshot.lines, vec![0..9, 11..15, 16..16]);
}

struct Harness {
    preview: Entity<SourcePreview>,
    font_size: f32,
}
impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .font_family("Arial")
            .text_size(px(self.font_size))
            .child(self.preview.clone())
    }
}
fn open(
    cx: &mut TestAppContext,
    source: String,
    target: std::ops::Range<usize>,
) -> (AnyWindowHandle, Entity<Harness>, Entity<SourcePreview>) {
    let snapshot = snapshot(source);
    let generation = snapshot.generation;
    let mut preview_out = None;
    let (window, harness) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(260.), px(180.)),
                ))),
                ..Default::default()
            },
            cx,
            |_, cx| {
                let palette =
                    Palette::resolve(&Default::default(), gpui_kit::component::Theme::global(cx));
                let preview = cx.new(|_| SourcePreview::new(snapshot, palette));
                preview.update(cx, |preview, cx| {
                    preview.reveal_range(target, generation, cx)
                });
                preview_out = Some(preview.clone());
                cx.new(|_| Harness {
                    preview,
                    font_size: 16.,
                })
            },
        )
        .unwrap()
    });
    frame(cx, window);
    (window, harness, preview_out.unwrap())
}
fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn actual_preview_centers_clamps_scrolls_full_source_and_preserves_anchor_on_reflow(
    cx: &mut TestAppContext,
) {
    let source = (0..100)
        .map(|i| format!("line {i} proportional WWW iii 日本語 e\u{301}\tend\r\n"))
        .collect::<String>();
    let start = source.find("line 50").unwrap();
    let (window, harness, preview) = open(cx, source.clone(), start..start + 7);
    preview.read_with(cx, |p, _| {
        assert!(p.viewport_height > 0.);
        let target_y = (p.row_for_source(start) as f32 + 0.5) * p.row_height - p.scroll;
        assert!((target_y - p.viewport_height * 0.5).abs() < 0.1);
    });
    cx.update_window(window, |_, window, cx| {
        window.scroll(
            "source-preview",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-90.))),
            cx,
        )
    })
    .unwrap();
    frame(cx, window);
    let anchor = preview.read_with(cx, |p, _| p.top_source());
    cx.simulate_window_resize(window, size(px(175.), px(180.)));
    frame(cx, window);
    preview.read_with(cx, |p, _| {
        let row = &p.rows[(p.scroll / p.row_height).floor() as usize];
        assert!(
            row.start <= anchor && anchor <= row.end,
            "resize keeps old top source in new top row"
        );
        assert!(matches!(p.target, Target::Range(_)));
    });
    let anchor = preview.read_with(cx, |p, _| p.top_source());
    cx.update(|cx| {
        harness.update(cx, |h, cx| {
            h.font_size = 21.;
            cx.notify();
        })
    });
    frame(cx, window);
    preview.read_with(cx, |p, _| {
        let row = &p.rows[(p.scroll / p.row_height).floor() as usize];
        assert!(
            row.start <= anchor && anchor <= row.end,
            "font change preserves source anchor"
        );
    });
    cx.update(|cx| preview.update(cx, |p, cx| p.scroll_by(f32::MAX, cx)));
    frame(cx, window);
    preview.read_with(cx, |p, _| assert_eq!(p.scroll, p.max_scroll()));
    cx.update(|cx| preview.update(cx, |p, cx| p.scroll_by(-f32::MAX, cx)));
    frame(cx, window);
    preview.read_with(cx, |p, _| assert_eq!(p.scroll, 0.));
    cx.update(|cx| {
        preview.update(cx, |p, cx| {
            let g = p.snapshot.generation;
            p.reveal_range(0..4, g, cx);
        })
    });
    frame(cx, window);
    preview.read_with(cx, |p, _| {
        assert_eq!(p.scroll, 0., "no leading centering padding")
    });
    cx.update(|cx| {
        preview.update(cx, |p, cx| {
            let g = p.snapshot.generation;
            let n = p.snapshot.source.len();
            p.reveal_range(n - 5..n - 2, g, cx);
        })
    });
    frame(cx, window);
    preview.read_with(cx, |p, _| {
        assert_eq!(p.scroll, p.max_scroll(), "no trailing centering padding")
    });
}

#[gpui_kit::test]
fn actual_long_line_wrap_covers_every_byte_and_shapes_only_visible_rows(cx: &mut TestAppContext) {
    let source = "Wi日本語e\u{301}👩‍💻\t".repeat(5000);
    let total = source.len();
    let started = std::time::Instant::now();
    let (window, _, preview) = open(cx, source, 0..2);
    preview.read_with(cx, |p, _| {
        assert_eq!(p.rows.first().unwrap().start, 0);
        assert_eq!(p.rows.last().unwrap().end, total);
        assert!(p.rows.len() > 1000);
        assert!(p.rows.windows(2).all(|r| r[0].end == r[1].start));
        assert!(
            p.rows
                .iter()
                .all(|r| p.snapshot.source.is_char_boundary(r.start)
                    && p.snapshot.source.is_char_boundary(r.end))
        );
    });
    cx.update_window(window, |_, window, cx| {
        preview.update(cx, |p, _| {
            let bounds = Bounds::new(point(px(0.), px(0.)), size(px(260.), px(180.)));
            let lines = p.visible_lines(bounds, window);
            assert!(lines.len() <= 10, "shape only visible rows plus one");
            for (line, _) in lines {
                assert!(
                    line.width() <= px(260.1),
                    "actual glyphs must fit: {:?}",
                    line.width()
                );
            }
        })
    })
    .unwrap();
    eprintln!(
        "source_preview_wrap_bytes={total} first_frame_ms={:.3}",
        started.elapsed().as_secs_f64() * 1000.
    );
}

#[gpui_kit::test]
fn links_events_preserve_manual_scroll_for_same_id_and_isolate_half_page_and_ime(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{EntityInputHandler as _, Focusable as _};
    let source =
        "[First](one.md)\n\n".to_owned() + &"body context\n\n".repeat(80) + "[Second](two.md)";
    let (window, viewer) = crate::ui::tests::open(cx, source);
    frame(cx, window);
    let press = |cx: &mut TestAppContext, key: &str| {
        cx.update_window(window, |_, window, cx| window.press(key, cx))
            .unwrap();
        frame(cx, window);
    };
    press(cx, "f");
    let (panel, preview, source) = viewer.read_with(cx, |v, cx| {
        let selection = v.link_panel.as_ref().unwrap();
        assert!(Arc::ptr_eq(
            &v.app.document().unwrap().source,
            &selection.preview.read(cx).snapshot.source
        ));
        (
            selection.panel.clone(),
            selection.preview.clone(),
            selection.preview.read(cx).snapshot.clone(),
        )
    });
    press(cx, "ctrl-d");
    let manual = preview.read_with(cx, |p, _| {
        assert!((p.scroll - p.viewport_height * 0.5).abs() < 0.1);
        p.scroll
    });
    for operation in [app::ExternalOperation::Bottom, app::ExternalOperation::Top] {
        cx.update(|cx| {
            viewer.update(cx, |v, cx| {
                v.request_external(
                    operation,
                    app::ExternalRequestId("preview-isolation".into()),
                    cx,
                );
            })
        });
        frame(cx, window);
        preview.read_with(cx, |p, _| {
            assert_eq!(p.scroll, manual, "external body movements preserve preview")
        });
    }
    cx.update_window(window, |_, window, cx| {
        let query = panel.read(cx).input.clone();
        query.update(cx, |input, cx| {
            input.replace_text_in_range(None, "first", window, cx)
        });
    })
    .unwrap();
    frame(cx, window);
    preview.read_with(cx, |p, _| {
        assert_eq!(p.scroll, manual, "same ID must not reveal again")
    });
    cx.update_window(window, |_, window, cx| {
        let query = panel.read(cx).input.clone();
        query.update(cx, |input, cx| {
            input.replace_and_mark_text_in_range(
                Some(0..input.value().encode_utf16().count()),
                "first",
                Some(5..5),
                window,
                cx,
            )
        });
    })
    .unwrap();
    frame(cx, window);
    press(cx, "ctrl-d");
    preview.read_with(cx, |p, _| assert_eq!(p.scroll, manual, "IME has priority"));
    press(cx, "escape");
    assert!(viewer.read_with(cx, |v, _| v.link_panel.is_some()));
    cx.update_window(window, |_, window, cx| {
        let query = panel.read(cx).input.clone();
        query.update(cx, |input, cx| {
            input.replace_text_in_range(
                Some(0..input.value().encode_utf16().count()),
                "no match",
                window,
                cx,
            )
        });
    })
    .unwrap();
    frame(cx, window);
    preview.read_with(cx, |p, _| {
        assert_eq!(p.target, Target::Empty);
        assert_eq!(p.scroll, 0.);
    });
    cx.update_window(window, |_, window, cx| {
        let query = panel.read(cx).input.clone();
        query.update(cx, |input, cx| {
            input.replace_text_in_range(
                Some(0..input.value().encode_utf16().count()),
                "",
                window,
                cx,
            )
        });
        assert!(query.focus_handle(cx).is_focused(window));
    })
    .unwrap();
    frame(cx, window);
    preview.read_with(cx, |p, _| {
        assert_eq!(p.scroll, 0., "revival reveals first location")
    });
    cx.update(|cx| {
        viewer.update(cx, |v, _| {
            v.document.analysis.as_mut().unwrap().candidates[1].source_range = None
        })
    });
    press(cx, "down");
    preview.read_with(cx, |p, _| assert_eq!(p.target, Target::Unavailable));
    let reads = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let captured = reads.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&viewer, move |_, request: &app::ReadRequest, _| {
            captured.borrow_mut().push(request.clone())
        })
    });
    press(cx, "enter");
    assert!(viewer.read_with(cx, |v, _| v.link_panel.is_none()));
    let request = reads
        .borrow_mut()
        .pop()
        .expect("unavailable location must not prevent confirmation");
    assert_eq!(request.path, PathBuf::from(r"C:\fixtures\two.md"));
    cx.update(|cx| {
        viewer.update(cx, |v, cx| {
            v.complete(
                app::ReadResult {
                    request,
                    content: Err("missing".into()),
                },
                cx,
            )
        })
    });
    frame(cx, window);
    press(cx, "f");
    viewer.read_with(cx, |v, cx| {
        let s = v.link_panel.as_ref().unwrap();
        assert_ne!(s.panel, panel);
        assert!(Arc::ptr_eq(&source, &s.preview.read(cx).snapshot));
        assert_eq!(s.preview.read(cx).scroll, 0.);
    });
}

#[gpui_kit::test]
fn stale_document_generation_rejects_all_old_panel_events_and_ranges(cx: &mut TestAppContext) {
    use crate::ui::selection_panel::Event;
    let (window, viewer) = crate::ui::tests::open(cx, "[Old](old.md)".into());
    frame(cx, window);
    cx.update_window(window, |_, window, cx| window.press("f", cx))
        .unwrap();
    frame(cx, window);
    let (panel, preview) = viewer.read_with(cx, |v, _| {
        let s = v.link_panel.as_ref().unwrap();
        (s.panel.clone(), s.preview.clone())
    });
    let generation = cx.update(|cx| {
        viewer.update(cx, |v, _| {
            let path = PathBuf::from(r"C:\fixtures\new.md");
            let request = v.app.open(path.clone());
            v.app.complete(app::ReadResult {
                request,
                content: Ok(app::LoadedDocument {
                    key: app::DocumentKey(path),
                    source: "[New](new.md)".into(),
                }),
            });
            v.app.document().unwrap().generation
        })
    });
    cx.update(|cx| {
        preview.update(cx, |p, cx| p.reveal_range(1..3, generation, cx));
        panel.update(cx, |_, cx| {
            cx.emit(Event::Query("old".into()));
            cx.emit(Event::SelectionChanged(None));
            cx.emit(Event::PreviewScrollRequested(true));
            cx.emit(Event::Confirm(0));
            cx.emit(Event::Cancel);
        });
    });
    frame(cx, window);
    viewer.read_with(cx, |v, _| {
        assert_eq!(
            v.link_panel.as_ref().unwrap().panel,
            panel,
            "stale cancel/confirm must not mutate lifetime"
        )
    });
    preview.read_with(cx, |p, _| assert_eq!(p.target, Target::Range(0..13)));
}

#[gpui_kit::test]
fn none_unavailable_clear_without_reusing_manual_position(cx: &mut TestAppContext) {
    let source = "link\n".to_owned() + &"body\n".repeat(100);
    let (window, _, preview) = open(cx, source, 0..4);
    cx.update(|cx| preview.update(cx, |p, cx| p.scroll_half_page(true, cx)));
    let old = preview.read_with(cx, |p, _| p.scroll);
    assert!(old > 0.);
    cx.update(|cx| preview.update(cx, |p, cx| p.clear_target(cx)));
    frame(cx, window);
    preview.read_with(cx, |p, _| {
        assert_eq!(p.target, Target::Empty);
        assert_eq!(p.scroll, 0.);
    });
    cx.update(|cx| {
        preview.update(cx, |p, cx| {
            let g = p.snapshot.generation;
            p.reveal_range(0..4, g, cx);
        })
    });
    frame(cx, window);
    preview.read_with(cx, |p, _| assert_eq!(p.scroll, 0.));
    cx.update(|cx| preview.update(cx, |p, cx| p.show_unavailable(cx)));
    frame(cx, window);
    preview.read_with(cx, |p, _| {
        assert_eq!(p.target, Target::Unavailable);
        assert_eq!(p.scroll, 0.);
    });
}
