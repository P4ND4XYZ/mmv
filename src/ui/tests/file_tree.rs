use super::selection_panel::{frame, input, press};
use super::*;
use crate::app::file_tree::{Completion, DirectoryRequest, Entry, Request};
use gpui_kit::{EntityInputHandler as _, VisualTestContext};

struct TreeTest {
    window: gpui_kit::AnyWindowHandle,
    view: gpui_kit::Entity<Viewer>,
    requests: Rc<RefCell<Vec<Request>>>,
    _subscription: gpui_kit::Subscription,
}

impl TreeTest {
    fn new(cx: &mut TestAppContext, visible: bool, width: f32) -> Self {
        let (window, view) = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::bind_keys(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(width), px(360.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let mut config = crate::config::Config::default();
                        config.tree.visible = visible;
                        Viewer::with_config(
                            PathBuf::new(),
                            Instant::now(),
                            config,
                            false,
                            window,
                            cx,
                        )
                    })
                },
            )
            .unwrap()
        });
        let requests = Rc::new(RefCell::new(Vec::new()));
        let captured = requests.clone();
        let subscription = cx.update(|cx| {
            cx.subscribe(&view, move |_, request: &Request, _| {
                captured.borrow_mut().push(request.clone());
            })
        });
        cx.update(|cx| {
            view.update(cx, |viewer, cx| {
                let request = viewer.app.open(PathBuf::from(r"C:\tree\sample.md"));
                viewer.complete(
                    success(request, "[Link](other.md)\n\n本文 target\n\n".repeat(60)),
                    cx,
                );
            })
        });
        frame(cx, window);
        Self {
            window,
            view,
            requests,
            _subscription: subscription,
        }
    }

    fn listing(&self) -> DirectoryRequest {
        match self.requests.borrow_mut().remove(0) {
            Request::Directory(request) => request,
            _ => panic!("expected directory request"),
        }
    }

    fn seed(&self, cx: &mut TestAppContext) {
        let request = match self.requests.borrow_mut().remove(0) {
            Request::Root(request) => request,
            _ => panic!("expected root request"),
        };
        cx.update(|cx| {
            self.view.update(cx, |viewer, cx| {
                viewer.complete_tree(Completion::Root(request, PathBuf::from(r"C:\tree")), cx);
            })
        });
        let listing = self.listing();
        self.complete(
            cx,
            listing,
            Ok(vec![entry("folder", true), entry("sample.md", false)]),
        );
    }

    fn complete(
        &self,
        cx: &mut TestAppContext,
        request: DirectoryRequest,
        entries: Result<Vec<Entry>, ()>,
    ) {
        cx.update(|cx| {
            self.view.update(cx, |viewer, cx| {
                viewer.complete_tree(Completion::Directory(request, entries), cx)
            })
        });
        frame(cx, self.window);
    }

    fn press(&self, cx: &mut TestAppContext, key: &str) {
        press(cx, self.window, key);
    }

    fn focus_tree(&self, cx: &mut TestAppContext) {
        self.press(cx, "ctrl-w");
        self.press(cx, "h");
        cx.update_window(self.window, |_, window, cx| {
            self.view.read_with(cx, |viewer, _| {
                assert!(viewer.tree.focus.is_focused(window));
            })
        })
        .unwrap();
    }
}

fn entry(name: &str, directory: bool) -> Entry {
    Entry {
        path: PathBuf::from(r"C:\tree").join(name),
        name: name.into(),
        directory,
        expandable: directory,
        reparse: false,
    }
}

struct TreeFiles(PathBuf);

impl TreeFiles {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "mmv-tree-runtime-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TreeFiles {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[gpui_kit::test]
fn real_background_document_and_tree_delivery_uses_canonical_root_and_refreshes_only_expanded_branches(
    cx: &mut TestAppContext,
) {
    let files = TreeFiles::new();
    std::fs::create_dir(files.0.join("docs")).unwrap();
    std::fs::write(files.0.join(".marksman.toml"), "").unwrap();
    std::fs::write(files.0.join("asset.bin"), "").unwrap();
    std::fs::write(files.0.join("docs").join("a.md"), "# Real document").unwrap();
    let test = TreeTest::new(cx, false, 800.);
    cx.update(|cx| {
        crate::runtime::connect_document_reads(&test.view, cx);
        crate::runtime::connect_file_tree(&test.view, cx);
        test.view.update(cx, |viewer, cx| {
            viewer.request_document(
                files.0.join("docs").join("..").join("docs").join("a.md"),
                None,
                cx,
            );
        });
    });
    frame(cx, test.window);
    assert!(
        test.requests.borrow().is_empty(),
        "hidden document adoption does not scan"
    );
    test.press(cx, "ctrl-shift-e");
    let root = std::fs::canonicalize(&files.0).unwrap();
    test.view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.app.tree.root, Some(root.clone()));
        assert_eq!(viewer.app.tree.rows().len(), 4);
    });
    let roots = test
        .requests
        .borrow()
        .iter()
        .filter(|request| matches!(request, Request::Root(_)))
        .count();
    assert_eq!(roots, 1);
    assert!(test.requests.borrow().iter().any(|request| matches!(request, Request::Root(request) if request.document == std::fs::canonicalize(files.0.join("docs/a.md")).unwrap())));
    test.focus_tree(cx);
    test.press(cx, "j");
    test.press(cx, "l");
    let generation = test.view.read_with(cx, |viewer, _| {
        assert!(
            viewer
                .app
                .tree
                .rows()
                .iter()
                .any(|(path, _)| path == &root.join("docs/a.md"))
        );
        viewer.app.document().unwrap().generation
    });
    std::fs::write(files.0.join("docs/.git"), "").unwrap();
    std::fs::write(files.0.join("docs/new.txt"), "new").unwrap();
    test.press(cx, "r");
    test.view.read_with(cx, |viewer, _| {
        assert_eq!(
            viewer.app.tree.root,
            Some(root.clone()),
            "refresh does not discover the new nearer marker"
        );
        assert!(
            viewer
                .app
                .tree
                .rows()
                .iter()
                .any(|(path, _)| path == &root.join("docs/new.txt"))
        );
        assert_eq!(viewer.app.document().unwrap().generation, generation);
    });
    assert_eq!(
        test.requests
            .borrow()
            .iter()
            .filter(|request| matches!(request, Request::Root(_)))
            .count(),
        roots
    );
    test.press(cx, "h");
    let start = test.requests.borrow().len();
    test.press(cx, "r");
    let requests = test.requests.borrow();
    assert_eq!(requests.len() - start, 1);
    assert!(matches!(&requests[start], Request::Directory(request) if request.path == root));
}

#[gpui_kit::test]
fn enter_and_l_open_markdown_but_right_arrow_and_other_files_do_not(cx: &mut TestAppContext) {
    for key in ["enter", "l"] {
        let test = TreeTest::new(cx, true, 800.);
        test.seed(cx);
        let reads = Rc::new(RefCell::new(Vec::new()));
        let captured = reads.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&test.view, move |_, request: &ReadRequest, _| {
                captured.borrow_mut().push(request.clone());
            })
        });
        test.focus_tree(cx);
        cx.update(|cx| {
            test.view.update(cx, |viewer, _| {
                viewer.app.tree.cursor = Some(PathBuf::from(r"C:\tree").join("sample.md"));
            })
        });
        test.press(cx, "right");
        assert!(reads.borrow().is_empty());
        let generation = test
            .view
            .read_with(cx, |viewer, _| viewer.app.document().unwrap().generation);
        test.press(cx, key);
        let request = reads.borrow_mut().pop().expect("tree document request");
        assert!(request.tree.is_some());
        cx.update(|cx| {
            test.view.update(cx, |viewer, cx| {
                viewer.complete(success(request, "ignored same file".into()), cx);
                assert_eq!(viewer.app.document().unwrap().generation, generation);
            })
        });
        frame(cx, test.window);
        cx.update_window(test.window, |_, window, cx| {
            test.view.read_with(cx, |viewer, _| {
                assert!(viewer.body_focus.is_focused(window));
            })
        })
        .unwrap();
        test.focus_tree(cx);
        // Different files move focus only after successful completion.
        cx.update(|cx| {
            test.view.update(cx, |viewer, cx| {
                let listing = viewer.app.tree.act(crate::app::file_tree::Action::Refresh);
                let Request::Directory(listing) = listing.into_iter().next().unwrap() else {
                    panic!()
                };
                let mut link = entry("link.md", false);
                link.reparse = true;
                viewer.complete_tree(
                    Completion::Directory(
                        listing,
                        Ok(vec![
                            entry("other.MD", false),
                            entry("asset.txt", false),
                            link,
                        ]),
                    ),
                    cx,
                );
            })
        });
        for name in ["asset.txt", "link.md"] {
            cx.update(|cx| {
                test.view.update(cx, |viewer, _| {
                    viewer.app.tree.cursor = Some(PathBuf::from(r"C:\tree").join(name));
                })
            });
            test.press(cx, key);
            assert!(reads.borrow().is_empty());
        }
        cx.update(|cx| {
            test.view.update(cx, |viewer, _| {
                viewer.app.tree.cursor = Some(PathBuf::from(r"C:\tree").join("other.MD"));
            })
        });
        test.press(cx, key);
        let request = reads.borrow_mut().pop().unwrap();
        cx.update(|cx| {
            test.view.update(cx, |viewer, cx| {
                viewer.complete(success(request, "# Other".into()), cx)
            })
        });
        frame(cx, test.window);
        cx.update_window(test.window, |_, window, cx| {
            test.view.read_with(cx, |viewer, cx| {
                assert!(viewer.body_focus.is_focused(window));
                assert_eq!(viewer.document.distance(cx), 0.);
                assert_eq!(viewer.app.document().unwrap().source, "# Other");
            })
        })
        .unwrap();
        test.press(cx, "ctrl-w");
        test.press(cx, "l");
        test.press(cx, "ctrl-o");
        assert!(
            reads
                .borrow_mut()
                .pop()
                .unwrap()
                .path
                .ends_with("sample.md")
        );
    }
}

#[gpui_kit::test]
fn real_tree_open_keeps_same_root_state_and_reports_stale_file_failure_in_english(
    cx: &mut TestAppContext,
) {
    let files = TreeFiles::new();
    std::fs::write(files.0.join(".marksman.toml"), "").unwrap();
    std::fs::write(files.0.join("a.md"), "# Original 日本語\n\n".repeat(80)).unwrap();
    std::fs::write(
        files.0.join("別文書.MD"),
        "# Destination 日本語\n\n".repeat(80),
    )
    .unwrap();
    std::fs::write(files.0.join("stale.md"), "# Deleted").unwrap();
    let test = TreeTest::new(cx, false, 800.);
    cx.update(|cx| {
        crate::runtime::connect_document_reads(&test.view, cx);
        crate::runtime::connect_file_tree(&test.view, cx);
        test.view.update(cx, |viewer, cx| {
            viewer.request_document(files.0.join("a.md"), None, cx);
        });
    });
    frame(cx, test.window);
    test.press(cx, "shift-g");
    let original_distance = test
        .view
        .read_with(cx, |viewer, cx| viewer.document.distance(cx));
    assert!(original_distance > 0.);
    test.press(cx, "ctrl-shift-e");
    test.focus_tree(cx);
    let root = std::fs::canonicalize(&files.0).unwrap();
    let destination = root.join("別文書.MD");
    let stale = root.join("stale.md");
    cx.update(|cx| {
        test.view.update(cx, |viewer, _| {
            viewer.app.tree.cursor = Some(destination.clone());
        })
    });
    test.press(cx, "enter");
    let (text, generation, rows, scroll, distance) = test.view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.app.document().unwrap().path, destination);
        assert!(
            viewer
                .app
                .document()
                .unwrap()
                .source
                .contains("Destination 日本語")
        );
        assert_eq!(viewer.app.tree.root, Some(root.clone()));
        assert_eq!(viewer.app.tree.cursor, Some(destination.clone()));
        assert_eq!(viewer.document.distance(cx), 0.);
        (
            viewer.document.text.clone(),
            viewer.app.document().unwrap().generation,
            viewer.app.tree.rows(),
            viewer.tree.scroll.offset(),
            viewer.document.distance(cx),
        )
    });
    test.focus_tree(cx);
    test.press(cx, "l");
    test.view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.document.text, text);
        assert_eq!(viewer.app.document().unwrap().generation, generation);
        assert_eq!(viewer.app.tree.rows(), rows);
        assert_eq!(viewer.tree.scroll.offset(), scroll);
    });
    test.focus_tree(cx);
    std::fs::remove_file(&stale).unwrap();
    cx.update(|cx| {
        test.view
            .update(cx, |viewer, _| viewer.app.tree.cursor = Some(stale.clone()))
    });
    test.press(cx, "l");
    cx.update_window(test.window, |_, window, cx| {
        test.view.read_with(cx, |viewer, cx| {
            assert!(viewer.tree.focus.is_focused(window));
            assert_eq!(viewer.document.text, text);
            assert_eq!(viewer.app.document().unwrap().generation, generation);
            assert_eq!(viewer.document.distance(cx), distance);
            assert_eq!(viewer.app.tree.rows(), rows);
            assert_eq!(viewer.app.tree.cursor, Some(stale.clone()));
            assert_eq!(
                viewer.status_text(),
                format!("File not found: {}", stale.display())
            );
        })
    })
    .unwrap();
    // Failure and same-document re-selection must not add history entries.
    test.press(cx, "ctrl-w");
    test.press(cx, "l");
    test.press(cx, "ctrl-o");
    test.view.read_with(cx, |viewer, cx| {
        assert!(viewer.app.document().unwrap().path.ends_with("a.md"));
        assert!((viewer.document.distance(cx) - original_distance).abs() < 2.);
    });
}

#[gpui_kit::test]
fn startup_focus_fixed_panes_and_full_width_footer(cx: &mut TestAppContext) {
    for visible in [false, true] {
        let test = TreeTest::new(cx, visible, crate::ui::pane::MIN_WINDOW_WIDTH);
        cx.update_window(test.window, |_, window, cx| {
            test.view.read_with(cx, |viewer, _| {
                assert!(viewer.body_focus.is_focused(window));
                assert!(!viewer.tree.focus.is_focused(window));
                assert_eq!(viewer.tree.visible, visible);
            })
        })
        .unwrap();
        let mut visual = VisualTestContext::from_window(test.window, cx);
        let body = visual.debug_bounds("document-body").unwrap();
        let footer = visual.debug_bounds("status-line").unwrap();
        assert_eq!(footer.size.width, px(481.));
        if visible {
            let tree = visual.debug_bounds("file-tree").unwrap();
            assert_eq!(tree.size.width, px(240.));
            assert_eq!(body.size.width, px(240.));
            assert_eq!(body.left() - tree.right(), px(1.));
            assert_eq!(body.bottom(), tree.bottom());
        } else {
            assert!(visual.debug_bounds("file-tree").is_none());
            assert_eq!(body.size.width, px(481.));
        }
        assert_eq!(body.bottom(), footer.top());
    }
}

#[gpui_kit::test]
fn tree_keys_are_read_only_and_do_not_leak_to_body_or_history(cx: &mut TestAppContext) {
    let test = TreeTest::new(cx, true, 800.);
    test.seed(cx);
    let (text, generation, prompt, distance) = test.view.read_with(cx, |viewer, cx| {
        (
            viewer.document.text.clone(),
            viewer.app.document().unwrap().generation,
            viewer.input.state.clone(),
            viewer.document.distance(cx),
        )
    });
    let reads = Rc::new(RefCell::new(Vec::new()));
    let captured = reads.clone();
    let _reads = cx.update(|cx| {
        cx.subscribe(&test.view, move |_, request: &ReadRequest, _| {
            captured.borrow_mut().push(request.clone())
        })
    });
    test.focus_tree(cx);
    for key in [
        "j", "k", "down", "up", "g", "g", "end", "home", "shift-g", "left", "right", "enter", "f5",
        "ctrl-o", "ctrl-i", "/", ":", "n", "shift-n", "f", "ctrl-d", "ctrl-u",
    ] {
        test.press(cx, key);
    }
    assert!(reads.borrow().is_empty());
    test.view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.document.text, text);
        assert_eq!(viewer.app.document().unwrap().generation, generation);
        assert_eq!(viewer.input.state, prompt);
        assert_eq!(viewer.app.mode, crate::app::Mode::Normal);
        assert_eq!(viewer.document.distance(cx), distance);
    });
    test.press(cx, "ctrl-w");
    test.press(cx, "l");
    test.press(cx, "/");
    assert_eq!(
        test.view.read_with(cx, |viewer, _| viewer.app.mode.clone()),
        crate::app::Mode::Search
    );
}

#[gpui_kit::test]
fn hiding_preserves_inflight_listing_and_reveal_neither_refetches_nor_steals_focus(
    cx: &mut TestAppContext,
) {
    let test = TreeTest::new(cx, false, 800.);
    assert!(test.requests.borrow().is_empty());
    test.press(cx, "ctrl-w");
    test.press(cx, "h");
    assert!(!test.view.read_with(cx, |viewer, _| viewer.tree.visible));
    test.press(cx, "ctrl-shift-e");
    let request = match test.requests.borrow_mut().remove(0) {
        Request::Root(r) => r,
        _ => panic!(),
    };
    cx.update(|cx| {
        test.view.update(cx, |viewer, cx| {
            viewer.complete_tree(Completion::Root(request, PathBuf::from(r"C:\tree")), cx)
        })
    });
    let listing = test.listing();
    test.press(cx, "ctrl-shift-e");
    test.complete(cx, listing, Ok(vec![entry("folder", true)]));
    test.press(cx, "ctrl-shift-e");
    assert!(test.requests.borrow().is_empty());
    assert_eq!(
        test.view
            .read_with(cx, |viewer, _| viewer.app.tree.rows().len()),
        2
    );
    cx.update_window(test.window, |_, window, cx| {
        test.view.read_with(cx, |viewer, _| {
            assert!(viewer.body_focus.is_focused(window))
        })
    })
    .unwrap();
    test.focus_tree(cx);
    test.press(cx, "j");
    test.press(cx, "ctrl-shift-e");
    test.press(cx, "ctrl-shift-e");
    test.view.read_with(cx, |viewer, _| {
        assert_eq!(
            viewer.app.tree.cursor,
            Some(PathBuf::from(r"C:\tree\folder"))
        )
    });
    assert!(test.requests.borrow().is_empty());
}

#[gpui_kit::test]
fn pending_gpui_pane_chord_cannot_cross_mode_visibility_or_composition_changes(
    cx: &mut TestAppContext,
) {
    let test = TreeTest::new(cx, true, 800.);
    test.seed(cx);
    test.press(cx, "ctrl-w");
    cx.update_window(test.window, |_, window, cx| {
        test.view
            .update(cx, |viewer, cx| viewer.begin_input(true, window, cx))
    })
    .unwrap();
    frame(cx, test.window);
    test.press(cx, "h");
    test.view.read_with(cx, |viewer, cx| {
        assert_eq!(viewer.input.state.read(cx).value().as_ref(), "h")
    });
    test.press(cx, "escape");
    test.press(cx, "ctrl-w");
    cx.update_window(test.window, |_, window, cx| {
        test.view
            .update(cx, |viewer, cx| viewer.toggle_tree(window, cx))
    })
    .unwrap();
    frame(cx, test.window);
    test.press(cx, "h");
    assert!(!test.view.read_with(cx, |viewer, _| viewer.tree.visible));
    test.press(cx, "ctrl-shift-e");
    test.press(cx, "ctrl-w");
    cx.update_window(test.window, |_, window, cx| {
        test.view.update(cx, |viewer, cx| {
            viewer.input.state.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "未", None, window, cx)
            });
            cx.notify();
        })
    })
    .unwrap();
    frame(cx, test.window);
    test.press(cx, "h");
    cx.update_window(test.window, |_, window, cx| {
        test.view.update(cx, |viewer, cx| {
            assert!(viewer.body_focus.is_focused(window));
            viewer
                .input
                .state
                .update(cx, |input, cx| input.unmark_text(window, cx));
            cx.notify();
        })
    })
    .unwrap();
    frame(cx, test.window);
    test.press(cx, "h");
    test.press(cx, "ctrl-w");
    test.press(cx, "ctrl-h");
    cx.update_window(test.window, |_, window, cx| {
        test.view.read_with(cx, |viewer, _| {
            assert!(viewer.body_focus.is_focused(window))
        })
    })
    .unwrap();
    test.focus_tree(cx);
}

#[gpui_kit::test]
fn tree_navigation_expansion_and_refresh_keys_have_separate_effects(cx: &mut TestAppContext) {
    let test = TreeTest::new(cx, true, 800.);
    test.seed(cx);
    test.focus_tree(cx);
    let cursor = |cx: &TestAppContext| {
        test.view
            .read_with(cx, |viewer, _| viewer.app.tree.cursor.clone())
    };
    test.press(cx, "j");
    assert_eq!(cursor(cx), Some(PathBuf::from(r"C:\tree\folder")));
    test.press(cx, "l");
    let folder = test.listing();
    assert_eq!(folder.path, PathBuf::from(r"C:\tree\folder"));
    test.complete(
        cx,
        folder,
        Ok(vec![Entry {
            path: PathBuf::from(r"C:\tree\folder\child.txt"),
            name: "child.txt".into(),
            directory: false,
            expandable: false,
            reparse: false,
        }]),
    );
    test.press(cx, "right");
    assert_eq!(cursor(cx), Some(PathBuf::from(r"C:\tree\folder\child.txt")));
    test.press(cx, "enter");
    assert!(test.requests.borrow().is_empty());
    test.press(cx, "left");
    assert_eq!(cursor(cx), Some(PathBuf::from(r"C:\tree\folder")));
    test.press(cx, "r");
    let refreshed: Vec<_> = test.requests.borrow_mut().drain(..).collect();
    assert_eq!(refreshed.len(), 2);
    assert!(
        refreshed
            .iter()
            .all(|request| matches!(request, Request::Directory(_)))
    );
    for request in refreshed {
        let Request::Directory(request) = request else {
            unreachable!()
        };
        // A failure keeps the existing content and allows the remaining keys.
        test.complete(cx, request, Err(()));
    }
    // Root failure allows retry and pane movement, but never enters retained children.
    test.press(cx, "right");
    assert_eq!(cursor(cx), Some(PathBuf::from(r"C:\tree")));
    test.press(cx, "r");
    let root = test.listing();
    test.complete(
        cx,
        root,
        Ok(vec![entry("folder", true), entry("sample.md", false)]),
    );
    for (key, expected) in [
        ("end", r"C:\tree\sample.md"),
        ("up", r"C:\tree\folder\child.txt"),
        ("home", r"C:\tree"),
        ("down", r"C:\tree\folder"),
    ] {
        test.press(cx, key);
        assert_eq!(cursor(cx), Some(PathBuf::from(expected)));
    }
    test.press(cx, "enter");
    assert_eq!(
        test.view
            .read_with(cx, |viewer, _| viewer.app.tree.rows().len()),
        3
    );
    test.press(cx, "enter");
    assert_eq!(test.listing().path, PathBuf::from(r"C:\tree\folder"));
    test.press(cx, "h");
    test.press(cx, "h");
    assert_eq!(cursor(cx), Some(PathBuf::from(r"C:\tree")));
    test.press(cx, "shift-g");
    assert_eq!(cursor(cx), Some(PathBuf::from(r"C:\tree\sample.md")));
    test.press(cx, "g");
    test.press(cx, "g");
    assert_eq!(cursor(cx), Some(PathBuf::from(r"C:\tree")));
}

#[gpui_kit::test]
fn tree_scroll_is_minimal_and_retained_for_hide_and_same_root_only(cx: &mut TestAppContext) {
    let test = TreeTest::new(cx, true, 800.);
    test.seed(cx);
    test.focus_tree(cx);
    test.press(cx, "r");
    let request = test.listing();
    test.complete(
        cx,
        request,
        Ok((0..80)
            .map(|i| entry(&format!("file{i:02}.txt"), false))
            .collect()),
    );
    test.press(cx, "end");
    let offset = test
        .view
        .read_with(cx, |viewer, _| viewer.tree.scroll.offset());
    assert!(offset.y < px(0.));
    test.press(cx, "k");
    assert_eq!(
        test.view
            .read_with(cx, |viewer, _| viewer.tree.scroll.offset()),
        offset
    );
    let cursor = test
        .view
        .read_with(cx, |viewer, _| viewer.app.tree.cursor.clone());
    test.press(cx, "ctrl-shift-e");
    test.press(cx, "ctrl-shift-e");
    assert_eq!(
        test.view
            .read_with(cx, |viewer, _| viewer.tree.scroll.offset()),
        offset
    );
    assert_eq!(
        test.view
            .read_with(cx, |viewer, _| viewer.app.tree.cursor.clone()),
        cursor
    );
    test.focus_tree(cx);
    cx.update(|cx| {
        test.view.update(cx, |viewer, cx| {
            let request = viewer.app.open(PathBuf::from(r"C:\tree\another.md"));
            viewer.complete(success(request, "another".into()), cx);
        })
    });
    frame(cx, test.window);
    let mut visual = VisualTestContext::from_window(test.window, cx);
    let bounds = visual.debug_bounds("file-tree").unwrap();
    visual.simulate_event(gpui_kit::ScrollEvent {
        position: bounds.origin + point(px(30.), px(30.)),
        delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(120.))),
        modifiers: Default::default(),
        touch_phase: gpui_kit::TouchPhase::Moved,
        source: gpui_kit::ScrollSource::Pointer,
    });
    frame(cx, test.window);
    test.press(cx, "home");
    assert_eq!(
        test.view
            .read_with(cx, |viewer, _| viewer.tree.scroll.offset()),
        offset
    );
    assert_eq!(
        test.view
            .read_with(cx, |viewer, _| viewer.app.tree.cursor.clone()),
        cursor
    );
    let Request::Root(request) = test.requests.borrow_mut().remove(0) else {
        panic!()
    };
    cx.update(|cx| {
        test.view.update(cx, |viewer, cx| {
            viewer.complete_tree(Completion::Root(request, PathBuf::from(r"C:\tree")), cx)
        })
    });
    frame(cx, test.window);
    assert!(test.requests.borrow().is_empty());
    assert_eq!(
        test.view
            .read_with(cx, |viewer, _| viewer.tree.scroll.offset()),
        offset
    );
    cx.update(|cx| {
        test.view.update(cx, |viewer, cx| {
            let request = viewer.app.open(PathBuf::from(r"C:\other\new.md"));
            viewer.complete(success(request, "new".into()), cx);
        })
    });
    let Request::Root(request) = test.requests.borrow_mut().remove(0) else {
        panic!()
    };
    cx.update(|cx| {
        test.view.update(cx, |viewer, cx| {
            viewer.complete_tree(Completion::Root(request, PathBuf::from(r"C:\other")), cx)
        })
    });
    frame(cx, test.window);
    assert_eq!(
        test.view
            .read_with(cx, |viewer, _| viewer.tree.scroll.offset()),
        point(px(0.), px(0.))
    );
    assert_eq!(
        test.view
            .read_with(cx, |viewer, _| viewer.app.tree.cursor.clone()),
        Some(PathBuf::from(r"C:\other"))
    );
    assert_eq!(test.listing().path, PathBuf::from(r"C:\other"));
}

#[gpui_kit::test]
fn pane_width_changes_keep_document_entity_and_absolute_scroll_distance(cx: &mut TestAppContext) {
    let test = TreeTest::new(cx, false, 1000.);
    cx.update(|cx| {
        test.view.update(cx, |viewer, cx| {
            viewer
                .document
                .text
                .read(cx)
                .list_state()
                .set_offset_from_scrollbar(point(px(0.), -px(480.)));
            cx.notify();
        })
    });
    frame(cx, test.window);
    let (text, distance) = test.view.read_with(cx, |viewer, cx| {
        (viewer.document.text.clone(), viewer.document.distance(cx))
    });
    assert!(distance > 400.);
    for _ in 0..4 {
        test.press(cx, "ctrl-shift-e");
        test.view.read_with(cx, |viewer, cx| {
            assert_eq!(viewer.document.text, text);
            assert!(
                (viewer.document.distance(cx) - distance).abs() < 1.,
                "{} != {distance}",
                viewer.document.distance(cx)
            );
        });
    }
}

#[gpui_kit::test]
fn root_error_and_clicks_leave_body_history_prompt_and_focus_intact(cx: &mut TestAppContext) {
    let test = TreeTest::new(cx, true, 800.);
    test.seed(cx);
    test.focus_tree(cx);
    let mut visual = VisualTestContext::from_window(test.window, cx);
    let bounds = visual.debug_bounds("file-tree").unwrap();
    let before = test.view.read_with(cx, |viewer, _| {
        (
            viewer.app.tree.cursor.clone(),
            viewer.document.text.clone(),
            viewer.input.state.clone(),
        )
    });
    visual.simulate_click(bounds.origin + point(px(50.), px(40.)), Default::default());
    frame(cx, test.window);
    assert_eq!(
        test.view
            .read_with(cx, |viewer, _| viewer.app.tree.cursor.clone()),
        before.0
    );
    assert!(test.requests.borrow().is_empty());
    test.press(cx, "r");
    let request = test.listing();
    test.complete(cx, request, Err(()));
    test.view.read_with(cx, |viewer, _| {
        assert_eq!(viewer.document.text, before.1);
        assert_eq!(viewer.input.state, before.2);
        assert!(viewer.status_text().is_empty());
        assert_eq!(viewer.app.tree.rows().len(), 1);
    });
    test.press(cx, "escape");
    cx.update_window(test.window, |_, window, cx| {
        test.view.read_with(cx, |viewer, _| {
            assert!(viewer.tree.focus.is_focused(window))
        })
    })
    .unwrap();
    test.press(cx, "ctrl-w");
    test.press(cx, "l");
    test.press(cx, ":");
    input(cx, test.window, "not-a-command");
    test.view.read_with(cx, |viewer, cx| {
        assert_eq!(
            viewer.input.state.read(cx).value().as_ref(),
            "not-a-command"
        )
    });
    test.press(cx, "escape");
    cx.update(|cx| {
        test.view.update(cx, |viewer, cx| {
            assert_eq!(
                viewer.request_operation(
                    crate::app::DocumentOperation::Back,
                    crate::app::OperationOrigin::External,
                    None,
                    cx
                ),
                crate::app::OperationResult::Complete {
                    response_to: None,
                    outcome: Err(crate::app::OperationError::NoPreviousEntry)
                }
            );
        })
    });
}

#[gpui_kit::test]
fn document_failure_preserves_tree_and_root_discovery_locks_old_tree_until_completion(
    cx: &mut TestAppContext,
) {
    let test = TreeTest::new(cx, true, 800.);
    test.seed(cx);
    test.focus_tree(cx);
    test.press(cx, "j");
    let old = test.view.read_with(cx, |viewer, _| {
        (viewer.app.tree.cursor.clone(), viewer.app.tree.rows())
    });
    cx.update(|cx| {
        test.view.update(cx, |viewer, cx| {
            let request = viewer.app.open(PathBuf::from(r"C:\missing\file.md"));
            viewer.complete(
                ReadResult {
                    request,
                    content: Err("unreadable".into()),
                },
                cx,
            );
        })
    });
    assert!(test.requests.borrow().is_empty());
    assert_eq!(
        test.view.read_with(cx, |viewer, _| (
            viewer.app.tree.cursor.clone(),
            viewer.app.tree.rows()
        )),
        old
    );
    cx.update(|cx| {
        test.view.update(cx, |viewer, cx| {
            let request = viewer.app.open(PathBuf::from(r"C:\tree\next.md"));
            viewer.complete(success(request, "next".into()), cx);
        })
    });
    frame(cx, test.window);
    for key in ["j", "l", "r", "end"] {
        test.press(cx, key);
    }
    assert_eq!(
        test.view.read_with(cx, |viewer, _| (
            viewer.app.tree.cursor.clone(),
            viewer.app.tree.rows()
        )),
        old
    );
    assert_eq!(test.requests.borrow().len(), 1);
    let Request::Root(request) = test.requests.borrow_mut().remove(0) else {
        panic!()
    };
    cx.update(|cx| {
        test.view.update(cx, |viewer, cx| {
            viewer.complete_tree(Completion::Root(request, PathBuf::from(r"C:\tree")), cx)
        })
    });
    frame(cx, test.window);
    assert!(test.requests.borrow().is_empty());
    test.press(cx, "j");
    assert_eq!(
        test.view
            .read_with(cx, |viewer, _| viewer.app.tree.cursor.clone()),
        Some(PathBuf::from(r"C:\tree\sample.md"))
    );
}

#[gpui_kit::test]
fn pane_shortcuts_are_suppressed_during_search_command_panel_and_composition(
    cx: &mut TestAppContext,
) {
    let test = TreeTest::new(cx, true, 800.);
    test.seed(cx);
    for mode in ["/", ":"] {
        test.press(cx, mode);
        input(cx, test.window, "kept");
        test.press(cx, "ctrl-shift-e");
        test.press(cx, "ctrl-w");
        test.press(cx, "h");
        test.view.read_with(cx, |viewer, cx| {
            assert!(viewer.tree.visible);
            assert!(viewer.input.state.read(cx).value().starts_with("kept"));
        });
        test.press(cx, "escape");
    }
    test.press(cx, "f");
    test.press(cx, "ctrl-shift-e");
    test.press(cx, "ctrl-w");
    test.press(cx, "l");
    test.view.read_with(cx, |viewer, _| {
        assert!(viewer.tree.visible);
        assert!(viewer.link_panel.is_some());
    });
    test.press(cx, "escape");
    cx.update_window(test.window, |_, window, cx| {
        test.view.update(cx, |viewer, cx| {
            viewer.input.state.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "未確定", None, window, cx)
            });
            cx.notify();
        })
    })
    .unwrap();
    frame(cx, test.window);
    test.press(cx, "ctrl-shift-e");
    test.press(cx, "ctrl-w");
    test.press(cx, "h");
    assert!(test.view.read_with(cx, |viewer, _| viewer.tree.visible));
    cx.update_window(test.window, |_, window, cx| {
        test.view.read_with(cx, |viewer, _| {
            assert!(viewer.body_focus.is_focused(window))
        })
    })
    .unwrap();
}

#[gpui_kit::test]
fn delayed_tree_completion_preserves_focus_on_failure_staleness_or_new_input(
    cx: &mut TestAppContext,
) {
    for scenario in ["failure", "stale", "input"] {
        let test = TreeTest::new(cx, true, 800.);
        test.seed(cx);
        test.focus_tree(cx);
        let request = cx.update(|cx| {
            test.view.update(cx, |viewer, _| {
                match viewer.app.operate(
                    crate::app::DocumentOperation::TreeOpen {
                        path: PathBuf::from(r"C:\tree").join("other.md"),
                        root: PathBuf::from(r"C:\tree"),
                    },
                    crate::app::OperationOrigin::Keyboard,
                    0.,
                    None,
                ) {
                    crate::app::OperationResult::Pending(request) => request,
                    _ => panic!("expected pending tree request"),
                }
            })
        });
        frame(cx, test.window);
        cx.update_window(test.window, |_, window, cx| {
            test.view.read_with(cx, |viewer, _| {
                assert!(viewer.tree.focus.is_focused(window));
            })
        })
        .unwrap();
        if scenario == "input" {
            test.press(cx, "ctrl-w");
            test.press(cx, "l");
            test.press(cx, "/");
        }
        cx.update(|cx| {
            test.view.update(cx, |viewer, cx| {
                if scenario == "stale" {
                    viewer.app.open(PathBuf::from("newer.md"));
                }
                let result = if scenario == "failure" {
                    ReadResult {
                        request,
                        content: Err("File not found: other.md".into()),
                    }
                } else {
                    success(request, "# Other".into())
                };
                viewer.complete(result, cx);
            })
        });
        frame(cx, test.window);
        cx.update_window(test.window, |_, window, cx| {
            test.view.read_with(cx, |viewer, cx| {
                if scenario == "input" {
                    assert!(
                        viewer
                            .input
                            .state
                            .read(cx)
                            .focus_handle(cx)
                            .is_focused(window)
                    );
                } else {
                    assert!(viewer.tree.focus.is_focused(window));
                    assert!(!viewer.app.document().unwrap().source.contains("# Other"));
                }
            })
        })
        .unwrap();
    }
}
