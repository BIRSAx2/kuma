use std::fmt;

/// A half-open byte range in the original UTF-8 source.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct Span {
    start: usize,
    end: usize,
}

impl Span {
    pub(crate) fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn start(self) -> usize {
        self.start
    }

    pub fn end(self) -> usize {
        self.end
    }

    pub fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// A source diagnostic with both byte and human-readable coordinates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    message: String,
    span: Span,
    line: usize,
    column: usize,
}

impl Diagnostic {
    pub(crate) fn new(message: String, span: Span, line: usize, column: usize) -> Self {
        Self {
            message,
            span,
            line,
            column,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn span(&self) -> Span {
        self.span
    }

    /// One-based source line.
    pub fn line(&self) -> usize {
        self.line
    }

    /// One-based UTF-8 source column.
    pub fn column(&self) -> usize {
        self.column
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}
