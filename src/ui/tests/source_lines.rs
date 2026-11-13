use super::*;

fn frame(cx: &mut TestAppContext, window: gpui_kit::AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
}

fn navigate(
    cx: &mut TestAppContext,
    view: &gpui_kit::Entity<Viewer>,
    line: u64,
) -> crate::app::PositionRequest {
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let key = viewer.app.document().unwrap().key.clone();
            let crate::app::OperationResult::Pending(request) = viewer.request_external(
                crate::app::ExternalOperation::Open {
                    path: viewer.path.clone(),
                    line: std::num::NonZeroU64::new(line),
                },
                ExternalRequestId(format!("line-{line}")),
                cx,
            ) else {
                panic!("identity check");
            };
            viewer.complete(
                crate::app::ReadResponse::Resolved {
                    request,
                    key: Ok(key),
                },
                cx,
            );
            viewer.app.position_request().unwrap().clone()
        })
    })
}

fn assert_visible_source(
    cx: &mut TestAppContext,
    window: gpui_kit::AnyWindowHandle,
    view: &gpui_kit::Entity<Viewer>,
    wanted: std::ops::Range<usize>,
) {
    let viewport = view.read_with(cx, |viewer, cx| {
        viewer.document.text.read(cx).list_state().viewport_bounds()
    });
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    let mut observed = Vec::new();
    let mut select = |from, to| {
        visual.simulate_mouse_down(from, gpui_kit::MouseButton::Left, Default::default());
        visual.simulate_mouse_move(to, gpui_kit::MouseButton::Left, Default::default());
        visual.simulate_mouse_up(to, gpui_kit::MouseButton::Left, Default::default());
        let selected = view.read_with(&visual, |viewer, cx| {
            viewer.document.text.read(cx).selected_source_range()
        });
        let visible = selected
            .as_ref()
            .is_some_and(|range| range.start < wanted.end && range.end > wanted.start);
        observed.push(selected);
        visible
    };
    for x in [2., 20., 60.] {
        if select(
            viewport.origin + point(px(x), px(2.)),
            point(viewport.right() - px(2.), viewport.bottom() - px(2.)),
        ) {
            return;
        }
    }
    // Table cells own their selection, so a viewport-wide drag cannot cover
    // all visible cells. Probe horizontal runs in the actual visible rows.
    let mut y = viewport.top() + px(2.);
    while y < viewport.bottom() {
        if select(
            point(viewport.left() + px(60.), y),
            point(
                (viewport.left() + px(240.)).min(viewport.right() - px(2.)),
                y,
            ),
        ) {
            return;
        }
        y += px(10.);
    }
    panic!("source {wanted:?} is not selectable in the viewport; observed {observed:?}");
}

#[gpui_kit::test]
fn external_source_lines_reveal_paragraph_code_nested_list_and_table(cx: &mut TestAppContext) {
    let rows = (0..240)
        .map(|ix| format!("source line {ix} with words 日本語\n"))
        .collect::<String>();
    let nested = (0..240)
        .map(|ix| format!("> - source line {ix} with words 日本語\n"))
        .collect::<String>();
    let table = (0..240)
        .map(|ix| format!("| source line {ix} | value |\n"))
        .collect::<String>();
    for (source, line) in [
        (rows.clone(), 181),
        (format!("```text\n{rows}```\n"), 182),
        (nested, 181),
        (format!("| name | value |\n| --- | --- |\n{table}"), 183),
    ] {
        let start = source.find("source line 180").unwrap();
        let (window, view) = open(cx, source);
        frame(cx, window);
        let (entity, generation) = view.read_with(cx, |viewer, _| {
            (
                viewer.document.text.clone(),
                viewer.app.document().unwrap().generation,
            )
        });
        navigate(cx, &view, line);
        for _ in 0..8 {
            frame(cx, window);
        }
        assert_visible_source(cx, window, &view, start..start + 15);
        let distance = view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.document.text, entity);
            assert_eq!(viewer.app.document().unwrap().generation, generation);
            assert!(viewer.app.position_request().is_none());
            viewer.document.distance(cx)
        });
        navigate(cx, &view, line);
        for _ in 0..5 {
            frame(cx, window);
        }
        view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.document.distance(cx), distance)
        });
    }
}

#[gpui_kit::test]
fn source_line_reapplies_after_reload_resize_and_rejects_old_callbacks(cx: &mut TestAppContext) {
    let source = (0..240)
        .map(|ix| format!("source line {ix} with words 日本語\n"))
        .collect::<String>();
    let start = source.find("source line 100").unwrap();
    let (window, view) = open(cx, source.clone());
    frame(cx, window);
    let old = navigate(cx, &view, 181);
    let text = view.read_with(cx, |viewer, _| viewer.document.text.clone());
    navigate(cx, &view, 101);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.apply_position_after_layout(&old, &text, cx)
        })
    });
    for _ in 0..6 {
        frame(cx, window);
    }
    assert_visible_source(cx, window, &view, start..start + 15);
    cx.simulate_window_resize(window, size(px(420.), px(360.)));
    for _ in 0..8 {
        frame(cx, window);
    }
    assert_visible_source(cx, window, &view, start..start + 15);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let crate::app::OperationResult::Pending(request) = viewer.request_operation(
                crate::app::DocumentOperation::Reload,
                crate::app::OperationOrigin::Automatic,
                None,
                cx,
            ) else {
                panic!();
            };
            viewer.complete(success(request, source), cx);
            viewer.apply_position_after_layout(&old, &text, cx);
        })
    });
    for _ in 0..8 {
        frame(cx, window);
    }
    assert_visible_source(cx, window, &view, start..start + 15);
}

#[gpui_kit::test]
fn repeated_text_uses_source_identity_and_blank_code_lines_reveal_nearby_text(
    cx: &mut TestAppContext,
) {
    let row = "same repeated source words\n";
    let (window, view) = open(cx, row.repeat(240));
    frame(cx, window);
    navigate(cx, &view, 181);
    for _ in 0..6 {
        frame(cx, window);
    }
    assert_visible_source(cx, window, &view, 180 * row.len()..181 * row.len());
    let code = (0..240)
        .map(|ix| {
            if ix == 180 {
                "\n".into()
            } else {
                format!("line {ix}\n")
            }
        })
        .collect::<String>();
    let source = format!("```text\n{code}```");
    let start = source.find("line 181").unwrap();
    let (window, view) = open(cx, source);
    frame(cx, window);
    navigate(cx, &view, 182);
    for _ in 0..6 {
        frame(cx, window);
    }
    assert_visible_source(cx, window, &view, start..start + 8);
}

#[gpui_kit::test]
fn source_line_confirmation_preserves_later_relative_movement(cx: &mut TestAppContext) {
    let source = (0..240)
        .map(|ix| format!("line {ix} with words\n"))
        .collect::<String>();
    let (window, view) = open(cx, source);
    frame(cx, window);
    navigate(cx, &view, 181);
    for _ in 0..6 {
        frame(cx, window);
    }
    let baseline = view.read_with(cx, |viewer, cx| viewer.document.distance(cx));
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.document.scroll_edge(false, cx);
            let key = viewer.app.document().unwrap().key.clone();
            let crate::app::OperationResult::Pending(request) = viewer.request_external(
                crate::app::ExternalOperation::Open {
                    path: viewer.path.clone(),
                    line: std::num::NonZeroU64::new(181),
                },
                ExternalRequestId("line".into()),
                cx,
            ) else {
                panic!();
            };
            viewer.request_external(
                crate::app::ExternalOperation::Scroll(crate::app::ScrollDirection::Down),
                ExternalRequestId("scroll".into()),
                cx,
            );
            assert_eq!(viewer.document.distance(cx), 0.);
            viewer.complete(
                crate::app::ReadResponse::Resolved {
                    request,
                    key: Ok(key),
                },
                cx,
            );
        })
    });
    for _ in 0..8 {
        frame(cx, window);
    }
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), baseline + 20.)
    });
}

#[gpui_kit::test]
fn history_reads_wait_for_source_line_departure_layout(cx: &mut TestAppContext) {
    use crate::app::{ExternalOperation, OperationResult};
    let source = (0..240)
        .map(|ix| format!("line {ix} with words\n"))
        .collect::<String>();
    let (window, view) = open(cx, source.clone());
    frame(cx, window);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let OperationResult::Pending(request) = viewer.request_external(
                ExternalOperation::Open {
                    path: viewer.path.with_file_name("b.md"),
                    line: None,
                },
                ExternalRequestId("b".into()),
                cx,
            ) else {
                panic!();
            };
            viewer.complete(success(request, source.clone()), cx);
        })
    });
    frame(cx, window);
    navigate(cx, &view, 181);
    for _ in 0..6 {
        frame(cx, window);
    }
    let baseline = view.read_with(cx, |viewer, cx| viewer.document.distance(cx));
    let reads = Rc::new(RefCell::new(Vec::<ReadRequest>::new()));
    let captured = reads.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |view, request: &ReadRequest, cx| {
            if request.response_to == Some(ExternalRequestId("back".into())) {
                view.read_with(cx, |viewer, cx| {
                    assert_eq!(viewer.document.distance(cx), baseline)
                });
                captured.borrow_mut().push(request.clone());
            }
        })
    });
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.document.scroll_edge(false, cx);
            let key = viewer.app.document().unwrap().key.clone();
            let OperationResult::Pending(request) = viewer.request_external(
                ExternalOperation::Open {
                    path: viewer.path.clone(),
                    line: std::num::NonZeroU64::new(181),
                },
                ExternalRequestId("line".into()),
                cx,
            ) else {
                panic!();
            };
            viewer.complete(
                crate::app::ReadResponse::Resolved {
                    request,
                    key: Ok(key),
                },
                cx,
            );
            viewer.request_external(
                ExternalOperation::Back,
                ExternalRequestId("back".into()),
                cx,
            );
        })
    });
    for _ in 0..6 {
        frame(cx, window);
    }
    let back = reads
        .borrow_mut()
        .pop()
        .expect("history starts after layout");
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.complete(success(back, source.clone()), cx)
        })
    });
    frame(cx, window);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let OperationResult::Pending(request) = viewer.request_external(
                ExternalOperation::Forward,
                ExternalRequestId("forward".into()),
                cx,
            ) else {
                panic!();
            };
            viewer.complete(success(request, source), cx);
        })
    });
    frame(cx, window);
    view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.distance(cx), baseline)
    });
}

#[gpui_kit::test]
fn same_source_line_open_keeps_prompt_and_link_selection_entities(cx: &mut TestAppContext) {
    for key in ["/", ":", "f"] {
        let (window, view) = open(
            cx,
            "[Link](other.md)\n\n".to_owned() + &"edit paragraph\n\n".repeat(180),
        );
        frame(cx, window);
        cx.update_window(window, |_, window, cx| {
            window.press(key, cx);
            if key != "f" {
                window.input("edit", cx);
            }
        })
        .unwrap();
        frame(cx, window);
        let (input, mode, panel) = view.read_with(cx, |viewer, _| {
            (
                viewer.input.state.clone(),
                viewer.app.mode.clone(),
                viewer
                    .link_panel
                    .as_ref()
                    .map(|selection| selection.panel.clone()),
            )
        });
        navigate(cx, &view, 181);
        for _ in 0..6 {
            frame(cx, window);
        }
        cx.update_window(window, |_, window, cx| {
            view.read_with(cx, |viewer, cx| {
                assert_eq!(viewer.input.state, input);
                assert_eq!(viewer.app.mode, mode);
                assert_eq!(
                    viewer
                        .link_panel
                        .as_ref()
                        .map(|selection| selection.panel.clone()),
                    panel
                );
                if key != "f" {
                    assert_eq!(input.read(cx).value().as_ref(), "edit");
                    assert!(input.focus_handle(cx).is_focused(window));
                }
            })
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn image_and_mermaid_targets_reapply_after_real_dimension_changes(cx: &mut TestAppContext) {
    let image = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/mmv.png");
    let source = "before\n\n".repeat(80)
        + &format!(
            "![image]({})\n\n```mermaid\ngraph LR; A-->B\n```\n\n",
            image.display()
        )
        + &"after\n\n".repeat(80);
    let image_start = source.find("![image]").unwrap();
    let diagram_start = source.find("```mermaid").unwrap();
    let (window, view) = open(cx, String::new());
    let requests = Rc::new(RefCell::new(Vec::<DiagramRequest>::new()));
    let captured = requests.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&view, move |_, request: &DiagramRequest, _| {
            captured.borrow_mut().push(request.clone())
        })
    });
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let request = viewer.app.open(viewer.path.clone());
            viewer.complete(success(request, source), cx);
        })
    });
    frame(cx, window);
    let visible = |cx: &TestAppContext, start| {
        view.read_with(cx, |viewer, cx| {
            let analysis = viewer.document.analysis.as_ref().unwrap();
            let ix = analysis
                .blocks
                .iter()
                .position(|span| span.as_ref().is_some_and(|span| span.contains(&start)))
                .unwrap();
            let list = viewer.document.text.read(cx).list_state();
            let bounds = list.bounds_for_item(ix).unwrap();
            let viewport = list.viewport_bounds();
            assert!(
                bounds.bottom() > viewport.top() && bounds.top() < viewport.bottom(),
                "{bounds:?} / {viewport:?}"
            );
        })
    };
    navigate(cx, &view, 161);
    for _ in 0..6 {
        frame(cx, window);
    }
    visible(cx, image_start);
    navigate(cx, &view, 164);
    for _ in 0..6 {
        frame(cx, window);
    }
    visible(cx, diagram_start);
    let request = requests.borrow_mut().pop().expect("diagram request");
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            viewer.document.scroll_edge(false, cx);
            viewer.complete_diagram(
                DiagramResult {
                    request,
                    png: Ok(include_bytes!("../../../assets/mmv.png").to_vec()),
                },
                cx,
            );
        })
    });
    for _ in 0..10 {
        frame(cx, window);
    }
    visible(cx, diagram_start);
}

#[gpui_kit::test]
fn empty_blank_and_past_end_source_lines_have_bounded_positions(cx: &mut TestAppContext) {
    for source in [String::new(), "\n\n".into(), "paragraph\n\n".repeat(100)] {
        let (window, view) = open(cx, source);
        frame(cx, window);
        for line in [1, 2, u64::MAX] {
            navigate(cx, &view, line);
            for _ in 0..5 {
                frame(cx, window);
            }
            view.read_with(cx, |viewer, cx| {
                assert!(viewer.app.position_request().is_none());
                let list = viewer.document.text.read(cx).list_state();
                if line == u64::MAX {
                    assert!(list.is_scrolled_to_end().unwrap_or(true));
                }
            });
        }
    }
}

#[gpui_kit::test]
fn upstream_reveal_reaches_a_line_inside_a_long_paragraph(cx: &mut TestAppContext) {
    let source = (0..240)
        .map(|ix| format!("source line {ix} with several words 日本語\n"))
        .collect::<String>();
    let start = source.find("source line 180 ").unwrap();
    let (window, view) = open(cx, source);
    frame(cx, window);
    cx.update(|cx| {
        view.update(cx, |viewer, cx| {
            let text = viewer.document.text.clone();
            let range = text
                .read(cx)
                .rendered_text()
                .range_for_source(start..start + 15)
                .unwrap();
            text.update(cx, |text, cx| text.reveal_range(range, cx))
                .unwrap();
        })
    });
    for _ in 0..4 {
        frame(cx, window);
    }
    let (viewport, offset) = view.read_with(cx, |viewer, cx| {
        let list = viewer.document.text.read(cx).list_state();
        (list.viewport_bounds(), viewer.document.distance(cx))
    });
    assert!(offset > 500., "{offset}");
    // Select only text physically laid out within the viewport. This checks the
    // visible content, not just successful acceptance of reveal_range.
    let mut visual = gpui_kit::VisualTestContext::from_window(window, cx);
    let from = viewport.origin + point(px(1.), px(2.));
    let to = point(viewport.right() - px(2.), viewport.bottom() - px(2.));
    visual.simulate_mouse_down(from, gpui_kit::MouseButton::Left, Default::default());
    visual.simulate_mouse_move(to, gpui_kit::MouseButton::Left, Default::default());
    visual.simulate_mouse_up(to, gpui_kit::MouseButton::Left, Default::default());
    let selected = view.read_with(&visual, |viewer, cx| {
        viewer.document.text.read(cx).selected_text()
    });
    assert!(
        selected.contains("source line 180 "),
        "visible selection: {selected:?}"
    );
}
