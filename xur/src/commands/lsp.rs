//! `xur lsp`: the Xur language server.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use xur_errors::{Diag, Level};
use xur_resolve::{Program, Res, names};
use xur_span::{OverlayFileLoader, SourceFile, SourceMap, Span};

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
                workspace_root: Mutex::new(None),
            });
            Server::new(stdin, stdout, socket).serve(service).await;
            Ok(())
        })
}

struct Backend {
    client: Client,
    documents: Mutex<HashMap<Url, String>>,
    state: Mutex<Option<AnalysisState>>,
    workspace_root: Mutex<Option<PathBuf>>,
}

struct AnalysisState {
    program: Program,
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

                *self.state.lock().unwrap() = Some(AnalysisState {
                    program,
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
        if let Some(ws) = self.workspace_root.lock().unwrap().as_ref() {
            let main = ws.join("main.xur");
            if main.is_file() {
                return Some(main);
            }
        }
        uri_to_path(changed_uri)
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Some(uri) = params.root_uri
            && let Some(path) = uri_to_path(&uri)
        {
            *self.workspace_root.lock().unwrap() = Some(path);
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
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
