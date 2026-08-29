use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::diagnostic::{Diagnostic, Span};
use crate::ir::Module;

use super::ParseFailure;

/// A textual IR parsing error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseError {
    diagnostic: Diagnostic,
}

impl ParseError {
    fn from_failure(failure: &ParseFailure) -> Self {
        Self {
            diagnostic: Diagnostic::new(
                failure.message.clone(),
                Span::new(failure.start, failure.end),
                failure.line,
                failure.column,
            ),
        }
    }

    pub(crate) fn internal(message: String) -> Self {
        Self {
            diagnostic: Diagnostic::new(message, Span::new(0, 0), 1, 1),
        }
    }

    pub fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parse error: {}", self.diagnostic)
    }
}

impl std::error::Error for ParseError {}

/// Parse textual Kuma IR into immutable semantic IR.
pub fn parse(source: &str) -> Result<Module, ParseError> {
    let parsed = catch_unwind(AssertUnwindSafe(|| super::parser::parse(source)))
        .map_err(|payload| {
            if let Some(failure) = payload.downcast_ref::<ParseFailure>() {
                ParseError::from_failure(failure)
            } else if let Some(message) = payload.downcast_ref::<String>() {
                ParseError::internal(message.clone())
            } else if let Some(message) = payload.downcast_ref::<&str>() {
                ParseError::internal((*message).to_owned())
            } else {
                ParseError::internal("the parser failed unexpectedly".to_owned())
            }
        })?
        .map_err(|failure| ParseError::from_failure(&failure))?;
    Ok(Module::from_parsed(parsed))
}
