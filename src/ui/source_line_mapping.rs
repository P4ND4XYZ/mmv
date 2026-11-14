//! Pure source-to-target projection for the committed stable 0.9 text snapshot.
//!
//! Callers must parse `blocks` once from exactly `snapshot.source()` with the
//! snapshot's Markdown extensions, and invalidate them on source/extension changes.
//! Stable 0.9 exposes no AST revision token: this precondition cannot be proved
//! from spans alone. The projection additionally checks the rendered leaf sequence.

use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU64,
    ops::Range,
};

use gpui_kit::base::text::{
    RenderedText,
    editor_host::{BlockNode, CodeBlock, Paragraph},
};

use crate::core::source_position::SourceLines;

#[derive(Clone, Debug)]
pub(crate) struct MappedSourceLine {
    snapshot: RenderedText,
    target: MappedLineTarget,
}

impl MappedSourceLine {
    pub(crate) fn snapshot(&self) -> &RenderedText {
        &self.snapshot
    }
    pub(crate) fn target(&self) -> &MappedLineTarget {
        &self.target
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MappedLineTarget {
    RenderedPosition(usize),
    CodePosition {
        source_span: Range<usize>,
        offset: usize,
    },
    EmptyCode {
        source_span: Range<usize>,
    },
    Noop,
    PastEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceLineMapError {
    StaleSource,
    UnmappedText,
    OpaqueMedia,
    InvalidStructure,
}

pub(crate) fn map_source_line(
    lines: &SourceLines,
    blocks: &[BlockNode],
    snapshot: &RenderedText,
    line: NonZeroU64,
) -> Result<MappedSourceLine, SourceLineMapError> {
    if !lines.matches_source(snapshot.source()) {
        return Err(SourceLineMapError::StaleSource);
    }
    let target = if lines.range(line).is_none() {
        MappedLineTarget::PastEnd
    } else {
        let mut projection = Projection {
            lines,
            snapshot,
            cursor: 0,
            body: BTreeMap::new(),
            fences: BTreeMap::new(),
            opaque: BTreeSet::new(),
        };
        for block in blocks {
            projection.block(block)?;
        }
        if projection.cursor != snapshot.as_str().len() {
            return Err(SourceLineMapError::InvalidStructure);
        }
        let wanted = (line.get() - 1) as usize;
        if let Some(target) = projection.fences.get(&wanted) {
            target.clone()
        } else if let Some(target) = projection.body.get(&wanted) {
            target.clone()
        } else if projection.opaque.contains(&wanted) {
            return Err(SourceLineMapError::OpaqueMedia);
        } else if let Some((_, target)) = projection
            .body
            .iter()
            .min_by_key(|(row, _)| (row.abs_diff(wanted), std::cmp::Reverse(**row)))
        {
            target.clone()
        } else if snapshot.source().trim().is_empty() {
            MappedLineTarget::Noop
        } else {
            return Err(SourceLineMapError::UnmappedText);
        }
    };
    Ok(MappedSourceLine {
        snapshot: snapshot.clone(),
        target,
    })
}

struct Projection<'a> {
    lines: &'a SourceLines,
    snapshot: &'a RenderedText,
    cursor: usize,
    body: BTreeMap<usize, MappedLineTarget>,
    fences: BTreeMap<usize, MappedLineTarget>,
    opaque: BTreeSet<usize>,
}

impl Projection<'_> {
    fn rows(&self, span: Range<usize>) -> Result<Range<usize>, SourceLineMapError> {
        if span.start > span.end || self.snapshot.source().get(span.clone()).is_none() {
            return Err(SourceLineMapError::InvalidStructure);
        }
        let first = self
            .lines
            .lines
            .partition_point(|row| row.start <= span.start)
            .saturating_sub(1);
        let end = self
            .lines
            .lines
            .partition_point(|row| row.start < span.end)
            .max(first + 1);
        if end > self.lines.logical_count() {
            return Err(SourceLineMapError::InvalidStructure);
        }
        Ok(first..end)
    }

    fn advance(&mut self, text: &str) -> Result<Range<usize>, SourceLineMapError> {
        let end = self
            .cursor
            .checked_add(text.len())
            .ok_or(SourceLineMapError::InvalidStructure)?;
        if self.snapshot.as_str().get(self.cursor..end) != Some(text) {
            return Err(SourceLineMapError::InvalidStructure);
        }
        let range = self.cursor..end;
        self.cursor = end;
        Ok(range)
    }

    fn block(&mut self, block: &BlockNode) -> Result<(), SourceLineMapError> {
        self.project_block(block, None)
    }

    fn project_block(
        &mut self,
        block: &BlockNode,
        inherited_opaque: Option<Range<usize>>,
    ) -> Result<(), SourceLineMapError> {
        let start = self.cursor;
        let span = block.span().map(|span| span.start..span.end);
        // The Markdown parser wraps HTML in a source-spanned Root, while its
        // HTML-derived leaves legitimately have no source positions. Serialize
        // that subtree exactly, but never invent source projections for it.
        let opaque = inherited_opaque.or_else(|| match block {
            BlockNode::Root { children, .. }
                if children.iter().any(|child| child.span().is_none()) =>
            {
                span.clone()
            }
            _ => None,
        });
        if let Some(span) = &opaque {
            self.opaque.extend(self.rows(span.clone())?);
        }
        let span = span.or_else(|| opaque.clone());
        match block {
            BlockNode::Root { children, .. }
            | BlockNode::Blockquote { children, .. }
            | BlockNode::List { children, .. }
            | BlockNode::ListItem { children, .. } => {
                for child in children {
                    self.project_block(child, opaque.clone())?;
                }
                if matches!(block, BlockNode::List { .. } | BlockNode::ListItem { .. }) {
                    return Ok(());
                }
            }
            BlockNode::Paragraph(paragraph)
            | BlockNode::Heading {
                children: paragraph,
                ..
            } => {
                if opaque.is_some() {
                    self.serialize_paragraph(paragraph)?;
                } else {
                    self.paragraph(paragraph, span.ok_or(SourceLineMapError::InvalidStructure)?)?;
                }
            }
            BlockNode::Table(table) => {
                let span = span.ok_or(SourceLineMapError::InvalidStructure)?;
                let source_rows = self.rows(span)?;
                // GFM table rows each occupy one physical line; the delimiter
                // follows the header but has no AST row or body candidate.
                if opaque.is_none() && source_rows.len() != table.rows().count() + 1 {
                    return Err(SourceLineMapError::InvalidStructure);
                }
                for (ordinal, row) in table.rows().enumerate() {
                    let source_row = source_rows.start + ordinal + usize::from(ordinal > 0);
                    let row_span = if opaque.is_none() {
                        Some(self.lines.lines[source_row].clone())
                    } else {
                        None
                    };
                    let mut any = false;
                    for cell in row {
                        if any {
                            self.advance(" ")?;
                        }
                        if let Some(span) = &row_span {
                            self.paragraph(cell, span.clone())?;
                        } else {
                            self.serialize_paragraph(cell)?;
                        }
                        any = true;
                    }
                    if any {
                        self.advance("\n")?;
                    }
                }
            }
            BlockNode::CodeBlock(code) => {
                if opaque.is_some() {
                    self.advance(&code.code())?;
                } else {
                    self.code(code, span.ok_or(SourceLineMapError::InvalidStructure)?)?;
                }
            }
            BlockNode::Custom { node, .. } => {
                self.opaque
                    .extend(self.rows(span.ok_or(SourceLineMapError::InvalidStructure)?)?);
                self.advance(node.as_text())?;
            }
            BlockNode::Definition { .. }
            | BlockNode::Break { .. }
            | BlockNode::HorizontalRule { .. } => {}
            _ => return Err(SourceLineMapError::InvalidStructure),
        }
        if self.cursor > start {
            self.advance("\n")?;
        }
        Ok(())
    }

    fn serialize_paragraph(&mut self, paragraph: &Paragraph) -> Result<(), SourceLineMapError> {
        for child in paragraph.children() {
            self.advance(child.text())?;
        }
        Ok(())
    }

    fn paragraph(
        &mut self,
        paragraph: &Paragraph,
        span: Range<usize>,
    ) -> Result<(), SourceLineMapError> {
        let mut text_runs = Vec::new();
        let mut opaque = false;
        for child in paragraph.children() {
            let range = self.advance(child.text())?;
            if child.as_image().is_some() || child.as_custom().is_some() {
                opaque = true;
            } else if !range.is_empty() {
                text_runs.push(range);
            }
        }
        for row in self.rows(span.clone())? {
            let source_row = &self.lines.lines[row];
            let source = source_row.start.max(span.start)..source_row.end.min(span.end);
            let projected = self.snapshot.range_for_source(source);
            let position = projected.and_then(|projected| {
                text_runs.iter().find_map(|run| {
                    let start = projected.start.max(run.start);
                    let end = projected.end.min(run.end);
                    self.snapshot
                        .as_str()
                        .get(start..end)?
                        .char_indices()
                        .find(|(_, character)| !character.is_whitespace())
                        .map(|(offset, _)| start + offset)
                })
            });
            if let Some(position) = position {
                // Source traversal and table cell order preserve the first real text.
                self.body
                    .entry(row)
                    .or_insert(MappedLineTarget::RenderedPosition(position));
            } else if opaque {
                self.opaque.insert(row);
            }
        }
        Ok(())
    }

    fn code(&mut self, code: &CodeBlock, span: Range<usize>) -> Result<(), SourceLineMapError> {
        let rows = self.rows(span.clone())?;
        let first = rows.start;
        let last = rows.end - 1;
        let raw = &self.snapshot.source()[span.clone()];
        let opening = raw.split('\n').next().unwrap_or("").trim_end_matches('\r');
        let trimmed = opening.trim_start_matches(' ');
        let indent = opening.len() - trimmed.len();
        let marker = trimmed.as_bytes().first().copied();
        let fence = marker
            .filter(|marker| matches!(marker, b'`' | b'~' | b'$'))
            .map(|marker| {
                (
                    marker,
                    trimmed.bytes().take_while(|byte| *byte == marker).count(),
                )
            })
            .filter(|(marker, count)| indent <= 3 && *count >= if *marker == b'$' { 2 } else { 3 });
        let closing_marker = fence.is_some_and(|(marker, count)| {
            if first == last {
                return false;
            }
            let tail = self.snapshot.source()[self.lines.lines[last].start..span.end].trim_end();
            let run = tail
                .bytes()
                .rev()
                .take_while(|byte| *byte == marker)
                .count();
            run >= count
                && tail[..tail.len() - run]
                    .chars()
                    .all(|character| matches!(character, ' ' | '\t' | '>'))
        });
        let body_start = first + usize::from(fence.is_some());
        let text = code.code();
        // The parsed body's line count disambiguates a real closing fence from
        // fence-shaped code (for example an over-indented final body line).
        let closing = closing_marker
            && (text.is_empty() || rows.end - body_start == text.split('\n').count() + 1);
        let mut body_end = rows.end - usize::from(closing);
        if fence.is_none() {
            // CommonMark indented code drops trailing blank lines from code(),
            // although its source span may still own them. They are structural
            // blanks, not fabricated code display rows or nearest-body candidates.
            let parsed_rows = text.split('\n').count();
            while body_end.saturating_sub(body_start) > parsed_rows {
                let row = &self.lines.lines[body_end - 1];
                let raw = &self.snapshot.source()[row.start.max(span.start)..row.end.min(span.end)];
                if !raw
                    .chars()
                    .all(|character| matches!(character, ' ' | '\t' | '>'))
                {
                    break;
                }
                body_end -= 1;
            }
        }
        let body_count = body_end.saturating_sub(body_start);
        self.advance(&text)?;
        if text.is_empty() {
            if body_count > 1 {
                return Err(SourceLineMapError::InvalidStructure);
            }
            let target = MappedLineTarget::EmptyCode { source_span: span };
            if body_count == 1 {
                self.body.insert(body_start, target.clone());
            }
            if fence.is_some() {
                self.fences.insert(first, target.clone());
            }
            if closing {
                self.fences.insert(last, target);
            }
            return Ok(());
        }
        if body_count != text.split('\n').count() {
            return Err(SourceLineMapError::InvalidStructure);
        }
        let mut offset = 0;
        let mut last_target = None;
        for (row, code_line) in (body_start..body_end).zip(text.split('\n')) {
            let target = MappedLineTarget::CodePosition {
                source_span: span.clone(),
                offset,
            };
            if row == body_start && fence.is_some() {
                self.fences.insert(first, target.clone());
            }
            self.body.insert(row, target.clone());
            last_target = Some(target);
            offset += code_line.len() + 1;
        }
        if closing {
            self.fences.insert(
                last,
                last_target.ok_or(SourceLineMapError::InvalidStructure)?,
            );
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
