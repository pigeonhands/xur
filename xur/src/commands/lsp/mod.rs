//! `xur lsp`: the Xur language server.

mod hints;
mod hover;
mod util;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use xur_ast::visit::Visitor;
use xur_bytecode::ItemIndex;
use xur_resolve::{DefKind, Namespace, Program, Res};
use xur_runtime::Registry;
use xur_span::{OverlayFileLoader, Span};

use hints::{
    CallCollector, LetCollector, MatchCollector, build_param_name_map, infer_expr_type,
    method_at_offset, method_signature,
};
use util::*;

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

        match Program::load(&root, loader, natives, Some(xur_std::PRELUDE)) {
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

        // Try resolved names first (paths, patterns).
        if let Some(name) = name_at_offset(program, offset)
            && let Some(&res) = resolutions.res.get(&name.id)
        {
            let description = hover::describe(
                res, resolutions, &state.items, &state.registry, sm,
            );
            let (_, range) = span_to_range(sm, name.span);
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("```xur\n{description}\n```"),
                }),
                range: Some(range),
            }));
        }

        // Fall back to method calls (dispatched at runtime, not in the name map).
        if let Some(mc) = method_at_offset(program, &state.items, &state.registry, offset)
            && let Some(sig) = method_signature(
                mc.receiver_ty.as_deref(), &mc.method_name, &state.registry,
            )
        {
            let (_, range) = span_to_range(sm, mc.method_span);
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("```xur\n{sig}\n```"),
                }),
                range: Some(range),
            }));
        }

        Ok(None)
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

        use xur_resolve::names;
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
        } else if let Some(module_def) = module_def {
            for (name, res) in resolutions.scope_entries(module_def, Namespace::Value) {
                items.push(CompletionItem {
                    label: name.as_str().to_owned(),
                    kind: Some(completion_kind(res)),
                    ..Default::default()
                });
            }
            for (name, res) in resolutions.scope_entries(module_def, Namespace::Type) {
                items.push(CompletionItem {
                    label: name.as_str().to_owned(),
                    kind: Some(completion_kind(res)),
                    ..Default::default()
                });
            }
            for &kw in KEYWORDS {
                items.push(CompletionItem {
                    label: kw.to_owned(),
                    kind: Some(CompletionItemKind::KEYWORD),
                    ..Default::default()
                });
            }
        }

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
        let Some(file) = find_file_by_uri(sm, uri) else {
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
