//! Textual IR frontend.

mod parser;

pub(crate) use parser::ParseResult;

#[derive(Debug)]
pub(crate) struct ParseFailure {
    message: String,
    start: usize,
    end: usize,
    line: usize,
    column: usize,
}

impl ParseFailure {
    pub(crate) fn at(source: &[u8], position: usize, line: u32, message: String) -> Self {
        let start = position.saturating_sub(1).min(source.len());
        let end = position.min(source.len()).max(start);
        let line_start = source[..start]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        let column = std::str::from_utf8(&source[line_start..start])
            .map_or(start - line_start + 1, |prefix| prefix.chars().count() + 1);
        Self {
            message,
            start,
            end,
            line: line as usize,
            column,
        }
    }
}

// The public parser facade is defined here alongside the private parser so callers
// never depend on compiler-owned mutable records.
pub use facade::{ParseError, parse};

mod facade;
