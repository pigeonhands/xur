use std::fmt;
use std::ops::Range;

/// A half-open byte range `start..end` into the source text.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
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

    /// The span from the start of `self` to the end of `end`.
    pub fn to(self, end: Span) -> Span {
        Span::new(self.start, end.end)
    }

    /// The empty span at the end of `self`.
    pub fn shrink_to_hi(self) -> Span {
        Span::new(self.end, self.end)
    }
}

/// Prints as `start..end`, keeping AST dumps compact.
impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
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
        assert_eq!(span.len(), 3);
        assert_eq!(Range::from(span), 4..7);
    }

    #[test]
    fn span_to_and_shrink() {
        let span = Span::new(2, 4).to(Span::new(8, 11));
        assert_eq!(span, Span::new(2, 11));
        assert_eq!(span.shrink_to_hi(), Span::new(11, 11));
        assert_eq!(format!("{span:?}"), "2..11");
    }
}
