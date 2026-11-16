use std::{collections::HashSet, ops::Range, path::Path};

use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::{LinkTarget, resolve_link};

/// An identity in one document snapshot, never an index in the filtered list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CandidateId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    Heading,
    LocalMarkdown,
    LocalMarkdownHeading,
    Web,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkCandidate {
    pub id: CandidateId,
    pub display_name: String,
    pub destination: String,
    pub target: LinkTarget,
    pub kind: LinkKind,
    pub document_order: usize,
    /// UTF-8 source bytes in the same document generation as this analysis.
    pub source_range: Option<Range<usize>>,
}

/// The adapter supplies links in document order, not viewport order.
/// Resolution and deduplication do not access the filesystem.
pub fn candidates(
    document: &Path,
    links: impl IntoIterator<Item = (String, String, Option<Range<usize>>)>,
) -> Vec<LinkCandidate> {
    let mut seen = HashSet::new();
    links
        .into_iter()
        .enumerate()
        .filter_map(|(order, (display_name, source, source_range))| {
            let target = resolve_link(document, &source).ok()?;
            // A same-document fragment and an explicit file URL to that
            // fragment denote the same resolved destination.
            let identity = match &target {
                LinkTarget::Heading(id) => LinkTarget::File {
                    path: document.to_owned(),
                    fragment: Some(id.clone()),
                },
                target => target.clone(),
            };
            if !seen.insert((display_name.clone(), identity)) {
                return None;
            }
            let (destination, kind) = match &target {
                LinkTarget::Heading(id) => (format!("#{id}"), LinkKind::Heading),
                LinkTarget::File { path, fragment } => (
                    format!(
                        "{}{}",
                        path.display(),
                        fragment
                            .as_ref()
                            .map(|id| format!("#{id}"))
                            .unwrap_or_default()
                    ),
                    if fragment.is_some() {
                        LinkKind::LocalMarkdownHeading
                    } else {
                        LinkKind::LocalMarkdown
                    },
                ),
                LinkTarget::Web(url) => (url.clone(), LinkKind::Web),
            };
            Some(LinkCandidate {
                id: CandidateId(order),
                display_name,
                destination,
                target,
                kind,
                document_order: order,
                source_range,
            })
        })
        .collect()
}

/// Reuse the matcher's scratch allocation across query updates.
/// Queries are a single literal fuzzy subsequence, with no fzf syntax.
#[derive(Default)]
pub struct LinkMatcher {
    matcher: Matcher,
}

impl LinkMatcher {
    /// Literal, case-insensitive fuzzy matching with UTF-8 byte highlights.
    pub fn fuzzy(&mut self, query: &str, text: &str) -> Option<(u16, Vec<Range<usize>>)> {
        let needle: Vec<char> = query
            .chars()
            .map(nucleo_matcher::chars::to_lower_case)
            .collect();
        self.matcher.config = Config::DEFAULT;
        self.matcher.config.ignore_case = true;
        self.matcher.config.normalize = false;
        self.field(text, &needle)
    }

    fn field(&mut self, text: &str, needle: &[char]) -> Option<(u16, Vec<Range<usize>>)> {
        let chars: Vec<char> = text.chars().collect();
        let mut indices = Vec::new();
        let score = self.matcher.fuzzy_indices(
            Utf32Str::Unicode(&chars),
            Utf32Str::Unicode(needle),
            &mut indices,
        )?;
        let mut boundaries: Vec<_> = text.char_indices().map(|(offset, _)| offset).collect();
        boundaries.push(text.len());
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for index in indices {
            let index = index as usize;
            let range = boundaries[index]..boundaries[index + 1];
            if let Some(last) = ranges.last_mut()
                && last.end == range.start
            {
                last.end = range.end;
            } else {
                ranges.push(range);
            }
        }
        Some((score, ranges))
    }
}

#[cfg(test)]
mod tests;
