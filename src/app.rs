use std::{collections::BTreeMap, ops::Range, path::PathBuf};

use crate::core::find_matches;
pub use crate::core::{ImageLocation, LinkTarget, resolve_image, resolve_link};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Generation(u64);

#[derive(Clone, Debug)]
pub struct ReadRequest {
    pub generation: Generation,
    pub path: PathBuf,
}

pub struct ReadResult {
    pub request: ReadRequest,
    pub content: Result<String, String>,
}

pub struct Document {
    pub generation: Generation,
    pub path: PathBuf,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagramRequest {
    pub generation: Generation,
    pub path: PathBuf,
    /// Full-document UTF-8 byte offset of the fenced code block.
    pub start: usize,
    pub source: String,
}

pub struct DiagramResult {
    pub request: DiagramRequest,
    pub png: Result<Vec<u8>, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagramStatus {
    Pending,
    Running,
    Ready,
    Failed,
}

pub struct Diagram {
    pub source: String,
    pub status: DiagramStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Normal,
    Search {
        previous_match: Option<(Generation, Range<usize>)>,
    },
    Command,
}

impl Mode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Search { .. } => "SEARCH",
            Self::Command => "COMMAND",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SearchOutcome {
    Found,
    NotFound,
    NoPreviousQuery,
}

#[derive(Default)]
pub struct App {
    next_generation: u64,
    pending: Option<ReadRequest>,
    pub document: Option<Document>,
    pub error: Option<String>,
    pub diagrams: BTreeMap<usize, Diagram>,
    // Kept across document replacement to serialize external processes.
    diagram_in_flight: Option<DiagramRequest>,
    pub mode: Mode,
    confirmed_query: String,
    query: String,
    rendered: Option<(Generation, String)>,
    pub matches: Vec<Range<usize>>,
    pub current_match: Option<usize>,
}

impl App {
    pub fn open(&mut self, path: PathBuf) -> ReadRequest {
        self.next_generation += 1;
        let request = ReadRequest {
            generation: Generation(self.next_generation),
            path,
        };
        self.pending = Some(request.clone());
        self.error = None;
        request
    }

    /// Returns true only when the current request successfully replaces the document.
    pub fn complete(&mut self, result: ReadResult) -> bool {
        if self.pending.as_ref().is_none_or(|pending| {
            pending.generation != result.request.generation || pending.path != result.request.path
        }) {
            return false;
        }
        self.pending = None;
        match result.content {
            Ok(source) => {
                self.document = Some(Document {
                    generation: result.request.generation,
                    path: result.request.path,
                    source,
                });
                self.diagrams.clear();
                self.rendered = None;
                self.matches.clear();
                self.current_match = None;
                true
            }
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    pub fn discover_diagrams(&mut self, sources: BTreeMap<usize, String>) {
        for (start, source) in sources {
            self.diagrams.entry(start).or_insert(Diagram {
                source,
                status: DiagramStatus::Pending,
            });
        }
    }

    pub fn next_diagram(&mut self) -> Option<DiagramRequest> {
        if self.diagram_in_flight.is_some() || self.is_loading() {
            return None;
        }
        let document = self.document.as_ref()?;
        let (&start, diagram) = self
            .diagrams
            .iter_mut()
            .find(|(_, diagram)| diagram.status == DiagramStatus::Pending)?;
        diagram.status = DiagramStatus::Running;
        let request = DiagramRequest {
            generation: document.generation,
            path: document.path.clone(),
            start,
            source: diagram.source.clone(),
        };
        self.diagram_in_flight = Some(request.clone());
        Some(request)
    }

    /// Releases the serial worker even for stale completions; only current results are accepted.
    pub fn complete_diagram(&mut self, result: &DiagramResult) -> bool {
        if self.diagram_in_flight.as_ref() != Some(&result.request) {
            return false;
        }
        self.diagram_in_flight = None;
        let Some(document) = &self.document else {
            return false;
        };
        if document.generation != result.request.generation || document.path != result.request.path
        {
            return false;
        }
        let Some(diagram) = self.diagrams.get_mut(&result.request.start) else {
            return false;
        };
        if diagram.source != result.request.source {
            return false;
        }
        diagram.status = if result.png.is_ok() {
            DiagramStatus::Ready
        } else {
            DiagramStatus::Failed
        };
        true
    }

    pub fn is_loading(&self) -> bool {
        self.pending.is_some()
    }

    pub fn begin_search(&mut self) {
        if self.mode != Mode::Normal {
            return;
        }
        let previous_match = self
            .current_match
            .and_then(|ix| self.matches.get(ix))
            .cloned()
            .and_then(|range| self.document.as_ref().map(|doc| (doc.generation, range)));
        self.mode = Mode::Search { previous_match };
        self.set_query(String::new());
    }

    pub fn preview_search(&mut self, query: String) {
        if matches!(self.mode, Mode::Search { .. }) {
            self.set_query(query);
        }
    }

    pub fn confirm_search(&mut self, query: String, from: usize) -> SearchOutcome {
        if !query.is_empty() {
            self.confirmed_query = query;
        }
        self.mode = Mode::Normal;
        self.set_query(self.confirmed_query.clone());
        if self.query.is_empty() {
            return SearchOutcome::NoPreviousQuery;
        }
        if self.matches.is_empty() {
            return SearchOutcome::NotFound;
        }
        self.current_match = Some(
            self.matches
                .iter()
                .position(|range| range.start >= from)
                .unwrap_or(0),
        );
        SearchOutcome::Found
    }

    pub fn begin_command(&mut self) {
        if self.mode == Mode::Normal {
            self.mode = Mode::Command;
        }
    }

    pub fn cancel_input(&mut self) {
        if let Mode::Search { previous_match } = std::mem::take(&mut self.mode) {
            self.set_query(self.confirmed_query.clone());
            if let Some((generation, range)) = previous_match
                && self
                    .document
                    .as_ref()
                    .is_some_and(|doc| doc.generation == generation)
            {
                self.current_match = self.matches.iter().position(|found| *found == range);
            }
        }
    }

    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.search();
    }

    pub fn rendered(&mut self, generation: Generation, text: &str) -> bool {
        if self
            .document
            .as_ref()
            .is_none_or(|doc| doc.generation != generation)
            || self
                .rendered
                .as_ref()
                .is_some_and(|(id, old)| *id == generation && old == text)
        {
            return false;
        }
        self.rendered = Some((generation, text.to_owned()));
        self.search();
        true
    }

    fn search(&mut self) {
        self.matches = self
            .rendered
            .as_ref()
            .map(|(_, text)| find_matches(text, &self.query))
            .unwrap_or_default();
        self.current_match = if self.matches.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Prototype condition: stepping wraps at either end.
    pub fn step_match(&mut self, forward: bool) {
        let Some(current) = self.current_match else {
            return;
        };
        let count = self.matches.len();
        self.current_match = Some(if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn success(request: ReadRequest, source: &str) -> ReadResult {
        ReadResult {
            request,
            content: Ok(source.into()),
        }
    }

    #[test]
    fn stale_success_and_failure_cannot_replace_the_latest_request() {
        let mut app = App::default();
        let first = app.open("first.md".into());
        let latest = app.open("latest.md".into());
        assert!(!app.complete(success(first.clone(), "old")));
        assert!(app.is_loading());
        assert!(app.complete(success(latest, "new")));
        assert!(!app.complete(ReadResult {
            request: first,
            content: Err("late failure".into())
        }));
        assert_eq!(app.document.as_ref().unwrap().source, "new");
        assert!(app.error.is_none());
    }

    #[test]
    fn reload_failure_keeps_the_displayed_document() {
        let mut app = App::default();
        let request = app.open("sample.md".into());
        app.complete(success(request.clone(), "retained"));
        let reload = app.open(request.path);
        assert!(!app.complete(ReadResult {
            request: reload,
            content: Err("unreadable".into())
        }));
        assert_eq!(app.document.as_ref().unwrap().source, "retained");
        assert_eq!(app.error.as_deref(), Some("unreadable"));
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
        assert_eq!(app.diagrams[&10].status, DiagramStatus::Failed);
        assert_eq!(app.document.as_ref().unwrap().source, "retained body");
        let second = app.next_diagram().unwrap();
        assert_eq!(second.start, 80);
        assert!(app.complete_diagram(&DiagramResult {
            request: second,
            png: Ok(vec![1]),
        }));
        app.discover_diagrams(BTreeMap::from([(10, "flowchart TD\nA --> B".into())]));
        assert!(app.next_diagram().is_none());
        assert_eq!(app.diagrams[&80].status, DiagramStatus::Ready);
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
            assert_eq!(app.diagrams[&10].status, DiagramStatus::Pending);
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
        let generation = request.generation;
        app.complete(success(request, "日本語 日本語"));
        app.rendered(generation, "日本語 日本語");
        app.begin_search();
        assert_eq!(
            app.confirm_search("日本語".into(), 10),
            SearchOutcome::Found
        );
        assert_eq!(app.current_match, Some(1));
        app.begin_search();
        app.preview_search("unknown".into());
        assert!(app.matches.is_empty());
        app.cancel_input();
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.matches, vec![0..9, 10..19]);
        assert_eq!(app.current_match, Some(1));
        app.begin_search();
        assert_eq!(app.confirm_search(String::new(), 20), SearchOutcome::Found);
        assert_eq!(app.current_match, Some(0));
    }

    #[test]
    fn cancel_does_not_restore_a_match_index_from_a_replaced_document() {
        let mut app = App::default();
        let first = app.open("a.md".into());
        app.complete(success(first.clone(), "x x"));
        app.rendered(first.generation, "x x");
        app.begin_search();
        app.confirm_search("x".into(), 2);
        assert_eq!(app.current_match, Some(1));
        app.begin_search();
        let second = app.open("b.md".into());
        app.complete(success(second.clone(), "x x"));
        app.rendered(second.generation, "x x");
        app.cancel_input();
        assert_eq!(app.current_match, Some(0));
    }

    #[test]
    fn search_rejects_old_rendered_text_and_resets_after_replacement() {
        let mut app = App::default();
        let first = app.open("a.md".into());
        app.complete(success(first.clone(), "日本語 日本語"));
        app.set_query("日本語".into());
        assert!(app.rendered(first.generation, "日本語 日本語"));
        assert_eq!(app.matches, vec![0..9, 10..19]);
        app.step_match(true);
        assert_eq!(app.current_match, Some(1));
        app.step_match(true);
        assert_eq!(app.current_match, Some(0));
        app.step_match(false);
        assert_eq!(app.current_match, Some(1));
        let second = app.open("b.md".into());
        app.complete(success(second.clone(), "other"));
        assert!(!app.rendered(first.generation, "日本語"));
        assert!(app.matches.is_empty());
        assert_eq!(app.current_match, None);
        assert!(app.rendered(second.generation, "other"));
        app.step_match(false);
        assert_eq!(app.current_match, None);
    }
}
