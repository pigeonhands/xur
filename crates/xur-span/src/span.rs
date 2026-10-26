use std::ops::Range;

/// A half-open byte range `start..end` into the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        debug_assert!(start <= end, "span start {start} is after end {end}");
        Self { start, end }
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// The source text this span covers.
    pub fn slice<'src>(&self, src: &'src str) -> &'src str {
        &src[self.start..self.end]
    }
}

impl From<Span> for Range<usize> {
    fn from(span: Span) -> Self {
        span.start..span.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_slice_and_range() {
        let span = Span::new(4, 7);
        assert_eq!(span.slice("let foo = 1"), "foo");
        assert_eq!(span.len(), 3);
        assert_eq!(Range::from(span), 4..7);
    }
}
