use std::fmt;

/// A 1-based line and column. The column counts characters, not bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    pub line: u32,
    pub col: u32,
}

impl fmt::Display for LineCol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

/// Maps byte offsets in a source text to line/column positions.
pub struct LineIndex<'src> {
    src: &'src str,
    /// Byte offset at which each line starts; always begins with `0`.
    line_starts: Vec<usize>,
}

impl<'src> LineIndex<'src> {
    pub fn new(src: &'src str) -> Self {
        let line_starts = std::iter::once(0)
            .chain(src.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self { src, line_starts }
    }

    /// The line/column of the character starting at byte `offset`.
    ///
    /// An offset equal to the source length (end of input) is allowed.
    pub fn line_col(&self, offset: usize) -> LineCol {
        let line = self.line_starts.partition_point(|&start| start <= offset) - 1;
        let line_start = self.line_starts[line];
        let col = self.src[line_start..offset].chars().count();
        LineCol {
            line: line as u32 + 1,
            col: col as u32 + 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_first_line() {
        let index = LineIndex::new("let x");
        assert_eq!(index.line_col(0), LineCol { line: 1, col: 1 });
        assert_eq!(index.line_col(4), LineCol { line: 1, col: 5 });
    }

    #[test]
    fn line_col_after_newlines() {
        let index = LineIndex::new("a\nbc\n\nd");
        assert_eq!(index.line_col(2), LineCol { line: 2, col: 1 });
        assert_eq!(index.line_col(3), LineCol { line: 2, col: 2 });
        assert_eq!(index.line_col(5), LineCol { line: 3, col: 1 });
        assert_eq!(index.line_col(6), LineCol { line: 4, col: 1 });
    }

    #[test]
    fn line_col_counts_chars_not_bytes() {
        // `α` is two bytes wide.
        let index = LineIndex::new("α = 1");
        assert_eq!(index.line_col(3), LineCol { line: 1, col: 3 });
    }

    #[test]
    fn line_col_at_end_of_input() {
        let src = "a\nb";
        assert_eq!(
            LineIndex::new(src).line_col(src.len()),
            LineCol { line: 2, col: 2 }
        );
    }
}
