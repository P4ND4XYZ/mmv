use std::ops::Range;

use super::Generation;
use crate::core::find_matches;

#[derive(Debug, PartialEq, Eq)]
pub enum SearchOutcome {
    Found,
    NotFound,
    NoPreviousQuery,
}

#[derive(Default)]
pub(super) struct SearchState {
    confirmed_query: String,
    query: String,
    rendered: Option<(Generation, String)>,
    previous_match: Option<(Generation, Range<usize>)>,
    pub highlights_hidden: bool,
    previous_highlights_hidden: bool,
    pub matches: Vec<Range<usize>>,
    pub current_match: Option<usize>,
}

impl SearchState {
    pub fn replace_document(&mut self) {
        self.rendered = None;
        self.matches.clear();
        self.current_match = None;
    }

    pub fn begin_search(&mut self, generation: Option<Generation>) {
        let previous_match = generation.zip(
            self.current_match
                .and_then(|ix| self.matches.get(ix))
                .cloned(),
        );
        self.previous_match = previous_match;
        self.previous_highlights_hidden = self.highlights_hidden;
        self.highlights_hidden = false;
        self.set_query(String::new());
    }

    pub fn preview_search(&mut self, query: String) {
        self.set_query(query);
    }

    pub fn confirm_search(&mut self, query: String, from: usize) -> SearchOutcome {
        if !query.is_empty() {
            self.confirmed_query = query;
        }
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

    pub fn cancel_input(&mut self, generation: Option<Generation>) {
        self.highlights_hidden = self.previous_highlights_hidden;
        self.set_query(self.confirmed_query.clone());
        if let Some((previous_generation, range)) = self.previous_match.take()
            && generation == Some(previous_generation)
        {
            self.current_match = self.matches.iter().position(|found| *found == range);
        }
    }

    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.search();
    }

    pub fn rendered(&mut self, generation: Generation, text: &str) -> bool {
        if self
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
        self.highlights_hidden = false;
        let count = self.matches.len();
        self.current_match = Some(if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        });
    }
}
