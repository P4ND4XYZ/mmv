mod modes;
mod phase_one;

use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Instant};

use gpui_kit::{
    AppContext as _, Bounds, TestAppContext, WindowBounds, WindowOptions, point, px, size,
    test::TestWindowExt as _,
};

use super::Viewer;
use crate::app::{DiagramRequest, DiagramResult, DiagramStatus, ReadRequest, ReadResult};

fn open(
    cx: &mut TestAppContext,
    source: String,
) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<Viewer>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::bind_keys(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(640.), px(360.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| {
                    Viewer::new(
                        PathBuf::from(r"C:\fixtures\sample.md"),
                        Instant::now(),
                        window,
                        cx,
                    )
                });
                view.update(cx, |view, cx| {
                    let request = view.app.open(view.path.clone());
                    view.complete(
                        ReadResult {
                            request,
                            content: Ok(source),
                        },
                        cx,
                    );
                });
                view
            },
        )
        .unwrap()
    })
}

#[gpui_kit::test]
fn clicking_local_links_replaces_the_document_and_reload_uses_the_new_path(
    cx: &mut TestAppContext,
) {
    let (window, view) = open(cx, "[Open file](notes/日本語%20memo.md)".into());
    cx.run_until_parked();
    let requests = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = requests.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone());
        })
    });
    let original = view.read_with(cx, |viewer, _| viewer.path.clone());
    let target = original.parent().unwrap().join("notes/日本語 memo.md");
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    let body = visual.debug_bounds("document-body").unwrap();
    // The single link is the first paragraph, inside the TextView's padding.
    let position = body.origin + point(px(20.), px(20.));
    visual.simulate_mouse_down(position, gpui_kit::MouseButton::Right, Default::default());
    visual.simulate_mouse_up(position, gpui_kit::MouseButton::Right, Default::default());
    assert!(requests.borrow().is_empty());
    visual.simulate_click(position, Default::default());
    let request = requests
        .borrow_mut()
        .pop()
        .expect("link emitted a read request");
    assert_eq!(request.path, target);
    view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.path, original);
        assert!(viewer.app.is_loading());
    });
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.complete(
                ReadResult {
                    request,
                    content: Ok("切り替え後の本文".into()),
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.path, target);
        assert_eq!(viewer.app.document().unwrap().path, target);
        assert!(
            viewer
                .document
                .text
                .read(cx)
                .rendered_text()
                .as_str()
                .contains("切り替え後の本文")
        );
    });
    cx.update_window(window, |_, window, cx| window.press("f5", cx))
        .unwrap();
    assert_eq!(requests.borrow_mut().pop().unwrap().path, target);
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |viewer, cx| viewer.open_link("../next.md", window, cx));
    })
    .unwrap();
    assert_eq!(
        requests.borrow_mut().pop().unwrap().path,
        original.with_file_name("next.md")
    );
}

#[gpui_kit::test]
fn failed_local_link_load_keeps_the_document_and_reload_target(cx: &mut TestAppContext) {
    let (window, view) = open(cx, "元の本文".into());
    cx.run_until_parked();
    let requests = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = requests.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone());
        })
    });
    let original = view.read_with(cx, |viewer, _| viewer.path.clone());
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |viewer, cx| viewer.open_link("missing.md", window, cx));
    })
    .unwrap();
    let request = requests.borrow_mut().pop().unwrap();
    assert_eq!(request.path, original.with_file_name("missing.md"));
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.complete(
                ReadResult {
                    request,
                    content: Err("file not found".into()),
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.path, original);
        assert_eq!(viewer.app.document().unwrap().path, original);
        assert_eq!(viewer.app.error(), Some("file not found"));
        assert!(
            viewer
                .document
                .text
                .read(cx)
                .rendered_text()
                .as_str()
                .contains("元の本文")
        );
    });
    cx.update_window(window, |_, window, cx| window.press("f5", cx))
        .unwrap();
    assert_eq!(requests.borrow_mut().pop().unwrap().path, original);
}

fn running_diagram(viewer: &Viewer) -> DiagramRequest {
    let document = viewer.app.document().unwrap();
    let (&start, diagram) = viewer
        .app
        .diagrams()
        .iter()
        .find(|(_, diagram)| diagram.status == DiagramStatus::Running)
        .unwrap();
    DiagramRequest {
        generation: document.generation,
        path: document.path.clone(),
        start,
        source: diagram.source.clone(),
    }
}

#[gpui_kit::test]
fn mermaid_parsing_hides_sources_and_keeps_search_working_during_generation(
    cx: &mut TestAppContext,
) {
    let source = "# 本文\n\n```mermaid\nflowchart TD\nA[秘密の図ラベル] --> B\n```\n\n検証本文\n\n```mermaid\nflowchart TD\nA[秘密の図ラベル] --> B\n```\n\n```rust\nlet normal_code = 1;\n```";
    let (window, view) = open(cx, source.into());
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let viewer = view.read(cx);
        let rendered = viewer.document.text.read(cx).rendered_text();
        assert!(rendered.as_str().contains("検証本文"));
        assert!(rendered.as_str().contains("normal_code"));
        assert!(!rendered.as_str().contains("秘密の図ラベル"));
        assert_eq!(viewer.app.diagrams().len(), 2);
        assert_eq!(
            running_diagram(viewer).start,
            source.find("```mermaid").unwrap()
        );
        let sources = viewer
            .app
            .diagrams()
            .values()
            .map(|diagram| &diagram.source)
            .collect::<Vec<_>>();
        assert_eq!(sources[0], sources[1]);
        window.press("/", cx);
        window.input("検証本文", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, _, cx| {
        view.update(cx, |viewer, cx| {
            assert_eq!(viewer.app.matches().len(), 1);
            viewer.complete_diagram(
                DiagramResult {
                    request: running_diagram(viewer),
                    png: Err("test generation failure".into()),
                },
                cx,
            );
            assert!(viewer.document.diagram_images.is_empty());
            assert_eq!(viewer.app.matches().len(), 1);
            assert_eq!(
                viewer
                    .app
                    .diagrams()
                    .values()
                    .filter(|diagram| diagram.status == DiagramStatus::Failed)
                    .count(),
                1
            );
            assert_eq!(
                viewer
                    .app
                    .diagrams()
                    .values()
                    .filter(|diagram| diagram.status == DiagramStatus::Running)
                    .count(),
                1
            );
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn background_parse_discovers_offscreen_diagrams(cx: &mut TestAppContext) {
    let source =
        "長い本文です。\n\n".repeat(500) + "```mermaid\nflowchart TD\nA[画面外の図] --> B\n```";
    let (window, view) = open(cx, source);
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.app.diagrams().len(), 1);
        assert_eq!(
            running_diagram(viewer).source,
            "flowchart TD\nA[画面外の図] --> B"
        );
        assert!(
            !viewer
                .document
                .text
                .read(cx)
                .rendered_text()
                .as_str()
                .contains("画面外の図")
        );
    });
}

#[gpui_kit::test]
#[ignore = "requires Node.js, mermaid-cli and its browser; run explicitly"]
fn real_mermaid_fixture_generates_and_decodes_in_gpui(cx: &mut TestAppContext) {
    let (window, view) = open(
        cx,
        include_str!("../../docs/design/prototypes/gpui-evaluation/fixtures/mermaid.md").into(),
    );
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    for _ in 0..3 {
        let request = view.read_with(cx, |viewer, _| running_diagram(viewer));
        let result = crate::infra::generate_diagram(request);
        assert!(result.png.is_ok(), "{:?}", result.png.as_ref().err());
        cx.update(|cx| view.update(cx, |viewer, cx| viewer.complete_diagram(result, cx)));
        cx.run_until_parked();
    }
    cx.update(|cx| {
        let images = view.read(cx).document.diagram_images.clone();
        assert_eq!(images.len(), 3);
        for image in images.into_values() {
            let decoded = image
                .to_image_data(cx.svg_renderer())
                .expect("GPUI decoded real PNG");
            assert!(decoded.size(0).width.0 > 0);
            assert!(decoded.size(0).height.0 > 0);
        }
        assert!(
            view.read(cx)
                .app
                .diagrams()
                .values()
                .all(|diagram| diagram.status == DiagramStatus::Ready)
        );
    });
}

#[gpui_kit::test]
fn late_diagram_display_has_real_layout_and_preserves_search_ranges(cx: &mut TestAppContext) {
    // A 20x10 transparent PNG generated by System.Drawing for this test.
    let png = vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 20, 0, 0, 0, 10, 8,
        6, 0, 0, 0, 180, 85, 126, 230, 0, 0, 0, 1, 115, 82, 71, 66, 0, 174, 206, 28, 233, 0, 0, 0,
        4, 103, 65, 77, 65, 0, 0, 177, 143, 11, 252, 97, 5, 0, 0, 0, 9, 112, 72, 89, 115, 0, 0, 14,
        195, 0, 0, 14, 195, 1, 199, 111, 168, 100, 0, 0, 0, 16, 73, 68, 65, 84, 56, 79, 99, 96, 24,
        5, 163, 96, 20, 16, 6, 0, 3, 42, 0, 1, 148, 87, 35, 248, 0, 0, 0, 0, 73, 69, 78, 68, 174,
        66, 96, 130,
    ];
    let (window, view) = open(
        cx,
        "```mermaid\nflowchart TD\nA --> B\n```\n\n検証本文".into(),
    );
    cx.run_until_parked();
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    assert_eq!(
        visual.debug_bounds("mermaid-0").unwrap().size.height,
        px(0.)
    );
    cx.update_window(window, |_, window, cx| {
        window.press("/", cx);
        window.input("検証本文", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let ranges = view.read_with(cx, |viewer, _| viewer.app.matches().to_vec());
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.complete_diagram(
                DiagramResult {
                    request: running_diagram(viewer),
                    png: Ok(png),
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let bounds = visual.debug_bounds("mermaid-0").unwrap();
    assert_eq!(bounds.size.width, px(10.));
    assert_eq!(bounds.size.height, px(5.));
    view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.app.matches(), ranges);
        assert_eq!(viewer.document.diagram_images.len(), 1);
        assert!(viewer.display_error.is_none());
    });
}

#[gpui_kit::test]
fn search_input_and_navigation_use_rendered_japanese_text(cx: &mut TestAppContext) {
    let (window, view) = open(
        cx,
        "# 見出し\n\n**日本語** と [日本語](https://example.com)".into(),
    );
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.press("/", cx);
        window.input("日本語", cx);
        assert_eq!(
            view.read(cx).input.state.read(cx).value().as_ref(),
            "日本語"
        );
    })
    .unwrap();
    // InputEvent notifications are delivered when the app update finishes.
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        let viewer = view.read(cx);
        let rendered = viewer.document.text.read(cx).rendered_text();
        assert!(!rendered.as_str().contains("**"));
        assert!(!rendered.as_str().contains("https://"));
        assert_eq!(viewer.app.matches().len(), 2);
        for range in viewer.app.matches() {
            assert_eq!(&rendered.as_str()[range.clone()], "日本語");
        }
        assert!(viewer.display_error.is_none());
        // 'n' in the input must be text, not the body's NextMatch action.
        window.press("n", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        assert_eq!(
            view.read(cx).input.state.read(cx).value().as_ref(),
            "日本語n"
        );
        assert!(view.read(cx).app.matches().is_empty());
        window.press("backspace", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        window.press("enter", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        window.press("n", cx);
        assert_eq!(view.read(cx).app.current_match(), Some(1));
        window.press("shift-n", cx);
        assert_eq!(view.read(cx).app.current_match(), Some(0));
        assert!(view.read(cx).display_error.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn revealing_an_offscreen_match_scrolls_the_real_text_view(cx: &mut TestAppContext) {
    let source = (0..80)
        .map(|ix| format!("段落 {ix} の本文です。\n\n"))
        .collect::<String>()
        + "末尾の一致";
    let (window, view) = open(cx, source);
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.press("/", cx);
        window.input("末尾の一致", cx);
    })
    .unwrap();
    cx.update_window(window, |_, window, cx| {
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let viewer = view.read(cx);
        assert_eq!(viewer.app.matches().len(), 1);
        assert!(viewer.display_error.is_none());
        assert!(
            viewer
                .document
                .text
                .read(cx)
                .list_state()
                .logical_scroll_top()
                .item_ix
                > 0
        );
    })
    .unwrap();
}
