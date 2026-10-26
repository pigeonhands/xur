use std::io::{self, Write};
use std::ops::Range;

use ariadne::{Color, Config, IndexType, Report, ReportKind, Source};

use crate::{Diag, Level};

/// Renders diagnostics for one source file.
pub struct Emitter<'a> {
    file_name: &'a str,
    src: &'a str,
    color: bool,
}

impl<'a> Emitter<'a> {
    pub fn new(file_name: &'a str, src: &'a str) -> Self {
        Self {
            file_name,
            src,
            color: false,
        }
    }

    pub fn with_color(mut self, color: bool) -> Self {
        self.color = color;
        self
    }

    pub fn emit(&self, diag: &Diag, out: &mut impl Write) -> io::Result<()> {
        self.report(diag)
            .write((self.file_name, Source::from(self.src)), out)
    }

    fn report(&self, diag: &Diag) -> Report<'a, (&'a str, Range<usize>)> {
        let (kind, primary_color) = match diag.level {
            Level::Error => (ReportKind::Error, Color::Red),
            Level::Warning => (ReportKind::Warning, Color::Yellow),
        };
        let config = Config::default()
            .with_index_type(IndexType::Byte)
            .with_color(self.color);

        let mut report = Report::build(kind, (self.file_name, diag.span.into()))
            .with_config(config)
            .with_message(&diag.message);
        for label in &diag.labels {
            let color = if label.span == diag.span {
                primary_color
            } else {
                Color::Blue
            };
            report.add_label(
                ariadne::Label::new((self.file_name, label.span.into()))
                    .with_message(&label.message)
                    .with_color(color),
            );
        }
        for note in &diag.notes {
            report.add_note(note);
        }
        report.finish()
    }
}

#[cfg(test)]
mod tests {
    use xur_span::Span;

    use super::*;

    fn render(src: &str, diag: &Diag) -> String {
        let mut out = Vec::new();
        Emitter::new("test.xur", src).emit(diag, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn renders_message_labels_and_notes() {
        let diag = Diag::error("unexpected character `$`", Span::new(8, 9))
            .with_label(Span::new(8, 9), "not valid here")
            .with_note("remove it");
        let out = render("let x = $;", &diag);
        assert!(out.starts_with("Error: unexpected character `$`"), "{out}");
        assert!(out.contains("test.xur:1:9"), "{out}");
        assert!(out.contains("not valid here"), "{out}");
        assert!(out.contains("Note: remove it"), "{out}");
    }
}
