use super::*;
use crate::config::Position;
use gpui_kit::{Modifiers, MouseButton, ScrollDelta, ScrollEvent, TouchPhase};

// A 20x10 transparent PNG, also used by the existing Mermaid layout test.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 20, 0, 0, 0, 10, 8, 6,
    0, 0, 0, 180, 85, 126, 230, 0, 0, 0, 1, 115, 82, 71, 66, 0, 174, 206, 28, 233, 0, 0, 0, 4, 103,
    65, 77, 65, 0, 0, 177, 143, 11, 252, 97, 5, 0, 0, 0, 9, 112, 72, 89, 115, 0, 0, 14, 195, 0, 0,
    14, 195, 1, 199, 111, 168, 100, 0, 0, 0, 16, 73, 68, 65, 84, 56, 79, 99, 96, 24, 5, 163, 96,
    20, 16, 6, 0, 3, 42, 0, 1, 148, 87, 35, 248, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

#[gpui_kit::test]
fn configured_body_width_is_centered_without_changing_footer_width(cx: &mut TestAppContext) {
    let (window, view) = open(cx, "short paragraph".into());
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.config.max_width = 320.;
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    let (viewport, first) = view.read_with(cx, |viewer, cx| {
        let list = viewer.document.text.read(cx).list_state();
        (list.viewport_bounds(), list.bounds_for_item(0).unwrap())
    });
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    let body = visual.debug_bounds("document-body").unwrap();
    let status = visual.debug_bounds("status-line").unwrap();
    let command = visual.debug_bounds("command-line").unwrap();
    assert_eq!(viewport.size.width, px(320.));
    assert_eq!(
        viewport.origin.x,
        body.origin.x + (body.size.width - px(320.)) / 2.
    );
    assert_eq!(first.origin.y, viewport.origin.y);
    assert_eq!(status.size.width, body.size.width);
    assert_eq!(command.size.width, body.size.width);
}

#[gpui_kit::test]
fn default_width_is_960_in_a_wide_window(cx: &mut TestAppContext) {
    let (window, view) = open(cx, "paragraph".into());
    cx.simulate_window_resize(window, size(px(1400.), px(360.)));
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    let viewport = view.read_with(cx, |viewer, cx| {
        viewer.document.text.read(cx).list_state().viewport_bounds()
    });
    assert_eq!(viewport.size.width, px(960.));
    assert_eq!(viewport.origin.x, px(220.));
}

#[gpui_kit::test]
fn code_block_scrolls_horizontally_without_expanding_the_body(cx: &mut TestAppContext) {
    let code = "A".repeat(60) + &"B".repeat(60) + &"C".repeat(60);
    let (window, view) = open(cx, format!("```rust\n{code}\n```\n\nparagraph"));
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.config.max_width = 320.;
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    let (viewport, row) = view.read_with(cx, |viewer, cx| {
        let list = viewer.document.text.read(cx).list_state();
        (list.viewport_bounds(), list.bounds_for_item(0).unwrap())
    });
    assert_eq!(viewport.size.width, px(320.));
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    let start = row.origin + point(px(30.), px(38.));
    let end = start + point(px(50.), px(0.));
    let select = |visual: &mut gpui_kit::VisualTestContext| {
        visual.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        visual.update(|window, cx| window.render_frame(cx));
        view.read_with(visual, |viewer, cx| {
            viewer.document.text.read(cx).selected_text()
        })
    };
    let before = select(&mut visual);
    visual.simulate_event(ScrollEvent {
        position: start,
        delta: ScrollDelta::Pixels(point(px(-500.), px(0.))),
        touch_phase: TouchPhase::Moved,
        ..Default::default()
    });
    visual.update(|window, cx| window.render_frame(cx));
    let after = select(&mut visual);
    assert!(
        !before.trim().is_empty() && before.trim().chars().all(|ch| ch == 'A'),
        "{before:?}"
    );
    assert!(after.contains('B'), "{after:?}");
    visual.simulate_event(ScrollEvent {
        position: start,
        delta: ScrollDelta::Pixels(point(px(-500.), px(0.))),
        touch_phase: TouchPhase::Moved,
        ..Default::default()
    });
    visual.update(|window, cx| window.render_frame(cx));
    let end_text = select(&mut visual);
    assert!(end_text.contains('C'), "{end_text:?}");
    let width = view.read_with(&mut visual, |viewer, cx| {
        viewer
            .document
            .text
            .read(cx)
            .list_state()
            .viewport_bounds()
            .size
            .width
    });
    assert_eq!(width, px(320.));
}

#[gpui_kit::test]
fn language_label_preserves_the_upstream_code_block_spacing(cx: &mut TestAppContext) {
    let (window, view) = open(
        cx,
        "```\ncode\n```\n\n```rust\ncode\n```\n\nparagraph".into(),
    );
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    let (plain, labeled) = view.read_with(cx, |viewer, cx| {
        let list = viewer.document.text.read(cx).list_state();
        (
            list.bounds_for_item(0).unwrap(),
            list.bounds_for_item(1).unwrap(),
        )
    });
    assert_eq!(labeled.size.height - plain.size.height, px(14.));
}

#[gpui_kit::test]
fn vertical_wheel_over_code_scrolls_the_document(cx: &mut TestAppContext) {
    let source = format!(
        "```rust\nshort code\n```\n\n{}",
        "long paragraph\n\n".repeat(100)
    );
    let (window, view) = open(cx, source);
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    let row = view.read_with(cx, |viewer, cx| {
        viewer
            .document
            .text
            .read(cx)
            .list_state()
            .bounds_for_item(0)
            .unwrap()
    });
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    visual.simulate_event(ScrollEvent {
        position: row.origin + point(px(30.), px(38.)),
        delta: ScrollDelta::Pixels(point(px(0.), px(-100.))),
        touch_phase: TouchPhase::Moved,
        ..Default::default()
    });
    visual.update(|window, cx| window.render_frame(cx));
    let offset = view.read_with(&mut visual, |viewer, cx| {
        viewer
            .document
            .text
            .read(cx)
            .list_state()
            .scroll_px_offset_for_scrollbar()
            .y
    });
    assert!(offset < px(0.), "document did not scroll: {offset:?}");
}

#[gpui_kit::test]
fn wide_table_scrolls_horizontally_without_expanding_the_body(cx: &mut TestAppContext) {
    let source = format!(
        "| {} | {} | {} |\n| --- | --- | --- |\n| one | two | three |",
        "A".repeat(80),
        "B".repeat(80),
        "C".repeat(80)
    );
    let (window, view) = open(cx, source);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.config.max_width = 320.;
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    let (viewport, row) = view.read_with(cx, |viewer, cx| {
        let list = viewer.document.text.read(cx).list_state();
        (list.viewport_bounds(), list.bounds_for_item(0).unwrap())
    });
    assert_eq!(viewport.size.width, px(320.));
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    let start = row.origin + point(px(20.), px(12.));
    let end = start + point(px(50.), px(0.));
    let select = |visual: &mut gpui_kit::VisualTestContext| {
        visual.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        visual.update(|window, cx| window.render_frame(cx));
        view.read_with(visual, |viewer, cx| {
            viewer.document.text.read(cx).selected_text()
        })
    };
    let before = select(&mut visual);
    visual.simulate_event(ScrollEvent {
        position: start,
        delta: ScrollDelta::Pixels(point(px(-500.), px(0.))),
        touch_phase: TouchPhase::Moved,
        ..Default::default()
    });
    visual.update(|window, cx| window.render_frame(cx));
    let after = select(&mut visual);
    assert!(
        !before.trim().is_empty() && before.trim().chars().all(|ch| ch == 'A'),
        "{before:?}"
    );
    assert!(after.contains('B'), "{after:?}");
    let width = view.read_with(&mut visual, |viewer, cx| {
        viewer
            .document
            .text
            .read(cx)
            .list_state()
            .viewport_bounds()
            .size
            .width
    });
    assert_eq!(width, px(320.));
}

#[gpui_kit::test]
fn paragraph_and_markdown_table_alignment_control_link_hit_positions(cx: &mut TestAppContext) {
    let source = "[paragraph](paragraph.md)\n\n| [L](left.md) | [C](center.md) | [R](right.md) |\n| :--- | :---: | ---: |";
    let (window, view) = open(cx, source.into());
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.config.max_width = 320.;
            cx.notify();
        })
    });
    let requests = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = requests.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone())
        })
    });
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    let (paragraph, table) = view.read_with(cx, |viewer, cx| {
        let list = viewer.document.text.read(cx).list_state();
        (
            list.bounds_for_item(0).unwrap(),
            list.bounds_for_item(1).unwrap(),
        )
    });
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    visual.simulate_click(
        paragraph.origin + point(px(3.), px(8.)),
        Modifiers::default(),
    );
    assert_eq!(
        requests
            .borrow_mut()
            .pop()
            .unwrap()
            .path
            .file_name()
            .unwrap(),
        "paragraph.md"
    );
    let mut hits = Vec::new();
    for x in (0..320).step_by(2) {
        visual.simulate_click(
            table.origin + point(px(x as f32), px(14.)),
            Modifiers::default(),
        );
        if let Some(request) = requests.borrow_mut().pop() {
            hits.push((
                x,
                request
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            ));
        }
    }
    let position = |name: &str| {
        hits.iter()
            .find(|(_, target)| target == name)
            .map(|(x, _)| *x)
    };
    let left = position("left.md").expect("left link");
    let center = position("center.md").expect("center link");
    let right = position("right.md").expect("right link");
    assert!(
        left < 30 && (140..180).contains(&center) && right > 280,
        "{hits:?}"
    );
}

#[gpui_kit::test]
fn mermaid_image_scales_to_body_width_with_its_aspect_ratio(cx: &mut TestAppContext) {
    let (window, view) = open(cx, "```mermaid\nflowchart TD\nA --> B\n```".into());
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.config.max_width = 8.;
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.complete_diagram(
                DiagramResult {
                    request: running_diagram(viewer),
                    png: Ok(PNG.to_vec()),
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    let bounds = visual.debug_bounds("mermaid-0").unwrap();
    assert_eq!(bounds.size, size(px(8.), px(4.)));
}

#[gpui_kit::test]
fn local_image_scales_to_body_width_with_its_aspect_ratio(cx: &mut TestAppContext) {
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let path = std::env::temp_dir().join(format!("mmv-layout-image-{}.png", std::process::id()));
    std::fs::write(&path, PNG).unwrap();
    let fixture = Fixture(path);
    let url = url::Url::from_file_path(&fixture.0).unwrap();
    let (window, view) = open(cx, format!("![test]({url})"));
    cx.run_until_parked();
    for width in [100., 8.] {
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                viewer.config.max_width = width;
                cx.notify();
            })
        });
        for _ in 0..3 {
            cx.update_window(window, |_, window, cx| window.render_frame(cx))
                .unwrap();
            cx.run_until_parked();
        }
        let row = view.read_with(cx, |viewer, cx| {
            viewer
                .document
                .text
                .read(cx)
                .list_state()
                .bounds_for_item(0)
                .unwrap()
        });
        assert_eq!(
            row.size.height,
            if width == 100. { px(10.) } else { px(4.) },
            "image at body width {width}"
        );
    }
}

#[gpui_kit::test]
fn left_and_narrow_layouts_keep_margin_and_maximum(cx: &mut TestAppContext) {
    let (window, view) = open(cx, "paragraph".into());
    for (position, maximum, expected_width, expected_x) in [
        (Position::Left, 320., 320., 16.),
        (Position::Left, 960., 608., 16.),
        (Position::Center, f64::MAX, 608., 16.),
        (Position::Left, f64::MIN_POSITIVE, 0., 16.),
    ] {
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                viewer.config.position = position;
                viewer.config.max_width = maximum;
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
        let viewport = view.read_with(cx, |viewer, cx| {
            viewer.document.text.read(cx).list_state().viewport_bounds()
        });
        assert_eq!(viewport.size.width, px(expected_width));
        assert_eq!(viewport.origin.x, px(expected_x));
    }
}
