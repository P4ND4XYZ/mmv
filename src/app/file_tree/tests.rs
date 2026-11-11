use super::*;

#[test]
fn only_normal_markdown_files_are_open_candidates() {
    let mut tree = FileTree::default();
    let root = tree
        .document_changed("/tree/a.md".into(), true, &[])
        .remove(0);
    let Request::Root(root) = root else { panic!() };
    let listing = directory(tree.complete(Completion::Root(root, "/tree".into())));
    let mut reparse = entry("/tree", "link.md", false);
    reparse.reparse = true;
    tree.complete(Completion::Directory(
        listing,
        Ok(vec![
            entry("/tree", "folder", true),
            entry("/tree", "a.MD", false),
            entry("/tree", "asset.txt", false),
            reparse,
        ]),
    ));
    for (name, allowed) in [
        ("folder", false),
        ("a.MD", true),
        ("asset.txt", false),
        ("link.md", false),
    ] {
        tree.cursor = Some(PathBuf::from("/tree").join(name));
        assert_eq!(tree.open_candidate().is_some(), allowed);
    }
}

fn entry(parent: &str, name: &str, directory: bool) -> Entry {
    Entry {
        path: PathBuf::from(parent).join(name),
        name: name.into(),
        directory,
        expandable: directory && name != ".git",
        reparse: false,
    }
}

fn directory(requests: Vec<Request>) -> DirectoryRequest {
    assert_eq!(requests.len(), 1);
    match requests.into_iter().next().unwrap() {
        Request::Directory(request) => request,
        _ => panic!("expected listing"),
    }
}

fn root(requests: Vec<Request>) -> RootRequest {
    assert_eq!(requests.len(), 1);
    match requests.into_iter().next().unwrap() {
        Request::Root(request) => request,
        _ => panic!("expected root discovery"),
    }
}

fn initialized() -> (FileTree, DirectoryRequest) {
    let mut tree = FileTree::default();
    let request = root(tree.document_changed("/repo/file.md".into(), true, &[]));
    let listing = directory(tree.complete(Completion::Root(request, "/repo".into())));
    (tree, listing)
}

fn loaded() -> FileTree {
    let (mut tree, listing) = initialized();
    tree.complete(Completion::Directory(
        listing,
        Ok(vec![
            entry("/repo", "a", true),
            entry("/repo", "b", false),
            entry("/repo", "c", false),
        ]),
    ));
    tree
}

#[test]
fn hidden_initial_fetch_is_retained_and_show_does_not_duplicate_it() {
    let (mut tree, listing) = initialized();
    // Hiding has no state transition; showing neither cancels nor duplicates.
    assert!(tree.show(&[]).is_empty());
    assert!(tree.act(Action::Refresh).is_empty());
    tree.complete(Completion::Directory(
        listing.clone(),
        Ok(vec![entry("/repo", "a", true)]),
    ));
    assert!(tree.show(&[]).is_empty());
    assert_eq!(tree.rows().len(), 2);
    tree.complete(Completion::Directory(listing, Ok(Vec::new())));
    assert_eq!(
        tree.rows().len(),
        2,
        "a repeated completion cannot replace the listing"
    );
}

#[test]
fn hidden_document_changes_defer_discovery_and_same_root_keeps_state() {
    let mut tree = loaded();
    tree.act(Action::Next);
    let child = directory(tree.act(Action::Right));
    let cursor = tree.cursor.clone();
    assert!(
        tree.document_changed("/repo/other.md".into(), false, &[])
            .is_empty()
    );
    assert!(tree.resolving_root());
    tree.complete(Completion::Directory(
        child,
        Ok(vec![entry("/repo/a", "leaf", false)]),
    ));
    assert!(tree.act(Action::Next).is_empty());
    assert_eq!(tree.cursor, cursor);
    let probe = root(tree.show(&["marker".into()]));
    assert_eq!(probe.markers, ["marker"]);
    assert!(tree.show(&[]).is_empty());
    assert!(
        tree.complete(Completion::Root(probe, "/repo".into()))
            .is_empty()
    );
    assert_eq!(tree.cursor, cursor);
    assert_eq!(tree.rows().len(), 5);
    assert!(tree.node(Path::new("/repo/a")).unwrap().expanded);
}

#[test]
fn newest_discovery_wins_and_a_b_a_rejects_old_listing() {
    let (mut tree, old_a) = initialized();
    let old_probe = root(tree.document_changed("/old/file.md".into(), true, &[]));
    let probe_b = root(tree.document_changed("/b/file.md".into(), true, &[]));
    assert!(
        tree.complete(Completion::Root(old_probe, "/old".into()))
            .is_empty()
    );
    let b = directory(tree.complete(Completion::Root(probe_b, "/b".into())));
    let probe_a = root(tree.document_changed("/repo/again.md".into(), true, &[]));
    let current_a = directory(tree.complete(Completion::Root(probe_a, "/repo".into())));
    tree.complete(Completion::Directory(
        old_a,
        Ok(vec![entry("/repo", "stale", true)]),
    ));
    tree.complete(Completion::Directory(
        b,
        Ok(vec![entry("/b", "stale", true)]),
    ));
    assert_eq!(tree.rows(), vec![(PathBuf::from("/repo"), 0)]);
    tree.complete(Completion::Directory(
        current_a,
        Ok(vec![entry("/repo", "new", false)]),
    ));
    assert_eq!(tree.rows().len(), 2);
    assert_eq!(tree.cursor, Some("/repo".into()));
}

#[test]
fn refresh_only_fetches_root_and_expanded_directories_and_merges_duplicates() {
    let mut tree = loaded();
    tree.act(Action::Next);
    let child = directory(tree.act(Action::Right));
    assert!(tree.act(Action::Right).is_empty());
    let refresh = tree.act(Action::Refresh);
    assert_eq!(refresh.len(), 1, "expanded pending child is merged");
    assert_eq!(directory(refresh).path, PathBuf::from("/repo"));
    tree.complete(Completion::Directory(
        child,
        Ok(vec![entry("/repo/a", "nested", true)]),
    ));
    assert!(tree.act(Action::Refresh).iter().all(|request| {
        matches!(request, Request::Directory(request) if request.path == Path::new("/repo/a"))
    }));
    tree.act(Action::Left);
    assert_eq!(tree.rows().len(), 4);
    assert!(
        tree.act(Action::Refresh).is_empty(),
        "root listing is still in flight"
    );
}

#[test]
fn collapse_reexpand_and_parent_deletion_discard_inflight_descendants() {
    let mut tree = loaded();
    tree.act(Action::Next);
    let first = directory(tree.act(Action::Right));
    tree.act(Action::Left);
    let second = directory(tree.act(Action::Right));
    tree.complete(Completion::Directory(
        first,
        Ok(vec![entry("/repo/a", "old", false)]),
    ));
    assert_eq!(tree.rows().len(), 4);
    tree.complete(Completion::Directory(
        second,
        Ok(vec![entry("/repo/a", "new", true)]),
    ));
    tree.act(Action::Right);
    let nested = directory(tree.act(Action::Right));
    let refresh = tree.act(Action::Refresh);
    let parent = refresh
        .into_iter()
        .find_map(|r| match r {
            Request::Directory(r) if r.path == Path::new("/repo") => Some(r),
            _ => None,
        })
        .unwrap();
    tree.complete(Completion::Directory(
        parent,
        Ok(vec![entry("/repo", "b", false)]),
    ));
    tree.complete(Completion::Directory(
        nested,
        Ok(vec![entry("/repo/a/new", "late", false)]),
    ));
    assert_eq!(tree.rows().len(), 2);
    assert!(tree.node(Path::new("/repo/a/new")).is_none());
    assert_eq!(
        tree.cursor, tree.root,
        "a deleted parent falls back to the nearest ancestor, not its sibling"
    );
}

#[test]
fn deleted_cursor_uses_old_successor_then_predecessor_then_parent_not_new_row() {
    for directory_kind in [false, true] {
        for (remaining, expected) in [
            (vec!["aa", "c", "d"], "/repo/c"),
            (vec!["aa", "a"], "/repo/a"),
            (vec!["aa"], "/repo"),
        ] {
            let (mut tree, listing) = initialized();
            tree.complete(Completion::Directory(
                listing,
                Ok(["a", "b", "c", "d"]
                    .map(|n| entry("/repo", n, directory_kind))
                    .to_vec()),
            ));
            tree.act(Action::Next);
            tree.act(Action::Next);
            let request = directory(tree.act(Action::Refresh));
            tree.complete(Completion::Directory(
                request,
                Ok(remaining
                    .into_iter()
                    .map(|n| entry("/repo", n, directory_kind))
                    .collect()),
            ));
            assert_eq!(tree.cursor, Some(expected.into()));
        }
    }
}

#[test]
fn failures_keep_branch_listing_and_cursor_but_root_failure_shows_only_root() {
    let mut tree = loaded();
    tree.act(Action::Next);
    let child = directory(tree.act(Action::Right));
    tree.complete(Completion::Directory(
        child,
        Ok(vec![entry("/repo/a", "leaf", false)]),
    ));
    tree.act(Action::Right);
    let cursor = tree.cursor.clone();
    let requests = tree.act(Action::Refresh);
    let mut root_request = None;
    for request in requests {
        let Request::Directory(request) = request else {
            panic!()
        };
        if request.path == Path::new("/repo/a") {
            tree.complete(Completion::Directory(request, Err(())));
        } else {
            root_request = Some(request);
        }
    }
    assert_eq!(tree.cursor, cursor);
    assert_eq!(tree.rows().len(), 5);
    assert!(tree.node(Path::new("/repo/a")).unwrap().error);
    tree.complete(Completion::Directory(root_request.unwrap(), Err(())));
    assert_eq!(tree.cursor, tree.root);
    assert_eq!(tree.rows(), vec![(PathBuf::from("/repo"), 0)]);
    for action in [Action::Right, Action::Toggle, Action::Next, Action::Last] {
        assert!(tree.act(action).is_empty());
        assert_eq!(tree.cursor, tree.root);
    }
    let retry = directory(tree.act(Action::Refresh));
    tree.complete(Completion::Directory(
        retry,
        Ok(vec![entry("/repo", "a", true)]),
    ));
    assert!(!tree.node(Path::new("/repo")).unwrap().error);
    assert_eq!(
        tree.rows().len(),
        3,
        "successful retry restores the retained expanded branch"
    );
}

#[test]
fn a_surviving_cursor_is_kept_when_new_siblings_change_its_row() {
    let mut tree = loaded();
    tree.act(Action::Last);
    let cursor = tree.cursor.clone();
    let request = directory(tree.act(Action::Refresh));
    tree.complete(Completion::Directory(
        request,
        Ok(vec![
            entry("/repo", "aa", false),
            entry("/repo", "c", false),
        ]),
    ));
    assert_eq!(tree.cursor, cursor);
    assert_eq!(
        tree.rows()
            .iter()
            .position(|(path, _)| Some(path) == cursor.as_ref()),
        Some(2)
    );
}

#[test]
fn deleted_then_recreated_parent_does_not_accept_the_old_child_completion() {
    let mut tree = loaded();
    tree.act(Action::Next);
    let old = directory(tree.act(Action::Right));
    let parent = directory(tree.act(Action::Refresh));
    tree.complete(Completion::Directory(parent, Ok(Vec::new())));
    let parent = directory(tree.act(Action::Refresh));
    tree.complete(Completion::Directory(
        parent,
        Ok(vec![entry("/repo", "a", true)]),
    ));
    tree.act(Action::Next);
    let new = directory(tree.act(Action::Right));
    tree.complete(Completion::Directory(
        old,
        Ok(vec![entry("/repo/a", "stale", false)]),
    ));
    assert_eq!(tree.rows().len(), 2);
    tree.complete(Completion::Directory(
        new,
        Ok(vec![entry("/repo/a", "current", false)]),
    ));
    assert_eq!(
        tree.rows().last().unwrap().0,
        PathBuf::from("/repo/a/current")
    );
}

#[test]
fn navigation_files_do_not_expand_and_boundaries_are_clamped() {
    let mut tree = loaded();
    tree.act(Action::Previous);
    assert_eq!(tree.cursor, tree.root);
    tree.act(Action::Last);
    assert_eq!(tree.cursor, Some("/repo/c".into()));
    assert!(tree.act(Action::Toggle).is_empty());
    tree.act(Action::Next);
    assert_eq!(tree.cursor, Some("/repo/c".into()));
    tree.act(Action::Left);
    assert_eq!(tree.cursor, tree.root);
    tree.act(Action::Right);
    assert_eq!(tree.cursor, Some("/repo/a".into()));
    tree.act(Action::First);
    assert_eq!(tree.cursor, tree.root);
}
