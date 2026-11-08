use super::*;

#[test]
fn watcher_changes_only_on_adoption_and_rejects_a_b_a_delayed_notifications() {
    let mut app = App::default();
    assert!(app.watch_target().is_none());
    let initial = read(&mut app, DocumentOperation::Initial("a.md".into()));
    assert!(app.watch_target().is_none());
    finish(&mut app, initial, "canonical-a", 0.);
    let a = app.watch_target().unwrap().clone();
    let stale = read(&mut app, DocumentOperation::Open("stale.md".into()));
    let failed = read(&mut app, DocumentOperation::Open("missing.md".into()));
    fail(&mut app, failed);
    finish(&mut app, stale, "canonical-stale", 0.);
    assert_eq!(app.watch_target(), Some(&a));
    let reload = read(&mut app, DocumentOperation::Reload);
    finish(&mut app, reload, "canonical-a", 450.);
    assert_eq!(app.watch_target(), Some(&a));
    let b = read(&mut app, DocumentOperation::Open("b.md".into()));
    assert_eq!(app.watch_target(), Some(&a));
    finish(&mut app, b, "canonical-b", 0.);
    let b_target = app.watch_target().unwrap().clone();
    assert!(app.watch_changed(&a).is_none());
    let back = read(&mut app, DocumentOperation::Open("a.md".into()));
    finish(&mut app, back, "canonical-a", 0.);
    assert_ne!(app.watch_target(), Some(&a));
    assert!(app.watch_changed(&a).is_none());
    assert!(app.watch_changed(&b_target).is_none());
    assert!(!app.is_loading());
}

#[test]
fn automatic_notifications_defer_without_cancelling_switch_and_retry_after_failure() {
    for succeeds in [false, true] {
        let mut app = loaded();
        let a = app.watch_target().unwrap().clone();
        let b = read(&mut app, DocumentOperation::Open("b.md".into()));
        assert!(app.watch_changed(&a).is_none());
        assert!(app.watch_changed(&a).is_none());
        if succeeds {
            assert!(finish(&mut app, b, "canonical-b", 480.).adopted);
            assert!(app.take_automatic_reload().is_none());
        } else {
            let history = app.history.entries.clone();
            assert!(!fail(&mut app, b).adopted);
            let trailing = app.take_automatic_reload().unwrap();
            assert_eq!(trailing.path, PathBuf::from("a.md"));
            assert_eq!(trailing.origin, OperationOrigin::Automatic);
            finish(&mut app, trailing, "canonical-a", 480.);
            assert_eq!(target(&app), PositionTarget::Distance(480.));
            assert_eq!(app.history.entries, history);
            assert!(app.take_automatic_reload().is_none());
        }
    }
}

#[test]
fn saves_during_read_collapse_to_one_trailing_reload_even_after_failure() {
    for succeeds in [false, true] {
        let mut app = loaded();
        app.begin_search();
        app.preview_search("edit".into());
        let a = app.watch_target().unwrap().clone();
        let request = app.watch_changed(&a).unwrap();
        assert_eq!(request.origin, OperationOrigin::Automatic);
        assert_eq!(app.document().unwrap().source, "target edit target");
        assert!(app.watch_changed(&a).is_none());
        assert!(app.watch_changed(&a).is_none());
        if succeeds {
            finish(&mut app, request, "canonical-a", 480.);
        } else {
            let position = app.position_request().cloned();
            let history = app.history.entries.clone();
            fail(&mut app, request);
            assert_eq!(app.position_request(), position.as_ref());
            assert_eq!(app.history.entries, history);
        }
        let trailing = app.take_automatic_reload().unwrap();
        assert!(app.take_automatic_reload().is_none());
        finish(&mut app, trailing, "canonical-a", 640.);
        assert!(app.take_automatic_reload().is_none());
        assert_eq!(target(&app), PositionTarget::Distance(640.));
        assert_eq!(app.watch_target(), Some(&a));
        assert_eq!(app.mode, Mode::Search);
        let generation = app.document().unwrap().generation;
        app.rendered(generation, "edit edit old");
        assert_eq!(app.matches(), &[0..4, 5..9]);
    }
}

fn read(app: &mut App, operation: DocumentOperation) -> ReadRequest {
    pending(app.operate(operation, OperationOrigin::External, 240., None))
}

fn pending(result: OperationResult) -> ReadRequest {
    match result {
        OperationResult::Pending(request) => request,
        other => panic!("expected pending read, got {other:?}"),
    }
}

fn finish(app: &mut App, request: ReadRequest, key: &str, distance: f32) -> ReadCompletion {
    app.complete_at(
        ReadResult {
            request,
            content: Ok(LoadedDocument {
                key: DocumentKey(key.into()),
                source: "target edit target".into(),
            }),
        },
        distance,
    )
    .unwrap()
}

fn loaded() -> App {
    let mut app = App::default();
    let request = read(&mut app, DocumentOperation::Initial("a.md".into()));
    finish(&mut app, request, "canonical-a", 0.);
    app
}

fn target(app: &App) -> PositionTarget {
    app.position_request().unwrap().target.clone()
}

fn fail(app: &mut App, request: ReadRequest) -> ReadCompletion {
    app.complete_at(
        ReadResult {
            request,
            content: Err("missing".into()),
        },
        999.,
    )
    .unwrap()
}

#[test]
fn unavailable_operations_preserve_state_and_pending_reads_and_return_external_ids() {
    for has_document in [false, true] {
        for mode in [Mode::Normal, Mode::Search, Mode::Command] {
            let mut app = if has_document {
                loaded()
            } else {
                App::default()
            };
            app.mode = mode.clone();
            let request = pending(app.operate(
                DocumentOperation::Open("pending.md".into()),
                OperationOrigin::External,
                240.,
                Some(ExternalRequestId("pending-read".into())),
            ));
            let document = app
                .document()
                .map(|doc| (doc.generation, doc.path.clone(), doc.source.clone()));
            let history = app.history.entries.clone();
            let current = app.history.current;
            let position = app.position_request().cloned();
            let mut cases = vec![
                (DocumentOperation::Back, OperationError::NoPreviousEntry),
                (DocumentOperation::Forward, OperationError::NoNextEntry),
            ];
            if !has_document {
                cases.extend([
                    (
                        DocumentOperation::Reload,
                        OperationError::DocumentUnavailable,
                    ),
                    (
                        DocumentOperation::Heading {
                            path: None,
                            id: "missing".into(),
                        },
                        OperationError::DocumentUnavailable,
                    ),
                ]);
            }
            for (index, (operation, error)) in cases.into_iter().enumerate() {
                let id = ExternalRequestId(format!("unavailable-{index}"));
                assert_eq!(
                    app.operate(operation, OperationOrigin::External, 999., Some(id.clone())),
                    OperationResult::Complete {
                        response_to: Some(id),
                        outcome: Err(error)
                    }
                );
                assert_eq!(
                    app.document().map(|doc| (
                        doc.generation,
                        doc.path.clone(),
                        doc.source.clone()
                    )),
                    document
                );
                assert_eq!(app.history.entries, history);
                assert_eq!(app.history.current, current);
                assert_eq!(app.position_request(), position.as_ref());
                assert_eq!(app.mode, mode);
                assert!(app.error().is_none());
                assert!(app.is_loading());
            }
            // Rejection must not cancel or replace the previously accepted operation.
            let completion = finish(&mut app, request, "pending-key", 480.);
            assert!(completion.adopted);
            assert_eq!(
                completion.response_to,
                Some(ExternalRequestId("pending-read".into()))
            );
        }
    }
}

#[test]
fn synchronous_heading_returns_success_with_the_external_id_without_changing_input_mode() {
    for mode in [Mode::Search, Mode::Command] {
        let mut app = loaded();
        app.mode = mode.clone();
        let generation = app.document().unwrap().generation;
        let id = ExternalRequestId("heading-request".into());
        assert_eq!(
            app.operate(
                DocumentOperation::Heading {
                    path: None,
                    id: "target".into()
                },
                OperationOrigin::External,
                480.,
                Some(id.clone()),
            ),
            OperationResult::Complete {
                response_to: Some(id),
                outcome: Ok(())
            }
        );
        assert_eq!(app.mode, mode);
        assert_eq!(app.document().unwrap().generation, generation);
        assert_eq!(app.history.entries.len(), 2);
        assert_eq!(app.history.current, Some(1));
        assert_eq!(app.history.entries[0].distance, 480.);
        assert_eq!(target(&app), PositionTarget::Heading("target".into()));
        assert!(!app.is_loading());
    }
}

#[test]
fn initial_failure_creates_neither_history_nor_position_request() {
    let mut app = App::default();
    let initial = read(&mut app, DocumentOperation::Initial("missing.md".into()));
    assert!(!fail(&mut app, initial).adopted);
    assert!(app.document().is_none());
    assert!(app.history.entries.is_empty());
    assert_eq!(app.history.current, None);
    assert!(app.position_request().is_none());
    let retry = read(&mut app, DocumentOperation::Initial("a.md".into()));
    finish(&mut app, retry, "canonical-a", 0.);
    assert_eq!(app.history.current, Some(0));
    assert_eq!(target(&app), PositionTarget::Top);
}

#[test]
fn external_history_navigation_preserves_search_and_command_modes() {
    for mode in [Mode::Search, Mode::Command] {
        let mut app = loaded();
        let b = read(&mut app, DocumentOperation::Open("b.md".into()));
        finish(&mut app, b, "canonical-b", 480.);
        if mode == Mode::Search {
            app.begin_search();
            app.preview_search("edit".into());
        } else {
            app.begin_command();
        }
        let back = read(&mut app, DocumentOperation::Back);
        assert_eq!(app.mode, mode);
        finish(&mut app, back, "canonical-a", 600.);
        assert_eq!(app.mode, mode);
        assert_eq!(target(&app), PositionTarget::Distance(480.));
        let position = app.position_request().cloned();
        app.rendered(app.document().unwrap().generation, "edit edit");
        assert_eq!(app.position_request(), position.as_ref());
        if mode == Mode::Search {
            assert_eq!(app.matches(), &[0..4, 5..9]);
        }
        let forward = read(&mut app, DocumentOperation::Forward);
        finish(&mut app, forward, "canonical-b", 500.);
        assert_eq!(app.mode, mode);
        assert_eq!(target(&app), PositionTarget::Distance(600.));
    }
}

#[test]
fn open_classifies_by_resolved_key_and_reload_uses_replacement_time_distance() {
    let mut app = loaded();
    assert_eq!(target(&app), PositionTarget::Top);
    assert_eq!(app.history.current, Some(0));
    assert_eq!(app.history.entries[0].path, PathBuf::from("a.md"));
    let old = app.position_request().unwrap().clone();

    // Different spelling but equal infra identity is a reload, not a navigation.
    let request = read(&mut app, DocumentOperation::Open("./A.md".into()));
    assert_eq!(app.position_request(), Some(&old));
    finish(&mut app, request, "canonical-a", 480.);
    assert_eq!(target(&app), PositionTarget::Distance(480.));
    assert_eq!(app.history.entries.len(), 1);
    assert!(!app.acknowledge_position(&old));

    let request = read(&mut app, DocumentOperation::Reload);
    finish(&mut app, request, "canonical-a", 720.);
    assert_eq!(target(&app), PositionTarget::Distance(720.));
    assert_eq!(app.history.entries.len(), 1);

    // Equal requested spelling is not sufficient to identify the resolved file.
    let request = read(&mut app, DocumentOperation::Open("./A.md".into()));
    finish(&mut app, request, "canonical-b", 960.);
    assert_eq!(target(&app), PositionTarget::Top);
    assert_eq!(app.history.current, Some(1));
    assert_eq!(app.history.entries[0].distance, 960.);
}

#[test]
fn headings_distinguish_synchronous_current_document_and_loaded_target() {
    let mut app = loaded();
    let generation = app.document().unwrap().generation;
    assert_eq!(
        app.operate(
            DocumentOperation::Heading {
                path: None,
                id: "使い方".into()
            },
            OperationOrigin::Link,
            120.,
            None,
        ),
        OperationResult::Complete {
            response_to: None,
            outcome: Ok(())
        }
    );
    assert!(!app.is_loading());
    assert_eq!(app.document().unwrap().generation, generation);
    assert_eq!(target(&app), PositionTarget::Heading("使い方".into()));
    assert_eq!(app.history.entries.len(), 2);
    assert_eq!(app.history.entries[0].distance, 120.);
    let first_heading = app.position_request().unwrap().clone();

    app.operate(
        DocumentOperation::Heading {
            path: None,
            id: "next".into(),
        },
        OperationOrigin::Link,
        180.,
        None,
    );
    assert!(!app.acknowledge_position(&first_heading));
    let request = read(
        &mut app,
        DocumentOperation::Heading {
            path: Some("b.md".into()),
            id: "elsewhere".into(),
        },
    );
    assert_eq!(app.history.entries.len(), 3);
    finish(&mut app, request, "canonical-b", 360.);
    assert_eq!(target(&app), PositionTarget::Heading("elsewhere".into()));
    assert_ne!(app.document().unwrap().generation, generation);
    assert_eq!(app.history.entries.len(), 4);
    assert_eq!(app.history.entries[2].distance, 360.);
}

#[test]
fn reload_before_heading_layout_preserves_intent_until_acknowledged() {
    for open_same_key in [false, true] {
        let mut app = loaded();
        app.operate(
            DocumentOperation::Heading {
                path: None,
                id: "使い方".into(),
            },
            OperationOrigin::Link,
            480.,
            None,
        );
        let heading = app.position_request().unwrap().clone();
        let history = app.history.entries.clone();
        let request = read(
            &mut app,
            if open_same_key {
                DocumentOperation::Open("./a.md".into())
            } else {
                DocumentOperation::Reload
            },
        );
        finish(&mut app, request, "canonical-a", 0.);
        assert_eq!(target(&app), PositionTarget::Heading("使い方".into()));
        assert_ne!(
            app.position_request().unwrap().generation,
            heading.generation
        );
        assert!(!app.acknowledge_position(&heading));
        assert_eq!(app.history.entries, history);
        let heading = app.position_request().unwrap().clone();
        app.acknowledge_position(&heading);
        let reload = read(&mut app, DocumentOperation::Reload);
        finish(&mut app, reload, "canonical-a", 640.);
        assert_eq!(target(&app), PositionTarget::Distance(640.));
    }
}

#[test]
fn failed_and_stale_heading_reads_preserve_document_history_and_pending_position() {
    let mut app = loaded();
    let history = app.history.entries.clone();
    let position = app.position_request().cloned();
    let old = read(
        &mut app,
        DocumentOperation::Heading {
            path: Some("old.md".into()),
            id: "古い".into(),
        },
    );
    let failed = read(
        &mut app,
        DocumentOperation::Heading {
            path: Some("missing.md".into()),
            id: "不明".into(),
        },
    );
    fail(&mut app, failed);
    assert!(!finish(&mut app, old, "old", 999.).adopted);
    assert_eq!(app.history.entries, history);
    assert_eq!(app.position_request(), position.as_ref());
    assert_eq!(app.document().unwrap().path, PathBuf::from("a.md"));
    assert_eq!(app.document().unwrap().source, "target edit target");
    assert_eq!(app.error(), Some("missing"));
}

#[test]
fn history_reads_saved_paths_restores_distances_and_truncates_only_on_new_success() {
    let mut app = loaded();
    assert_eq!(
        app.operate(DocumentOperation::Back, OperationOrigin::External, 0., None),
        OperationResult::Complete {
            response_to: None,
            outcome: Err(OperationError::NoPreviousEntry)
        }
    );
    let b = read(&mut app, DocumentOperation::Open("b.md".into()));
    finish(&mut app, b, "canonical-b", 480.);
    let back = read(&mut app, DocumentOperation::Back);
    assert_eq!(back.path, PathBuf::from("a.md"));
    assert_eq!(app.history.current, Some(1));
    finish(&mut app, back, "canonical-a", 600.);
    assert_eq!(app.history.current, Some(0));
    assert_eq!(target(&app), PositionTarget::Distance(480.));
    assert_eq!(app.history.entries[1].distance, 600.);

    let forward = read(&mut app, DocumentOperation::Forward);
    assert_eq!(forward.path, PathBuf::from("b.md"));
    finish(&mut app, forward, "canonical-b", 500.);
    assert_eq!(target(&app), PositionTarget::Distance(600.));
    let back = read(&mut app, DocumentOperation::Back);
    finish(&mut app, back, "canonical-a", 800.);
    assert_eq!(target(&app), PositionTarget::Distance(500.));

    let failed = read(&mut app, DocumentOperation::Open("missing.md".into()));
    fail(&mut app, failed);
    assert_eq!(app.history.entries.len(), 2);
    assert_eq!(app.history.current, Some(0));
    let reload = read(&mut app, DocumentOperation::Reload);
    finish(&mut app, reload, "canonical-a", 550.);
    assert_eq!(app.history.entries.len(), 2);
    let next = read(&mut app, DocumentOperation::Open("c.md".into()));
    assert_eq!(app.history.entries[1].path, PathBuf::from("b.md"));
    finish(&mut app, next, "canonical-c", 560.);
    assert_eq!(app.history.entries.len(), 2);
    assert_eq!(app.history.entries[1].path, PathBuf::from("c.md"));
    assert_eq!(app.history.entries[0].distance, 560.);
    assert_eq!(
        app.operate(
            DocumentOperation::Forward,
            OperationOrigin::External,
            0.,
            None
        ),
        OperationResult::Complete {
            response_to: None,
            outcome: Err(OperationError::NoNextEntry)
        }
    );
}

#[test]
fn failures_and_reversed_completions_preserve_history_document_and_position() {
    let mut app = loaded();
    let b = read(&mut app, DocumentOperation::Open("b.md".into()));
    finish(&mut app, b, "canonical-b", 100.);
    let history = app.history.entries.clone();
    let position = app.position_request().cloned();
    let generation = app.document().unwrap().generation;
    let older = read(&mut app, DocumentOperation::Open("older.md".into()));
    let back = read(&mut app, DocumentOperation::Back);
    assert!(!fail(&mut app, back).adopted);
    assert!(!finish(&mut app, older, "older", 999.).adopted);
    assert_eq!(app.history.entries, history);
    assert_eq!(app.history.current, Some(1));
    assert_eq!(app.position_request(), position.as_ref());
    assert_eq!(app.document().unwrap().generation, generation);
    assert_eq!(app.error(), Some("missing"));

    let older = read(&mut app, DocumentOperation::Back);
    let newer = read(&mut app, DocumentOperation::Open("c.md".into()));
    finish(&mut app, newer, "canonical-c", 200.);
    let history = app.history.entries.clone();
    let position = app.position_request().cloned();
    assert!(!finish(&mut app, older, "canonical-a", 999.).adopted);
    assert_eq!(app.history.entries, history);
    assert_eq!(app.history.current, Some(2));
    assert_eq!(app.position_request(), position.as_ref());
}

#[test]
fn synchronous_heading_supersedes_inflight_reads_but_completes_external_response() {
    for succeeds in [false, true] {
        let mut app = loaded();
        let external_id = ExternalRequestId("cli-open".into());
        let request = pending(app.operate(
            DocumentOperation::Open("b.md".into()),
            OperationOrigin::External,
            240.,
            Some(external_id.clone()),
        ));
        app.operate(
            DocumentOperation::Heading {
                path: None,
                id: "newer".into(),
            },
            OperationOrigin::Link,
            480.,
            None,
        );
        let history = app.history.entries.clone();
        let position = app.position_request().cloned();
        assert!(!app.is_loading());
        let completion = if succeeds {
            finish(&mut app, request.clone(), "canonical-b", 999.)
        } else {
            fail(&mut app, request.clone())
        };
        assert!(!completion.adopted);
        assert_eq!(completion.response_to, Some(external_id));
        assert_eq!(completion.outcome.is_ok(), succeeds);
        assert_eq!(app.history.entries, history);
        assert_eq!(app.history.current, Some(1));
        assert_eq!(app.position_request(), position.as_ref());
        assert_eq!(app.document().unwrap().path, PathBuf::from("a.md"));
        assert!(app.error().is_none());
        assert!(
            app.complete_at(
                ReadResult {
                    request,
                    content: Err("duplicate".into())
                },
                0.
            )
            .is_none()
        );
    }
}

#[test]
fn external_and_automatic_operations_keep_modes_and_search_updates_do_not_request_movement() {
    for mode in [Mode::Normal, Mode::Search, Mode::Command] {
        for origin in [OperationOrigin::External, OperationOrigin::Automatic] {
            let mut app = loaded();
            let generation = app.document().unwrap().generation;
            app.rendered(generation, "target edit target");
            app.begin_search();
            app.confirm_search("target".into(), 0);
            if mode == Mode::Search {
                app.begin_search();
                app.preview_search("edit".into());
            } else if mode == Mode::Command {
                app.begin_command();
            }
            let request = pending(app.operate(DocumentOperation::Reload, origin, 240., None));
            assert_eq!(request.origin, origin);
            assert_eq!(app.mode, mode);
            finish(&mut app, request, "canonical-a", 480.);
            let position = app.position_request().unwrap().clone();
            let generation = app.document().unwrap().generation;
            assert!(app.rendered(generation, "edit target edit"));
            assert_eq!(app.mode, mode);
            assert_eq!(app.position_request(), Some(&position));
            assert_eq!(position.target, PositionTarget::Distance(480.));
            if mode == Mode::Search {
                assert_eq!(app.matches(), &[0..4, 12..16]);
            } else {
                assert_eq!(app.matches().len(), 1);
                assert_eq!(app.matches()[0], 5..11);
            }
            assert!(app.acknowledge_position(&position));
            app.rendered(generation, "target edit");
            app.step_match(true);
            assert!(app.position_request().is_none());
            assert_eq!(app.history.entries.len(), 1);
        }
    }
}
