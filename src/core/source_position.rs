use std::{num::NonZeroU64, ops::Range, sync::Arc};

/// Original UTF-8 line spans. The final empty preview row is retained separately
/// from the logical line count used by external source-line navigation.
pub struct SourceLines {
    pub lines: Vec<Range<usize>>,
    count: usize,
    source: Arc<str>,
}

impl SourceLines {
    pub fn new(source: &str) -> Self {
        let mut start = 0;
        let mut lines = Vec::new();
        for line in source.split_inclusive('\n') {
            let content = line.strip_suffix('\n').unwrap_or(line);
            let content = if line.ends_with('\n') {
                content.strip_suffix('\r').unwrap_or(content)
            } else {
                content
            };
            lines.push(start..start + content.len());
            start += line.len();
        }
        let count = lines.len().max(1);
        if source.is_empty() || source.ends_with('\n') {
            lines.push(source.len()..source.len());
        }
        Self {
            lines,
            count,
            source: Arc::from(source),
        }
    }

    /// Exact source identity, retained once when indexing the document, not per request.
    pub fn matches_source(&self, source: &str) -> bool {
        self.source.as_ref() == source
    }

    /// Number of saved physical lines, excluding the terminal preview row.
    pub fn logical_count(&self) -> usize {
        self.count
    }

    /// None means beyond the last saved source line, not a truncated usize.
    pub fn range(&self, line: NonZeroU64) -> Option<Range<usize>> {
        if line.get() > self.count as u64 {
            return None;
        }
        Some(self.lines[(line.get() - 1) as usize].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn line(value: u64) -> NonZeroU64 {
        NonZeroU64::new(value).unwrap()
    }

    #[test]
    fn source_lines_keep_utf8_crlf_and_preview_terminal_rows() {
        let source = "日本語\r\nsecond\n\n";
        let index = SourceLines::new(source);
        assert_eq!(index.lines, vec![0..9, 11..17, 18..18, 19..19]);
        assert_eq!(index.range(line(1)), Some(0..9));
        assert_eq!(index.range(line(3)), Some(18..18));
        assert_eq!(index.range(line(4)), None);
        assert_eq!(index.range(line(u64::MAX)), None);
        assert_eq!(SourceLines::new("").range(line(1)), Some(0..0));
        assert_eq!(SourceLines::new("a\n").range(line(2)), None);
        assert_eq!(SourceLines::new("a").range(line(1)), Some(0..1));
    }

    #[test]
    fn source_identity_distinguishes_equal_length_documents() {
        let index = SourceLines::new("one\r\n");
        assert!(index.matches_source("one\r\n"));
        assert!(!index.matches_source("two\r\n"));
        assert_eq!(index.logical_count(), 1);
    }
}
