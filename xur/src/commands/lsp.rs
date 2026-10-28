//! `xur lsp`: the Xur language server.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use xur_errors::{Diag, Level};
use xur_ast::{Expr, ExprKind, FnRetTy, ItemKind, LitKind, Local, PatKind};
use xur_ast::visit::{self, Visitor};
use xur_bytecode::{ItemIndex, ItemRef, type_name};
use xur_resolve::{DefId, DefKind, NativeFnId, Namespace, PrimTy, Program, Res, Resolutions, names};
use xur_runtime::Registry;
use xur_span::{Ident, OverlayFileLoader, SourceFile, SourceMap, Span, Symbol};

pub fn run() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let stdin = tokio::io::stdin();
            let stdout = tokio::io::stdout();
            let (service, socket) = LspService::new(|client| Backend {
                client,
                documents: Mutex::new(HashMap::new()),
                state: Mutex::new(None),
            });
            Server::new(stdin, stdout, socket).serve(service).await;
            Ok(())
        })
}

struct Backend {
    client: Client,
    documents: Mutex<HashMap<Url, String>>,
    state: Mutex<Option<AnalysisState>>,
}

struct AnalysisState {
    program: Program,
    registry: Arc<Registry>,
    items: ItemIndex,
    diagnosed_uris: HashSet<Url>,
}

impl Backend {
    async fn recompile(&self, changed_uri: &Url) {
        let root = self.find_root(changed_uri);
        let Some(root) = root else { return };

        let overrides: HashMap<PathBuf, String> = {
            let docs = self.documents.lock().unwrap();
            docs.iter()
                .filter_map(|(uri, src)| Some((uri_to_path(uri)?, src.clone())))
                .collect()
        };

        let loader = Box::new(OverlayFileLoader::new(overrides));
        let registry = Arc::new(xur_std::registry());
        let natives = Box::new(Arc::clone(&registry));

        match Program::load(&root, loader, natives) {
            Ok((program, diags)) => {
                let grouped = group_diagnostics(&program.krate.source_map, &diags);
                let mut new_uris = HashSet::new();

                for (uri, file_diags) in &grouped {
                    new_uris.insert(uri.clone());
                    self.client
                        .publish_diagnostics(uri.clone(), file_diags.clone(), None)
                        .await;
                }

                let prev = self.state.lock().unwrap().take();
                if let Some(prev) = prev {
                    for uri in &prev.diagnosed_uris {
                        if !new_uris.contains(uri) {
                            self.client
                                .publish_diagnostics(uri.clone(), Vec::new(), None)
                                .await;
                        }
                    }
                }

                let items = ItemIndex::new(&program);
                *self.state.lock().unwrap() = Some(AnalysisState {
                    program,
                    registry,
                    items,
                    diagnosed_uris: new_uris,
                });
            }
            Err(e) => {
                let diag = Diagnostic {
                    message: format!("failed to load: {e}"),
                    severity: Some(DiagnosticSeverity::ERROR),
                    ..Default::default()
                };
                self.client
                    .publish_diagnostics(changed_uri.clone(), vec![diag], None)
                    .await;
            }
        }
    }

    fn find_root(&self, changed_uri: &Url) -> Option<PathBuf> {
        uri_to_path(changed_uri)
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".into(), ":".into()]),
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "xur".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "xur language server ready")
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        self.documents
            .lock()
            .unwrap()
            .insert(uri.clone(), params.text_document.text);
        self.recompile(&uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        if let Some(change) = params.content_changes.into_iter().last() {
            self.documents
                .lock()
                .unwrap()
                .insert(uri.clone(), change.text);
        }
        self.recompile(&uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.recompile(&params.text_document.uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .lock()
            .unwrap()
            .remove(&params.text_document.uri);
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let state = self.state.lock().unwrap();
        let Some(state) = state.as_ref() else {
            return Ok(None);
        };
        let program = &state.program;
        let sm = &program.krate.source_map;
        let resolutions = program.resolutions();

        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let Some(offset) = uri_position_to_offset(sm, uri, pos) else {
            return Ok(None);
        };
        let Some(name) = name_at_offset(program, offset) else {
            return Ok(None);
        };
        let Some(&res) = resolutions.res.get(&name.id) else {
            return Ok(None);
        };

        let def_span = match res {
            Res::Def(_, def_id) => resolutions.defs.get(def_id).span,
            Res::Local(node_id) => resolutions.locals.get(&node_id).map(|id| id.span),
            _ => None,
        };

        let Some(span) = def_span else {
            return Ok(None);
        };
        let (target_uri, range) = span_to_range(sm, span);
        Ok(Some(GotoDefinitionResponse::Scalar(Location::new(
            target_uri, range,
        ))))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let state = self.state.lock().unwrap();
        let Some(state) = state.as_ref() else {
            return Ok(None);
        };
        let program = &state.program;
        let sm = &program.krate.source_map;
        let resolutions = program.resolutions();

        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let Some(offset) = uri_position_to_offset(sm, uri, pos) else {
            return Ok(None);
        };
        let Some(name) = name_at_offset(program, offset) else {
            return Ok(None);
        };
        let Some(&res) = resolutions.res.get(&name.id) else {
            return Ok(None);
        };

        let description = resolutions.describe(res, sm);
        let (_, range) = span_to_range(sm, name.span);
        Ok(Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(description)),
            range: Some(range),
        }))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let state = self.state.lock().unwrap();
        let Some(state) = state.as_ref() else {
            return Ok(None);
        };
        let program = &state.program;
        let sm = &program.krate.source_map;
        let resolutions = program.resolutions();

        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;

        let Some(offset) = uri_position_to_offset(sm, uri, pos) else {
            return Ok(None);
        };
        let Some(name) = name_at_offset(program, offset) else {
            return Ok(None);
        };
        let Some(&target_res) = resolutions.res.get(&name.id) else {
            return Ok(None);
        };

        let mut locations = Vec::new();

        for module in &program.krate.modules {
            for candidate in names(module.ast) {
                if let Some(&res) = resolutions.res.get(&candidate.id)
                    && same_target(res, target_res)
                {
                    let (uri, range) = span_to_range(sm, candidate.span);
                    locations.push(Location::new(uri, range));
                }
            }
        }

        if params.context.include_declaration {
            let def_span = match target_res {
                Res::Def(_, def_id) => resolutions.defs.get(def_id).span,
                Res::Local(node_id) => resolutions.locals.get(&node_id).map(|id| id.span),
                _ => None,
            };
            if let Some(span) = def_span {
                let (uri, range) = span_to_range(sm, span);
                let loc = Location::new(uri, range);
                if !locations.contains(&loc) {
                    locations.push(loc);
                }
            }
        }

        Ok(Some(locations))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let state = self.state.lock().unwrap();
        let Some(state) = state.as_ref() else {
            return Ok(None);
        };
        let program = &state.program;
        let resolutions = program.resolutions();

        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;

        let prefix = {
            let docs = self.documents.lock().unwrap();
            docs.get(uri).and_then(|src| line_prefix(src, pos))
        };

        let module_def = find_module_def(program, uri);
        let mut items = Vec::new();

        let path_scope = prefix.as_deref()
            .and_then(|pfx| resolve_path_prefix(pfx, module_def?, resolutions));

        if let Some(scope) = path_scope {
            for (name, res) in resolutions.scope_entries(scope, Namespace::Value) {
                items.push(CompletionItem {
                    label: name.as_str().to_owned(),
                    kind: Some(completion_kind(res)),
                    ..Default::default()
                });
            }
            for (name, res) in resolutions.scope_entries(scope, Namespace::Type) {
                items.push(CompletionItem {
                    label: name.as_str().to_owned(),
                    kind: Some(completion_kind(res)),
                    ..Default::default()
                });
            }
        } else {
            if let Some(def_id) = module_def {
                for (name, res) in resolutions.scope_entries(def_id, Namespace::Value) {
                    items.push(CompletionItem {
                        label: name.as_str().to_owned(),
                        kind: Some(completion_kind(res)),
                        ..Default::default()
                    });
                }
                for (name, res) in resolutions.scope_entries(def_id, Namespace::Type) {
                    items.push(CompletionItem {
                        label: name.as_str().to_owned(),
                        kind: Some(completion_kind(res)),
                        ..Default::default()
                    });
                }
            }

            for (name, _) in resolutions.root_modules() {
                items.push(CompletionItem {
                    label: name.as_str().to_owned(),
                    kind: Some(CompletionItemKind::MODULE),
                    ..Default::default()
                });
            }

            for ty in PrimTy::ALL {
                items.push(CompletionItem {
                    label: ty.name().to_owned(),
                    kind: Some(CompletionItemKind::CLASS),
                    ..Default::default()
                });
            }

            for kw in KEYWORDS {
                items.push(CompletionItem {
                    label: (*kw).to_owned(),
                    kind: Some(CompletionItemKind::KEYWORD),
                    ..Default::default()
                });
            }
        }

        items.sort_by(|a, b| a.label.cmp(&b.label));
        items.dedup_by(|a, b| a.label == b.label);
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let state = self.state.lock().unwrap();
        let Some(state) = state.as_ref() else {
            return Ok(None);
        };
        let program = &state.program;
        let sm = &program.krate.source_map;
        let resolutions = program.resolutions();

        let uri = &params.text_document.uri;
        let file = find_file_by_uri(sm, uri);
        let Some(file) = file else {
            return Ok(None);
        };
        let file_start = file.start;
        let file_end = file.end();
        let in_file = |span: Span| span.start >= file_start && span.start < file_end;

        let param_names = build_param_name_map(program);
        let mut hints = Vec::new();

        for module in &program.krate.modules {
            let mut call_collector = CallCollector(Vec::new());
            let mut let_collector = LetCollector(Vec::new());
            let mut match_collector = MatchCollector(Vec::new());
            call_collector.visit_module(module.ast);
            let_collector.visit_module(module.ast);
            match_collector.visit_module(module.ast);

            // Parameter name hints at call sites.
            for (callee_id, args) in call_collector.0 {
                let Some(&res) = resolutions.res.get(&callee_id) else {
                    continue;
                };
                let names: Vec<String> = match res {
                    Res::Def(DefKind::Fn, def_id) => {
                        match param_names.get(&def_id) {
                            Some(idents) => idents.iter().map(|id| id.name.to_string()).collect(),
                            None => continue,
                        }
                    }
                    Res::Def(DefKind::NativeFn(id), _) => {
                        state.registry.function(id).param_names.clone()
                    }
                    _ => continue,
                };
                for (i, arg_span) in args.iter().enumerate() {
                    if !in_file(*arg_span) {
                        continue;
                    }
                    if let Some(name) = names.get(i) {
                        if name == "_" || name.is_empty() {
                            continue;
                        }
                        let pos = file.line_col(arg_span.start);
                        hints.push(InlayHint {
                            position: Position::new(pos.line - 1, pos.col - 1),
                            label: InlayHintLabel::String(format!("{name}: ")),
                            kind: Some(InlayHintKind::PARAMETER),
                            padding_left: None,
                            padding_right: None,
                            text_edits: None,
                            tooltip: None,
                            data: None,
                        });
                    }
                }
            }

            // Type hints on let bindings without explicit annotations.
            for (pat_end, init) in &let_collector.0 {
                if !in_file(Span { start: *pat_end, end: *pat_end }) {
                    continue;
                }
                if let Some(ty) = infer_expr_type(init, resolutions, &state.items, &state.registry) {
                    let pos = file.line_col(*pat_end);
                    hints.push(InlayHint {
                        position: Position::new(pos.line - 1, pos.col - 1),
                        label: InlayHintLabel::String(format!(": {ty}")),
                        kind: Some(InlayHintKind::TYPE),
                        padding_left: None,
                        padding_right: None,
                        text_edits: None,
                        tooltip: None,
                        data: None,
                    });
                }
            }

            // Type hints on match scrutinees.
            for (scrutinee_end, scrutinee) in &match_collector.0 {
                if !in_file(Span { start: *scrutinee_end, end: *scrutinee_end }) {
                    continue;
                }
                if let Some(ty) = infer_expr_type(scrutinee, resolutions, &state.items, &state.registry) {
                    let pos = file.line_col(*scrutinee_end);
                    hints.push(InlayHint {
                        position: Position::new(pos.line - 1, pos.col - 1),
                        label: InlayHintLabel::String(format!(": {ty}")),
                        kind: Some(InlayHintKind::TYPE),
                        padding_left: Some(true),
                        padding_right: None,
                        text_edits: None,
                        tooltip: None,
                        data: None,
                    });
                }
            }
        }

        Ok(Some(hints))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn uri_to_path(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path().ok()
}

fn find_file_by_uri<'a>(sm: &'a SourceMap, uri: &Url) -> Option<&'a Arc<SourceFile>> {
    let path = uri_to_path(uri)?;
    let path_str = path.to_str()?;
    sm.files().iter().find(|f| f.name == path_str)
}

fn uri_position_to_offset(sm: &SourceMap, uri: &Url, pos: Position) -> Option<usize> {
    let file = find_file_by_uri(sm, uri)?;
    position_to_offset(file, pos)
}

fn position_to_offset(file: &SourceFile, pos: Position) -> Option<usize> {
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

fn name_at_offset(program: &Program, offset: usize) -> Option<xur_resolve::NameRef> {
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

fn span_to_range(sm: &SourceMap, span: Span) -> (Url, Range) {
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

fn group_diagnostics(sm: &SourceMap, diags: &[Diag]) -> HashMap<Url, Vec<Diagnostic>> {
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

fn same_target(a: Res, b: Res) -> bool {
    match (a, b) {
        (Res::Def(_, a), Res::Def(_, b)) => a == b,
        (Res::Local(a), Res::Local(b)) => a == b,
        (Res::PrimTy(a), Res::PrimTy(b)) => a == b,
        _ => false,
    }
}

fn find_module_def(program: &Program, uri: &Url) -> Option<xur_resolve::DefId> {
    let path = uri_to_path(uri)?;
    let resolutions = program.resolutions();
    for (i, module) in program.krate.modules.iter().enumerate() {
        if module.path == path {
            return resolutions.module_defs.get(i).copied();
        }
    }
    None
}

fn completion_kind(res: Res) -> CompletionItemKind {
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

fn line_prefix(src: &str, pos: Position) -> Option<String> {
    let line = src.lines().nth(pos.line as usize)?;
    let col = (pos.character as usize).min(line.len());
    Some(line[..col].to_owned())
}

fn resolve_path_prefix(
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

struct CallCollector(Vec<(xur_ast::NodeId, Vec<Span>)>);

impl<'ast> Visitor<'ast> for CallCollector {
    fn visit_expr(&mut self, expr: &'ast xur_ast::Expr) {
        if let ExprKind::Call(callee, args) = &expr.kind
            && let ExprKind::Path(path) = &callee.kind
        {
            let arg_spans: Vec<_> = args.iter().map(|a| a.span).collect();
            self.0.push((path.id, arg_spans));
        }
        visit::walk_expr(self, expr);
    }
}

/// Collects let bindings without type annotations: `(pat_span_end, init_expr)`.
struct LetCollector<'a>(Vec<(usize, &'a Expr)>);

impl<'ast> Visitor<'ast> for LetCollector<'ast> {
    fn visit_local(&mut self, local: &'ast Local) {
        if local.ty.is_none()
            && let Some(init) = &local.init
        {
            self.0.push((local.pat.span.end, init));
        }
        visit::walk_local(self, local);
    }
}

/// Collects match expressions: `(scrutinee_span_end, scrutinee_expr)`.
struct MatchCollector<'a>(Vec<(usize, &'a Expr)>);

impl<'ast> Visitor<'ast> for MatchCollector<'ast> {
    fn visit_expr(&mut self, expr: &'ast xur_ast::Expr) {
        if let ExprKind::Match(scrutinee, _) = &expr.kind {
            self.0.push((scrutinee.span.end, scrutinee));
        }
        visit::walk_expr(self, expr);
    }
}

/// Syntactically infers the type of an expression, returning a display name
/// when one is apparent from the expression form alone (literals, constructors,
/// known function return types).
fn infer_expr_type(
    expr: &Expr,
    resolutions: &Resolutions,
    items: &ItemIndex,
    registry: &Registry,
) -> Option<String> {
    match &expr.kind {
        ExprKind::Lit(lit) => match &lit.kind {
            LitKind::Str(_) => Some("str".into()),
            LitKind::Int(_) => Some("i64".into()),
            LitKind::Float(_) => Some("f64".into()),
            LitKind::Bool(_) => Some("bool".into()),
            LitKind::ByteStr(_) => Some("bytes".into()),
            LitKind::Err => None,
        },
        ExprKind::List(_) => Some("list".into()),
        ExprKind::Map(_) => Some("map".into()),
        ExprKind::Record(_) => Some("record".into()),
        ExprKind::Tuple(elems) => {
            if elems.is_empty() {
                return Some("()".into());
            }
            let types: Vec<_> = elems.iter()
                .map(|e| infer_expr_type(e, resolutions, items, registry)
                    .unwrap_or_else(|| "_".into()))
                .collect();
            Some(format!("({})", types.join(", ")))
        }
        ExprKind::Struct(s) => {
            let res = resolutions.res.get(&s.path.id)?;
            match res {
                Res::Def(_, def) => Some(resolutions.defs.get(*def).name.to_string()),
                _ => None,
            }
        }
        ExprKind::Call(callee, _) => {
            if let ExprKind::Path(path) = &callee.kind {
                let res = resolutions.res.get(&path.id)?;
                match res {
                    Res::Def(DefKind::Fn, def) => {
                        if let Some(ItemRef::Fn { item, .. }) = items.get(*def) {
                            match &item.decl.output {
                                FnRetTy::Ty(ty) => Some(type_name(resolutions, ty)),
                                FnRetTy::Default(_) => Some("()".into()),
                            }
                        } else {
                            None
                        }
                    }
                    Res::Def(DefKind::NativeFn(id), _) => {
                        native_return_type(*id, registry)
                    }
                    Res::Def(DefKind::Struct | DefKind::NativeType(_), def) => {
                        Some(resolutions.defs.get(*def).name.to_string())
                    }
                    Res::Def(DefKind::Variant | DefKind::NativeVariant(_), def) => {
                        let parent = resolutions.defs.get(*def).parent?;
                        Some(resolutions.defs.get(parent).name.to_string())
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        ExprKind::MethodCall(mc) => {
            // Can't resolve method calls without full type info.
            let _ = mc;
            None
        }
        ExprKind::Paren(inner) => infer_expr_type(inner, resolutions, items, registry),
        ExprKind::Block(block) => {
            block.stmts.last().and_then(|stmt| {
                if let xur_ast::StmtKind::Expr(e) = &stmt.kind {
                    infer_expr_type(e, resolutions, items, registry)
                } else {
                    None
                }
            })
        }
        ExprKind::If(_, then_block, _) => {
            then_block.stmts.last().and_then(|stmt| {
                if let xur_ast::StmtKind::Expr(e) = &stmt.kind {
                    infer_expr_type(e, resolutions, items, registry)
                } else {
                    None
                }
            })
        }
        ExprKind::Match(_, arms) => {
            arms.first().and_then(|arm| infer_expr_type(&arm.body, resolutions, items, registry))
        }
        ExprKind::Path(_) => None,
        _ => None,
    }
}

/// The return type of a native function, mapped to a xur type name.
fn native_return_type(id: NativeFnId, registry: &Registry) -> Option<String> {
    let native_fn = registry.function(id);
    let sig = native_fn.signature.as_ref()?;
    Some(registry.shape_xur_name(sig.ret))
}

fn build_param_name_map(program: &Program) -> HashMap<DefId, Vec<Ident>> {
    let resolutions = program.resolutions();
    let mut map = HashMap::new();
    for module in &program.krate.modules {
        for item in &module.ast.items {
            if let ItemKind::Fn(f) = &item.kind
                && let Some(&def_id) = resolutions.node_defs.get(&item.id)
            {
                let names = param_idents(&f.decl.inputs);
                map.insert(def_id, names);
            }
            if let ItemKind::Impl(imp) = &item.kind {
                for impl_item in &imp.items {
                    if let ItemKind::Fn(f) = &impl_item.kind
                        && let Some(&def_id) = resolutions.node_defs.get(&impl_item.id)
                    {
                        let names = param_idents(&f.decl.inputs);
                        map.insert(def_id, names);
                    }
                }
            }
        }
    }
    map
}

fn param_idents(inputs: &[xur_ast::Param]) -> Vec<Ident> {
    inputs.iter().filter_map(|p| {
        if let PatKind::Ident(ident) = &p.pat.kind {
            Some(*ident)
        } else {
            None
        }
    }).collect()
}

const KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "else", "enum", "false", "fn", "for", "if", "impl",
    "in", "is", "let", "loop", "match", "pub", "return", "struct", "true", "use", "while",
];
