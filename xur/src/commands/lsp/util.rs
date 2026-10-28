use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tower_lsp::lsp_types::*;

use xur_errors::{Diag, Level};
use xur_resolve::{DefKind, DefId, Namespace, Program, Res};
use xur_span::{SourceFile, SourceMap, Span, Symbol};

pub fn uri_to_path(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path().ok()
}

pub fn find_file_by_uri<'a>(sm: &'a SourceMap, uri: &Url) -> Option<&'a Arc<SourceFile>> {
    let path = uri_to_path(uri)?;
    let path_str = path.to_str()?;
    sm.files().iter().find(|f| f.name == path_str)
}

pub fn uri_position_to_offset(sm: &SourceMap, uri: &Url, pos: Position) -> Option<usize> {
    let file = find_file_by_uri(sm, uri)?;
    position_to_offset(file, pos)
}

pub fn position_to_offset(file: &SourceFile, pos: Position) -> Option<usize> {
    let target_line = pos.line as usize;
    let target_col = pos.character as usize;
    let mut line = 0;
    let mut col = 0;
    for (i, ch) in file.src.char_indices() {
        if line == target_line && col == target_col {
            return Some(file.start + i);
        }
        if ch == '\n' {
            if line == target_line {
                return Some(file.start + i);
            }
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    if line == target_line && col == target_col {
        Some(file.start + file.src.len())
    } else {
        None
    }
}

pub fn name_at_offset(program: &Program, offset: usize) -> Option<xur_resolve::NameRef> {
    use xur_resolve::names;
    let mut best: Option<xur_resolve::NameRef> = None;
    for module in &program.krate.modules {
        for name in names(module.ast) {
            if name.span.start <= offset
                && offset < name.span.end
                && best.as_ref().is_none_or(|b| name.span.len() < b.span.len())
            {
                best = Some(name);
            }
        }
    }
    best
}

pub fn span_to_range(sm: &SourceMap, span: Span) -> (Url, Range) {
    let file = sm.lookup_file(span.start);
    let start_lc = file.line_col(span.start);
    let end_lc = file.line_col(span.end.min(file.end()));
    let uri = Url::from_file_path(&file.name)
        .unwrap_or_else(|_| Url::parse(&format!("file:///{}", file.name)).unwrap());
    let range = Range {
        start: Position::new(start_lc.line - 1, start_lc.col - 1),
        end: Position::new(end_lc.line - 1, end_lc.col - 1),
    };
    (uri, range)
}

pub fn group_diagnostics(sm: &SourceMap, diags: &[Diag]) -> HashMap<Url, Vec<Diagnostic>> {
    let mut grouped: HashMap<Url, Vec<Diagnostic>> = HashMap::new();
    for diag in diags {
        let (uri, range) = span_to_range(sm, diag.span);
        let severity = match diag.level {
            Level::Error => DiagnosticSeverity::ERROR,
            Level::Warning => DiagnosticSeverity::WARNING,
        };
        let related: Vec<DiagnosticRelatedInformation> = diag
            .labels
            .iter()
            .map(|label| {
                let (label_uri, label_range) = span_to_range(sm, label.span);
                DiagnosticRelatedInformation {
                    location: Location::new(label_uri, label_range),
                    message: label.message.clone(),
                }
            })
            .collect();
        let mut message = diag.message.clone();
        for note in &diag.notes {
            message.push_str("\nnote: ");
            message.push_str(note);
        }
        grouped.entry(uri).or_default().push(Diagnostic {
            range,
            severity: Some(severity),
            message,
            related_information: if related.is_empty() {
                None
            } else {
                Some(related)
            },
            ..Default::default()
        });
    }
    grouped
}

pub fn same_target(a: Res, b: Res) -> bool {
    match (a, b) {
        (Res::Def(_, a), Res::Def(_, b)) => a == b,
        (Res::Local(a), Res::Local(b)) => a == b,
        (Res::PrimTy(a), Res::PrimTy(b)) => a == b,
        _ => false,
    }
}

pub fn find_module_def(program: &Program, uri: &Url) -> Option<DefId> {
    let path = uri_to_path(uri)?;
    let resolutions = program.resolutions();
    for (i, module) in program.krate.modules.iter().enumerate() {
        if module.path == path {
            return resolutions.module_defs.get(i).copied();
        }
    }
    None
}

pub fn completion_kind(res: Res) -> CompletionItemKind {
    match res {
        Res::Def(DefKind::Fn | DefKind::NativeFn(_), _) => CompletionItemKind::FUNCTION,
        Res::Def(DefKind::Mod, _) => CompletionItemKind::MODULE,
        Res::Def(DefKind::Struct | DefKind::NativeType(_), _) => CompletionItemKind::STRUCT,
        Res::Def(DefKind::Enum, _) => CompletionItemKind::ENUM,
        Res::Def(DefKind::Variant | DefKind::NativeVariant(_), _) => {
            CompletionItemKind::ENUM_MEMBER
        }
        Res::Def(DefKind::Const, _) => CompletionItemKind::CONSTANT,
        Res::Def(DefKind::TyParam, _) => CompletionItemKind::TYPE_PARAMETER,
        Res::PrimTy(_) => CompletionItemKind::CLASS,
        Res::Local(_) => CompletionItemKind::VARIABLE,
        Res::TyParamAssoc(..) => CompletionItemKind::METHOD,
        Res::Err => CompletionItemKind::TEXT,
    }
}

pub fn line_prefix(src: &str, pos: Position) -> Option<String> {
    let line = src.lines().nth(pos.line as usize)?;
    let col = (pos.character as usize).min(line.len());
    Some(line[..col].to_owned())
}

pub fn resolve_path_prefix(
    prefix: &str,
    module_def: DefId,
    resolutions: &xur_resolve::Resolutions,
) -> Option<DefId> {
    let trimmed = prefix.trim_start();
    if !trimmed.ends_with("::") {
        return None;
    }
    let path_str = trimmed.trim_end_matches("::");
    let segments: Vec<&str> = path_str.rsplit("::").collect::<Vec<_>>().into_iter().rev().collect();
    let first = segments.first()?;
    let first = first.split_whitespace().last()?;
    if first.is_empty() {
        return None;
    }

    let first_sym = Symbol::intern(first);
    let first_res = resolutions.lookup(module_def, Namespace::Type, first_sym)
        .or_else(|| resolutions.root_module(first_sym).map(|id| Res::Def(DefKind::Mod, id)))?;
    let mut scope = first_res.scope()?;

    for &seg in &segments[1..] {
        let res = resolutions.lookup(scope, Namespace::Type, Symbol::intern(seg))?;
        scope = res.scope()?;
    }

    Some(scope)
}

pub const KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "else", "enum", "false", "fn", "for", "if", "impl",
    "in", "is", "let", "loop", "match", "pub", "return", "struct", "true", "use", "while",
];
