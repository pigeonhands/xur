//! Rendering of compiler errors as annotated source snippets.

use std::io::{self, IsTerminal};
use std::ops::Range;

use ariadne::{Color, Config, IndexType, Label, Report, ReportKind, Source};
use xur_parser::{LexError, LexErrorKind};

use crate::source::SourceFile;

type ReportSpan<'a> = (&'a str, Range<usize>);

/// Prints every lex error in `file` to stderr.
pub fn report_lex_errors(file: &SourceFile, errors: &[LexError]) -> io::Result<()> {
    for error in errors {
        let span = (file.name.as_str(), error.span.into());
        lex_error_report(error.kind, span)
            .eprint((file.name.as_str(), Source::from(file.text.as_str())))?;
    }
    Ok(())
}

fn lex_error_report(kind: LexErrorKind, span: ReportSpan<'_>) -> Report<'_, ReportSpan<'_>> {
    let label = match kind {
        LexErrorKind::UnexpectedChar(_) => "not valid in Xur source",
        LexErrorKind::UnterminatedString => "missing closing `\"`",
        LexErrorKind::UnterminatedBlockComment => "comment is never closed",
        LexErrorKind::IntTooLarge => "does not fit in 64 bits",
    };
    Report::build(ReportKind::Error, span.clone())
        .with_config(
            Config::default()
                .with_index_type(IndexType::Byte)
                .with_color(io::stderr().is_terminal()),
        )
        .with_message(kind)
        .with_label(Label::new(span).with_message(label).with_color(Color::Red))
        .finish()
}
