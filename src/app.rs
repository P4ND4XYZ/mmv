mod document;
mod history;
mod mermaid;
mod search;

#[cfg(test)]
use std::path::PathBuf;
use std::{collections::BTreeMap, ops::Range};

pub use crate::core::{ImageLocation, LinkTarget, resolve_image, resolve_link};
pub use document::{
    Document, DocumentKey, DocumentOperation, ExternalRequestId, Generation, LoadedDocument,
    OperationError, OperationOrigin, OperationResult, PositionRequest, PositionTarget,
    ReadCompletion, ReadRequest, ReadResult, WatchTarget,
};
#[cfg(test)]
pub use mermaid::{Diagram, DiagramStatus};
pub use mermaid::{DiagramRequest, DiagramResult};
pub use search::SearchOutcome;

use document::DocumentState;
use history::History;
use mermaid::MermaidState;
use search::SearchState;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    Search,
    Command,
}

impl Mode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Search => "SEARCH",
            Self::Command => "COMMAND",
        }
    }
}

#[derive(Default)]
pub struct App {
    document: DocumentState,
    search: SearchState,
    mermaid: MermaidState,
    history: History,
    pending_operation: Option<(document::ReadRequestId, PendingOperation)>,
    position: Option<PositionRequest>,
    next_position: u64,
    watch_target: Option<WatchTarget>,
    next_watch: u64,
    deferred_reload: bool,
    pub mode: Mode,
}

enum PendingOperation {
    Open,
    Reload,
    Heading(String),
    History { index: usize, distance: f32 },
}

impl App {
    pub fn document(&self) -> Option<&Document> {
        self.document.current.as_ref()
    }

    pub fn error(&self) -> Option<&str> {
        self.document.error.as_deref()
    }

    pub fn clear_error(&mut self) {
        self.document.clear_error();
    }

    pub fn matches(&self) -> &[Range<usize>] {
        &self.search.matches
    }

    pub fn current_match(&self) -> Option<usize> {
        self.search.current_match
    }

    #[cfg(test)]
    pub fn diagrams(&self) -> &BTreeMap<usize, Diagram> {
        &self.mermaid.diagrams
    }

    #[cfg(test)]
    pub fn open(&mut self, path: PathBuf) -> ReadRequest {
        self.open_request(path, None)
    }

    #[cfg(test)]
    pub fn open_request(
        &mut self,
        path: PathBuf,
        response_to: Option<ExternalRequestId>,
    ) -> ReadRequest {
        match self.operate(
            DocumentOperation::Open(path),
            OperationOrigin::External,
            0.,
            response_to,
        ) {
            OperationResult::Pending(request) => request,
            OperationResult::Complete { .. } => panic!("open always reads"),
        }
    }

    /// `distance` is supplied by UI, never inferred from source or layout in app.
    /// Pending reads and synchronous success/failure remain distinct at the boundary.
    /// Input mode and input ownership are deliberately untouched.
    pub fn operate(
        &mut self,
        operation: DocumentOperation,
        origin: OperationOrigin,
        distance: f32,
        response_to: Option<ExternalRequestId>,
    ) -> OperationResult {
        let complete = |outcome| OperationResult::Complete {
            response_to: response_to.clone(),
            outcome,
        };
        let (path, pending) = match operation {
            DocumentOperation::Initial(path) | DocumentOperation::Open(path) => {
                (path, PendingOperation::Open)
            }
            DocumentOperation::Reload => {
                let Some(document) = self.document() else {
                    return complete(Err(OperationError::DocumentUnavailable));
                };
                (document.path.clone(), PendingOperation::Reload)
            }
            DocumentOperation::Heading { path: None, id } => {
                let Some(document) = self.document() else {
                    return complete(Err(OperationError::DocumentUnavailable));
                };
                let path = document.path.clone();
                self.document.supersede();
                self.pending_operation = None;
                self.history.push(path, distance);
                self.request_position(PositionTarget::Heading(id));
                return complete(Ok(()));
            }
            DocumentOperation::Heading {
                path: Some(path),
                id,
            } => (path, PendingOperation::Heading(id)),
            DocumentOperation::Back | DocumentOperation::Forward => {
                let forward = matches!(operation, DocumentOperation::Forward);
                let Some((index, entry)) = self.history.target(forward) else {
                    return complete(Err(if forward {
                        OperationError::NoNextEntry
                    } else {
                        OperationError::NoPreviousEntry
                    }));
                };
                (
                    entry.path,
                    PendingOperation::History {
                        index,
                        distance: entry.distance,
                    },
                )
            }
        };
        let request = self.document.open(path, response_to, origin);
        self.pending_operation = Some((request.id, pending));
        OperationResult::Pending(request)
    }

    #[cfg(test)]
    pub fn complete(&mut self, result: ReadResult) -> Option<ReadCompletion> {
        self.complete_at(result, 0.)
    }

    /// Capture the departure distance immediately before replacing the displayed body.
    pub fn complete_at(&mut self, result: ReadResult, distance: f32) -> Option<ReadCompletion> {
        let previous_key = self.document().map(|document| document.key.clone());
        let completion = self.document.complete(result)?;
        if self
            .pending_operation
            .as_ref()
            .is_some_and(|(id, _)| *id == completion.request_id)
        {
            let (_, operation) = self.pending_operation.take().expect("matching operation");
            if completion.adopted {
                let document = self.document().expect("adopted document");
                let same_document = previous_key.as_ref() == Some(&document.key);
                let path = document.path.clone();
                let target = match operation {
                    PendingOperation::Open if same_document => PositionTarget::Distance(distance),
                    PendingOperation::Open => {
                        self.history.push(path, distance);
                        PositionTarget::Top
                    }
                    PendingOperation::Reload => PositionTarget::Distance(distance),
                    PendingOperation::Heading(id) => {
                        self.history.push(path, distance);
                        PositionTarget::Heading(id)
                    }
                    PendingOperation::History {
                        index,
                        distance: saved,
                    } => {
                        self.history.travel(index, distance);
                        PositionTarget::Distance(saved)
                    }
                };
                if self.watch_target.as_ref().map(|target| &target.key)
                    != self.document().map(|document| &document.key)
                {
                    self.next_watch += 1;
                    self.watch_target = Some(WatchTarget {
                        serial: self.next_watch,
                        key: self.document().expect("adopted document").key.clone(),
                    });
                    self.deferred_reload = false;
                }
                self.request_position(target);
                self.mermaid.replace_document();
                self.search.replace_document();
            }
        }
        Some(completion)
    }

    pub fn watch_target(&self) -> Option<&WatchTarget> {
        self.watch_target.as_ref()
    }

    pub fn watch_changed(&mut self, target: &WatchTarget) -> Option<ReadRequest> {
        if self.watch_target.as_ref() != Some(target) {
            return None;
        }
        self.deferred_reload = true;
        self.take_automatic_reload()
    }

    /// One trailing read covers all saves received while a read was pending.
    /// A successful switch discards the old file's deferred work; failure keeps it.
    pub fn take_automatic_reload(&mut self) -> Option<ReadRequest> {
        if !self.deferred_reload || self.is_loading() {
            return None;
        }
        self.deferred_reload = false;
        match self.operate(
            DocumentOperation::Reload,
            OperationOrigin::Automatic,
            0.,
            None,
        ) {
            OperationResult::Pending(request) => Some(request),
            OperationResult::Complete { .. } => None,
        }
    }

    fn request_position(&mut self, target: PositionTarget) {
        self.next_position += 1;
        self.position = Some(PositionRequest {
            generation: self
                .document()
                .expect("position requires document")
                .generation,
            target,
            serial: self.next_position,
        });
    }

    pub fn position_request(&self) -> Option<&PositionRequest> {
        self.position.as_ref()
    }

    /// A delayed consumer must not remove a newer generation or synchronous move.
    pub fn acknowledge_position(&mut self, request: &PositionRequest) -> bool {
        if self.position.as_ref() != Some(request) {
            return false;
        }
        self.position = None;
        true
    }

    pub fn is_loading(&self) -> bool {
        self.document.is_loading()
    }

    pub fn discover_diagrams(&mut self, sources: BTreeMap<usize, String>) {
        self.mermaid.discover_diagrams(sources);
    }

    pub fn next_diagram(&mut self) -> Option<DiagramRequest> {
        self.mermaid
            .next_diagram(self.document.current.as_ref(), self.is_loading())
    }

    pub fn complete_diagram(&mut self, result: &DiagramResult) -> bool {
        self.mermaid
            .complete_diagram(result, self.document.current.as_ref())
    }

    pub fn begin_search(&mut self) {
        if self.mode != Mode::Normal {
            return;
        }
        self.search
            .begin_search(self.document().map(|doc| doc.generation));
        self.mode = Mode::Search;
    }

    pub fn preview_search(&mut self, query: String) {
        if matches!(self.mode, Mode::Search) {
            self.search.preview_search(query);
        }
    }

    pub fn confirm_search(&mut self, query: String, from: usize) -> SearchOutcome {
        self.mode = Mode::Normal;
        self.search.confirm_search(query, from)
    }

    pub fn begin_command(&mut self) {
        if self.mode == Mode::Normal {
            self.mode = Mode::Command;
        }
    }

    pub fn cancel_input(&mut self) {
        if let Mode::Search = std::mem::take(&mut self.mode) {
            self.search
                .cancel_input(self.document().map(|doc| doc.generation));
        }
    }

    #[cfg(test)]
    pub fn set_query(&mut self, query: String) {
        self.search.set_query(query);
    }

    pub fn rendered(&mut self, generation: Generation, text: &str) -> bool {
        if self
            .document()
            .is_none_or(|doc| doc.generation != generation)
        {
            return false;
        }
        self.search.rendered(generation, text)
    }

    pub fn step_match(&mut self, forward: bool) {
        self.search.step_match(forward);
    }
}

#[cfg(test)]
mod operation_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn success(request: ReadRequest, source: &str) -> ReadResult {
        let key = DocumentKey(request.path.clone());
        ReadResult {
            request,
            content: Ok(LoadedDocument {
                key,
                source: source.into(),
            }),
        }
    }

    #[test]
    fn stale_success_and_failure_cannot_replace_the_latest_request() {
        let mut app = App::default();
        let first = app.open("first.md".into());
        let latest = app.open("latest.md".into());
        assert!(!app.complete(success(first.clone(), "old")).unwrap().adopted);
        assert!(app.is_loading());
        assert!(app.complete(success(latest, "new")).unwrap().adopted);
        assert!(
            app.complete(ReadResult {
                request: first,
                content: Err("late failure".into())
            })
            .is_none()
        );
        assert_eq!(app.document().unwrap().source, "new");
        assert!(app.error().is_none());

        let first = app.open("first.md".into());
        let latest = app.open("latest.md".into());
        assert!(
            !app.complete(ReadResult {
                request: latest,
                content: Err("latest failure".into()),
            })
            .unwrap()
            .adopted
        );
        assert!(
            !app.complete(success(first, "late success"))
                .unwrap()
                .adopted
        );
        assert_eq!(app.document().unwrap().path, PathBuf::from("latest.md"));
        assert_eq!(app.document().unwrap().source, "new");
        assert_eq!(app.error(), Some("latest failure"));
        assert!(!app.is_loading());
    }

    #[test]
    fn external_completions_keep_their_ids_and_results_when_finishing_in_reverse() {
        for a_succeeds in [false, true] {
            for b_succeeds in [false, true] {
                let mut app = with_confirmed_search();
                app.begin_search();
                app.preview_search("get".into());
                app.step_match(true);
                app.discover_diagrams(BTreeMap::from([(10, "diagram".into())]));
                let diagram = app.next_diagram().unwrap();
                let generation = app.document().unwrap().generation;
                let a_id = ExternalRequestId("cli-a".into());
                let b_id = ExternalRequestId("cli-b".into());
                let a = app.open_request("a.md".into(), Some(a_id.clone()));
                let b = app.open_request("b.md".into(), Some(b_id.clone()));
                let finish = |request: ReadRequest, succeeds| {
                    if succeeds {
                        success(request, "new body")
                    } else {
                        ReadResult {
                            request,
                            content: Err("read failed".into()),
                        }
                    }
                };
                let b_completion = app.complete(finish(b.clone(), b_succeeds)).unwrap();
                assert_eq!(b_completion.request_id, b.id);
                assert_eq!(b_completion.response_to, Some(b_id));
                assert_eq!(
                    b_completion.outcome,
                    if b_succeeds {
                        Ok(())
                    } else {
                        Err("read failed".into())
                    }
                );
                assert_eq!(b_completion.adopted, b_succeeds);
                let a_completion = app.complete(finish(a.clone(), a_succeeds)).unwrap();
                assert_eq!(a_completion.request_id, a.id);
                assert_eq!(a_completion.response_to, Some(a_id));
                assert_eq!(
                    a_completion.outcome,
                    if a_succeeds {
                        Ok(())
                    } else {
                        Err("read failed".into())
                    }
                );
                assert!(!a_completion.adopted);
                assert!(!app.is_loading());
                assert_eq!(app.mode, Mode::Search);
                if b_succeeds {
                    assert_eq!(app.document().unwrap().source, "new body");
                    assert_ne!(app.document().unwrap().generation, generation);
                    assert!(app.error().is_none());
                    assert!(app.matches().is_empty());
                    assert!(app.diagrams().is_empty());
                } else {
                    assert_eq!(app.document().unwrap().source, "target target");
                    assert_eq!(app.document().unwrap().generation, generation);
                    assert_eq!(app.error(), Some("read failed"));
                    assert_eq!(app.matches(), vec![3..6, 10..13]);
                    assert_eq!(app.current_match(), Some(1));
                    assert_eq!(app.diagrams()[&10].status, DiagramStatus::Running);
                    assert!(app.complete_diagram(&DiagramResult {
                        request: diagram,
                        png: Ok(vec![1])
                    }));
                }
                assert!(app.complete(finish(a, a_succeeds)).is_none());
                assert!(app.complete(finish(b, b_succeeds)).is_none());
            }
        }
    }

    #[test]
    fn duplicate_completion_cannot_replace_or_reset_the_current_document() {
        let mut app = with_confirmed_search();
        app.step_match(true);
        app.discover_diagrams(BTreeMap::from([(10, "diagram".into())]));
        let generation = app.document().unwrap().generation;
        let request = app.open("new.md".into());
        let mut altered = request.clone();
        altered.response_to = Some(ExternalRequestId("not-the-caller".into()));
        assert!(app.complete(success(altered, "wrong")).is_none());
        assert!(app.is_loading());
        assert!(
            app.complete(success(request.clone(), "new body"))
                .unwrap()
                .adopted
        );
        let adopted_generation = app.document().unwrap().generation;
        assert_ne!(adopted_generation, generation);
        app.rendered(adopted_generation, "target target");
        app.step_match(true);
        app.discover_diagrams(BTreeMap::from([(20, "new diagram".into())]));
        assert!(app.complete(success(request, "new body")).is_none());
        assert_eq!(app.document().unwrap().generation, adopted_generation);
        assert_eq!(app.current_match(), Some(1));
        assert_eq!(app.diagrams()[&20].status, DiagramStatus::Pending);
        assert!(!app.is_loading());
    }

    fn with_confirmed_search() -> App {
        let mut app = App::default();
        let request = app.open("sample.md".into());
        app.complete(success(request, "target target"));
        let generation = app.document().unwrap().generation;
        app.rendered(generation, "target target");
        app.begin_search();
        assert_eq!(app.confirm_search("target".into(), 0), SearchOutcome::Found);
        app
    }

    #[test]
    fn opening_another_file_keeps_the_document_and_search_until_completion() {
        let mut app = with_confirmed_search();
        app.step_match(true);
        app.open("other.md".into());
        let document = app.document().unwrap();
        assert_eq!(document.path, PathBuf::from("sample.md"));
        assert_eq!(document.source, "target target");
        assert_eq!(app.matches(), vec![0..6, 7..13]);
        assert_eq!(app.current_match(), Some(1));
        assert!(app.is_loading());
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // Expected match ranges, not indices.
    fn replacement_recomputes_the_confirmed_search_in_normal_and_command_modes() {
        for path in ["sample.md", "other.md"] {
            for mode in [Mode::Normal, Mode::Command] {
                let mut app = with_confirmed_search();
                if mode == Mode::Command {
                    app.begin_command();
                }
                let request = app.open(path.into());
                assert!(
                    app.complete(success(request, "prefix target"))
                        .unwrap()
                        .adopted
                );
                let generation = app.document().unwrap().generation;
                assert!(app.rendered(generation, "prefix target"));
                assert_eq!(app.mode, mode);
                assert_eq!(app.matches(), vec![7..13]);
                assert_eq!(app.current_match(), Some(0));
            }
        }
    }

    #[test]
    fn replacement_uses_the_search_preview_instead_of_the_confirmed_query() {
        let mut app = with_confirmed_search();
        app.begin_search();
        app.preview_search("edit".into());
        let mode = app.mode.clone();
        let request = app.open("sample.md".into());
        assert!(
            app.complete(success(request, "edit edit target"))
                .unwrap()
                .adopted
        );
        let generation = app.document().unwrap().generation;
        assert!(app.rendered(generation, "edit edit target"));
        assert_eq!(app.mode, mode);
        assert_eq!(app.matches(), vec![0..4, 5..9]);
    }

    #[test]
    fn replacement_keeps_an_empty_search_preview_empty() {
        let mut app = with_confirmed_search();
        app.begin_search();
        let mode = app.mode.clone();
        let request = app.open("sample.md".into());
        assert!(
            app.complete(success(request, "prefix target"))
                .unwrap()
                .adopted
        );
        let generation = app.document().unwrap().generation;
        assert!(app.rendered(generation, "prefix target"));
        assert_eq!(app.mode, mode);
        assert!(app.matches().is_empty());
        assert_eq!(app.current_match(), None);
        // Empty confirmation still reuses the confirmed query after replacement.
        assert_eq!(app.confirm_search(String::new(), 0), SearchOutcome::Found);
    }

    #[test]
    fn read_failure_keeps_the_input_mode_document_and_search() {
        for search in [false, true] {
            let mut app = with_confirmed_search();
            if search {
                app.begin_search();
                app.preview_search("get".into());
            } else {
                app.begin_command();
            }
            app.step_match(true);
            let mode = app.mode.clone();
            let matches = app.matches().to_vec();
            let request = app.open("missing.md".into());
            assert!(
                !app.complete(ReadResult {
                    request,
                    content: Err("unreadable".into()),
                })
                .unwrap()
                .adopted
            );
            assert_eq!(app.mode, mode);
            assert_eq!(app.document().unwrap().path, PathBuf::from("sample.md"));
            assert_eq!(app.document().unwrap().source, "target target");
            assert_eq!(app.matches(), matches);
            assert_eq!(app.current_match(), Some(1));
            assert_eq!(app.error(), Some("unreadable"));
            assert!(!app.is_loading());
            if search {
                // Recompute on a new snapshot to observe preservation of the preview query.
                let generation = app.document().unwrap().generation;
                app.rendered(generation, "get get target");
                assert_eq!(app.matches(), vec![0..3, 4..7, 11..14]);
            }
        }
    }

    #[test]
    fn repeated_rendered_snapshot_keeps_the_selected_match() {
        let mut app = with_confirmed_search();
        app.step_match(true);
        let generation = app.document().unwrap().generation;
        assert!(!app.rendered(generation, "target target"));
        assert_eq!(app.matches(), vec![0..6, 7..13]);
        assert_eq!(app.current_match(), Some(1));
    }

    #[test]
    fn reload_failure_keeps_the_displayed_document() {
        let mut app = App::default();
        let request = app.open("sample.md".into());
        app.complete(success(request.clone(), "retained"));
        let reload = app.open(request.path);
        assert!(
            !app.complete(ReadResult {
                request: reload,
                content: Err("unreadable".into())
            })
            .unwrap()
            .adopted
        );
        assert_eq!(app.document().unwrap().source, "retained");
        assert_eq!(app.error(), Some("unreadable"));
        assert!(!app.is_loading());
    }

    #[test]
    fn diagrams_run_serially_and_failure_does_not_hide_the_document_or_retry() {
        let mut app = App::default();
        let read = app.open("sample.md".into());
        app.complete(success(read, "retained body"));
        app.discover_diagrams(BTreeMap::from([
            (10, "flowchart TD\nA --> B".into()),
            (80, "classDiagram\nA --> B".into()),
        ]));
        let first = app.next_diagram().unwrap();
        assert_eq!(first.start, 10);
        assert!(app.next_diagram().is_none());
        assert!(app.complete_diagram(&DiagramResult {
            request: first,
            png: Err("syntax error".into()),
        }));
        assert_eq!(app.diagrams()[&10].status, DiagramStatus::Failed);
        assert_eq!(app.document().unwrap().source, "retained body");
        let second = app.next_diagram().unwrap();
        assert_eq!(second.start, 80);
        assert!(app.complete_diagram(&DiagramResult {
            request: second,
            png: Ok(vec![1]),
        }));
        app.discover_diagrams(BTreeMap::from([(10, "flowchart TD\nA --> B".into())]));
        assert!(app.next_diagram().is_none());
        assert_eq!(app.diagrams()[&80].status, DiagramStatus::Ready);
    }

    #[test]
    fn old_diagram_results_release_the_worker_without_touching_the_new_document() {
        for png in [Ok(vec![1]), Err("late failure".into())] {
            let mut app = App::default();
            let first = app.open("sample.md".into());
            app.complete(success(first, "old body"));
            app.discover_diagrams(BTreeMap::from([(10, "old diagram".into())]));
            let old = app.next_diagram().unwrap();
            let reload = app.open("sample.md".into());
            app.complete(success(reload, "new body"));
            app.discover_diagrams(BTreeMap::from([(10, "new diagram".into())]));
            assert!(app.next_diagram().is_none());
            assert!(!app.complete_diagram(&DiagramResult { request: old, png }));
            assert_eq!(app.diagrams()[&10].status, DiagramStatus::Pending);
            let current = app.next_diagram().unwrap();
            assert_eq!(current.source, "new diagram");
            let mut wrong = current.clone();
            wrong.source = "wrong source".into();
            assert!(!app.complete_diagram(&DiagramResult {
                request: wrong,
                png: Ok(vec![2]),
            }));
            assert!(app.next_diagram().is_none());
            assert!(app.complete_diagram(&DiagramResult {
                request: current,
                png: Ok(vec![3]),
            }));
        }
    }

    #[test]
    fn search_preview_cancel_and_empty_commit_keep_the_confirmed_query() {
        let mut app = App::default();
        let request = app.open("sample.md".into());
        app.complete(success(request, "日本語 日本語"));
        let generation = app.document().unwrap().generation;
        app.rendered(generation, "日本語 日本語");
        app.begin_search();
        assert_eq!(
            app.confirm_search("日本語".into(), 10),
            SearchOutcome::Found
        );
        assert_eq!(app.current_match(), Some(1));
        app.begin_search();
        app.preview_search("unknown".into());
        assert!(app.matches().is_empty());
        app.cancel_input();
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.matches(), vec![0..9, 10..19]);
        assert_eq!(app.current_match(), Some(1));
        app.begin_search();
        assert_eq!(app.confirm_search(String::new(), 20), SearchOutcome::Found);
        assert_eq!(app.current_match(), Some(0));
    }

    #[test]
    fn cancel_does_not_restore_a_match_index_from_a_replaced_document() {
        let mut app = App::default();
        let first = app.open("a.md".into());
        app.complete(success(first.clone(), "x x"));
        app.rendered(app.document().unwrap().generation, "x x");
        app.begin_search();
        app.confirm_search("x".into(), 2);
        assert_eq!(app.current_match(), Some(1));
        app.begin_search();
        let second = app.open("b.md".into());
        app.complete(success(second.clone(), "x x"));
        app.rendered(app.document().unwrap().generation, "x x");
        app.cancel_input();
        assert_eq!(app.current_match(), Some(0));
    }

    #[test]
    fn search_rejects_old_rendered_text_and_resets_after_replacement() {
        let mut app = App::default();
        let first = app.open("a.md".into());
        app.complete(success(first.clone(), "日本語 日本語"));
        let first_generation = app.document().unwrap().generation;
        app.set_query("日本語".into());
        assert!(app.rendered(first_generation, "日本語 日本語"));
        assert_eq!(app.matches(), vec![0..9, 10..19]);
        app.step_match(true);
        assert_eq!(app.current_match(), Some(1));
        app.step_match(true);
        assert_eq!(app.current_match(), Some(0));
        app.step_match(false);
        assert_eq!(app.current_match(), Some(1));
        let second = app.open("b.md".into());
        app.complete(success(second.clone(), "other"));
        assert!(!app.rendered(first_generation, "日本語"));
        assert!(app.matches().is_empty());
        assert_eq!(app.current_match(), None);
        assert!(app.rendered(app.document().unwrap().generation, "other"));
        app.step_match(false);
        assert_eq!(app.current_match(), None);
    }
}
