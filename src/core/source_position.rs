use std::{num::NonZeroU64, ops::Range};

/// Original UTF-8 line spans. The final empty preview row is retained separately
/// from the logical line count used by external source-line navigation.
pub struct SourceLines {
    pub lines: Vec<Range<usize>>,
    count: usize,
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
        Self { lines, count }
    }

    /// None means beyond the last saved source line, not a truncated usize.
    pub fn range(&self, line: NonZeroU64) -> Option<Range<usize>> {
        if line.get() > self.count as u64 {
            return None;
        }
        Some(self.lines[(line.get() - 1) as usize].clone())
    }

    pub fn nearest_block(
        &self,
        line: NonZeroU64,
        blocks: &[Option<Range<usize>>],
    ) -> Option<usize> {
        let wanted = (line.get() - 1).min((self.count - 1) as u64) as usize;
        blocks
            .iter()
            .enumerate()
            .filter_map(|(ix, span)| {
                let span = span.as_ref()?;
                let first = self
                    .lines
                    .partition_point(|row| row.start <= span.start)
                    .saturating_sub(1);
                let last = self
                    .lines
                    .partition_point(|row| row.start < span.end)
                    .saturating_sub(1)
                    .max(first);
                let distance = if wanted < first {
                    first - wanted
                } else {
                    wanted.saturating_sub(last)
                };
                // A following block wins ties; source order is not rendered-text search.
                Some(((distance, std::cmp::Reverse(first)), ix))
            })
            .min_by_key(|(key, _)| *key)
            .map(|(_, ix)| ix)
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
    fn blank_lines_choose_the_nearest_block_and_following_ties() {
        let index = SourceLines::new("a\n\n\nb\n\n\nc");
        let blocks = vec![Some(0..1), Some(5..6), Some(10..11)];
        assert_eq!(index.nearest_block(line(2), &blocks), Some(0));
        assert_eq!(index.nearest_block(line(3), &blocks), Some(1));
        let index = SourceLines::new("a\n\nb");
        assert_eq!(
            index.nearest_block(line(2), &[Some(0..1), Some(3..4)]),
            Some(1)
        );
        assert_eq!(index.nearest_block(line(2), &[None]), None);
    }
}
