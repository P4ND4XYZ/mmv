use std::{ops::Range, path::PathBuf};

use crate::core::find_matches;
pub use crate::core::{ImageLocation, resolve_image};

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

#[derive(Default)]
pub struct App {
    next_generation: u64,
    pending: Option<ReadRequest>,
    pub document: Option<Document>,
    pub error: Option<String>,
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

    pub fn is_loading(&self) -> bool {
        self.pending.is_some()
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
