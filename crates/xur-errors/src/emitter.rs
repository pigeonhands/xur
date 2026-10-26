use std::fmt;
use std::io::{self, Write};
use std::ops::Range;
use std::sync::Arc;

use ariadne::{Cache, Color, Config, IndexType, Report, ReportKind, Source};
use xur_span::{SourceFile, SourceMap, Span};

use crate::{Diag, Level};

/// A span as ariadne sees it: a file index and a byte range within the file.
type ReportSpan = (usize, Range<usize>);

/// Renders diagnostics whose spans may point into any file of a source map.
pub struct Emitter<'a> {
    files: &'a [Arc<SourceFile>],
    sources: Vec<Source<&'a str>>,
    color: bool,
}

impl<'a> Emitter<'a> {
    pub fn new(source_map: &'a SourceMap) -> Self {
        let files = source_map.files();
        Self {
            files,
            sources: files
                .iter()
                .map(|file| Source::from(file.src.as_str()))
                .collect(),
            color: false,
        }
    }

    pub fn with_color(mut self, color: bool) -> Self {
        self.color = color;
        self
    }

    pub fn emit(&self, diag: &Diag, out: &mut impl Write) -> io::Result<()> {
        let cache = Sources {
            files: self.files,
            sources: &self.sources,
        };
        self.report(diag).write(cache, out)
    }

    fn locate(&self, span: Span) -> ReportSpan {
        let index = self
            .files
            .iter()
            .position(|file| file.contains(span.start))
            .expect("diagnostic span is in no source file");
        let start = self.files[index].start;
        (index, span.start - start..span.end - start)
    }

    fn report(&self, diag: &Diag) -> Report<'_, ReportSpan> {
        let (kind, primary_color) = match diag.level {
            Level::Error => (ReportKind::Error, Color::Red),
            Level::Warning => (ReportKind::Warning, Color::Yellow),
        };
        let config = Config::default()
            .with_index_type(IndexType::Byte)
            .with_color(self.color);

        let mut report = Report::build(kind, self.locate(diag.span))
            .with_config(config)
            .with_message(&diag.message);
        for label in &diag.labels {
            let color = if label.span == diag.span {
                primary_color
            } else {
                Color::Blue
            };
            report.add_label(
                ariadne::Label::new(self.locate(label.span))
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

/// Gives ariadne access to every file's text, by file index.
struct Sources<'a> {
    files: &'a [Arc<SourceFile>],
    sources: &'a [Source<&'a str>],
}

impl<'a> Cache<usize> for Sources<'a> {
    type Storage = &'a str;

    fn fetch(&mut self, id: &usize) -> Result<&Source<&'a str>, impl fmt::Debug> {
        self.sources.get(*id).ok_or("unknown source file")
    }

    fn display<'b>(&self, id: &'b usize) -> Option<impl fmt::Display + 'b> {
        Some(self.files[*id].name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(map: &SourceMap, diag: &Diag) -> String {
        let mut out = Vec::new();
        Emitter::new(map).emit(diag, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn renders_message_labels_and_notes() {
        let mut map = SourceMap::new();
        map.add_file("test.xur", "let x = $;");
        let diag = Diag::error("unexpected character `$`", Span::new(8, 9))
            .with_label(Span::new(8, 9), "not valid here")
            .with_note("remove it");
        let out = render(&map, &diag);
        assert!(out.starts_with("Error: unexpected character `$`"), "{out}");
        assert!(out.contains("test.xur:1:9"), "{out}");
        assert!(out.contains("not valid here"), "{out}");
        assert!(out.contains("Note: remove it"), "{out}");
    }

    #[test]
    fn labels_can_point_into_other_files() {
        let mut map = SourceMap::new();
        let a = map.add_file("a.xur", "fn f() {}");
        let b = map.add_file("b.xur", "fn f() {}");
        let in_a = Span::new(a.start + 3, a.start + 4);
        let in_b = Span::new(b.start + 3, b.start + 4);
        let diag = Diag::error("the name `f` is defined multiple times", in_b)
            .with_label(in_b, "redefined here")
            .with_label(in_a, "previous definition here");
        let out = render(&map, &diag);
        assert!(out.contains("b.xur:1:4"), "{out}");
        assert!(out.contains("a.xur:1:4"), "{out}");
    }
}
