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

use xur_ast::{Expr, ExprKind, NodeId};
use xur_ast::visit::Visitor;
use xur_bytecode::ItemIndex;
use xur_resolve::{DefKind, Program, Res};
use xur_runtime::Registry;
use xur_span::{OverlayFileLoader, Span};

use hints::{
    HintCollector, InferredTy, LetBinding,
    build_param_name_map, infer_expr_type, infer_iter_element_type, infer_match_arm_type,
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

        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let Some(offset) = uri_position_to_offset(sm, uri, pos) else {
            return Ok(None);
        };
        Ok(definition_at(program, offset).map(GotoDefinitionResponse::Scalar))
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

        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;

        let prefix = {
            let docs = self.documents.lock().unwrap();
            docs.get(uri).and_then(|src| line_prefix(src, pos))
        };

        let items = completions(program, uri, prefix.as_deref());
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

        let lang = program.lang_items();
        let param_names = build_param_name_map(program);
        let mut hints = Vec::new();

        for module in &program.krate.modules {
            let mut collector = HintCollector::new();
            collector.visit_module(module.ast);

            for (callee_id, args) in collector.calls {
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

            let mut local_types: HashMap<NodeId, InferredTy> = HashMap::new();
            let mut local_inits: HashMap<NodeId, &Expr> = HashMap::new();

            for binding in &collector.lets {
                if let Some(id) = binding.binding_id {
                    local_inits.insert(id, binding.init);
                }
            }

            // Merge let and for-loop bindings so they are processed in source
            // order.  This ensures a for-loop variable (e.g. `ch`) is in
            // local_types before let bindings inside its body are inferred.
            enum Binding<'a> {
                Let(&'a LetBinding<'a>),
                For(usize, Option<NodeId>, &'a Expr),
            }
            let mut bindings: Vec<(usize, Binding<'_>)> = Vec::new();
            for b in &collector.lets {
                bindings.push((b.pat_end, Binding::Let(b)));
            }
            for &(pat_end, binding_id, iter) in &collector.for_loops {
                bindings.push((pat_end, Binding::For(pat_end, binding_id, iter)));
            }
            bindings.sort_by_key(|(pos, _)| *pos);

            for (_, binding) in &bindings {
                match binding {
                    Binding::Let(binding) => {
                        if let Some(ann) = binding.ty {
                            if let Some(id) = binding.binding_id {
                                local_types.insert(id, InferredTy::from_ast(ann, resolutions));
                            }
                        } else if let Some(ty) = infer_expr_type(binding.init, resolutions, &state.items, &state.registry, &local_types) {
                            if let Some(id) = binding.binding_id {
                                local_types.insert(id, ty.clone());
                            }
                            if !matches!(ty, InferredTy::Infer | InferredTy::Module)
                                && in_file(Span { start: binding.pat_end, end: binding.pat_end })
                            {
                                let pos = file.line_col(binding.pat_end);
                                let label = format!(": {}", ty.display(resolutions));
                                hints.push(InlayHint {
                                    position: Position::new(pos.line - 1, pos.col - 1),
                                    label: InlayHintLabel::String(label),
                                    kind: Some(InlayHintKind::TYPE),
                                    padding_left: None,
                                    padding_right: None,
                                    text_edits: None,
                                    tooltip: None,
                                    data: None,
                                });
                            }
                        }
                    }
                    Binding::For(pat_end, binding_id, iter) => {
                        let effective_iter = if let ExprKind::Path(path) = &iter.kind
                            && let Some(Res::Local(node_id)) = resolutions.res.get(&path.id)
                            && let Some(init) = local_inits.get(node_id)
                        {
                            init
                        } else {
                            iter
                        };
                        if let Some(ty) = infer_iter_element_type(effective_iter, resolutions, &state.items, &state.registry, &local_types) {
                            if let Some(id) = binding_id {
                                local_types.insert(*id, ty.clone());
                            }
                            if !matches!(ty, InferredTy::Infer | InferredTy::Module)
                                && in_file(Span { start: *pat_end, end: *pat_end })
                            {
                                let pos = file.line_col(*pat_end);
                                let label = format!(": {}", ty.display(resolutions));
                                hints.push(InlayHint {
                                    position: Position::new(pos.line - 1, pos.col - 1),
                                    label: InlayHintLabel::String(label),
                                    kind: Some(InlayHintKind::TYPE),
                                    padding_left: None,
                                    padding_right: None,
                                    text_edits: None,
                                    tooltip: None,
                                    data: None,
                                });
                            }
                        }
                    }
                }
            }

            for (scrutinee_end, scrutinee) in &collector.matches {
                if !in_file(Span { start: *scrutinee_end, end: *scrutinee_end }) {
                    continue;
                }
                if let Some(ty) = infer_expr_type(scrutinee, resolutions, &state.items, &state.registry, &local_types)
                    && !matches!(ty, InferredTy::Infer | InferredTy::Module)
                {
                    let pos = file.line_col(*scrutinee_end);
                    let label = format!(": {}", ty.display(resolutions));
                    hints.push(InlayHint {
                        position: Position::new(pos.line - 1, pos.col - 1),
                        label: InlayHintLabel::String(label),
                        kind: Some(InlayHintKind::TYPE),
                        padding_left: Some(true),
                        padding_right: None,
                        text_edits: None,
                        tooltip: None,
                        data: None,
                    });
                }
            }

            for &(pat_end, binding_id, scrutinee, variant_path_id) in &collector.match_arms {
                if !in_file(Span { start: pat_end, end: pat_end }) {
                    continue;
                }
                let variant_def = match resolutions.res.get(&variant_path_id) {
                    Some(Res::Def(DefKind::Variant, def)) => *def,
                    _ => continue,
                };
                let ty = infer_match_arm_type(
                    scrutinee, variant_def, &lang,
                    resolutions, &state.items, &state.registry, &local_types,
                );
                if let Some(ty) = ty {
                    local_types.insert(binding_id, ty.clone());
                    if !matches!(ty, InferredTy::Infer) {
                        let pos = file.line_col(pat_end);
                        let label = format!(": {}", ty.display(resolutions));
                        hints.push(InlayHint {
                            position: Position::new(pos.line - 1, pos.col - 1),
                            label: InlayHintLabel::String(label),
                            kind: Some(InlayHintKind::TYPE),
                            padding_left: None,
                            padding_right: None,
                            text_edits: None,
                            tooltip: None,
                            data: None,
                        });
                    }
                }
            }

            for (span_end, method_start, receiver) in &collector.chains {
                if !in_file(Span { start: *span_end, end: *span_end }) {
                    continue;
                }
                let receiver_end_line = file.line_col(*span_end).line;
                let method_line = file.line_col(*method_start).line;
                if receiver_end_line == method_line {
                    continue;
                }
                if let Some(ty) = infer_expr_type(receiver, resolutions, &state.items, &state.registry, &local_types)
                    && !matches!(ty, InferredTy::Infer | InferredTy::Module)
                {
                    let pos = file.line_col(*span_end);
                    let label = format!(" {}", ty.display(resolutions));
                    hints.push(InlayHint {
                        position: Position::new(pos.line - 1, pos.col - 1),
                        label: InlayHintLabel::String(label),
                        kind: Some(InlayHintKind::TYPE),
                        padding_left: None,
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

#[cfg(test)]
mod tests {
    //! Cross-file analysis for static-string `std::import`s: a
    //! `const m = std::import("./other.xur")` makes `m::Item` resolve into
    //! `other.xur`, so navigation and completion reach across files.

    use super::*;
    use std::path::Path;
    use xur_span::MemoryFileLoader;

    /// The module the example `example/lang/main.xur` imports, trimmed to the
    /// public and private items the tests exercise.
    const MATH: &str = "\
pub enum MathError { DivideByZero Overflow }
pub fn add(a: u32, b: u32) -> u32 { a + b }
pub fn div(a: u32, b: u32) -> Result<u32, MathError> {
    if b == 0 { Err(MathError::DivideByZero) } else { Ok(a / b) }
}
fn secret() {}
";

    fn load(files: &[(&str, &str)]) -> Program {
        let loader = Box::new(MemoryFileLoader::new(files.iter().copied()));
        let registry = Arc::new(xur_std::registry());
        let natives = Box::new(Arc::clone(&registry));
        let (program, _diags) =
            Program::load(Path::new(files[0].0), loader, natives, Some(xur_std::PRELUDE)).unwrap();
        program
    }

    /// The global offset of the first occurrence of `needle` in the file whose
    /// name ends with `file`.
    fn offset_of(program: &Program, file: &str, needle: &str) -> usize {
        let sf = program
            .krate
            .source_map
            .files()
            .iter()
            .find(|f| f.name.ends_with(file))
            .unwrap_or_else(|| panic!("no file ending `{file}`"));
        let pos = sf
            .src
            .find(needle)
            .unwrap_or_else(|| panic!("`{needle}` not in `{file}`"));
        sf.start + pos
    }

    /// The source text the location points at.
    fn text_at(program: &Program, loc: &Location) -> String {
        let sm = &program.krate.source_map;
        let sf = find_file_by_uri(sm, &loc.uri).expect("located file");
        let start = position_to_offset(sf, loc.range.start).unwrap();
        let end = position_to_offset(sf, loc.range.end).unwrap();
        sm.slice(Span { start, end }).to_owned()
    }

    fn uri(path: &str) -> Url {
        Url::from_file_path(path).unwrap()
    }

    #[test]
    fn goto_definition_jumps_into_imported_file() {
        let program = load(&[
            (
                "/proj/main.xur",
                "const math = std::import(\"./math.xur\");\n\
                 fn f() -> math::MathError { math::MathError::Overflow }\n",
            ),
            ("/proj/math.xur", MATH),
        ]);

        // `math::MathError` in the return type jumps to the enum in math.xur.
        let at = offset_of(&program, "main.xur", "math::MathError {");
        let loc = definition_at(&program, at).expect("goto resolves");
        assert!(
            loc.uri.as_str().ends_with("math.xur"),
            "expected math.xur, got {}",
            loc.uri
        );
        assert_eq!(text_at(&program, &loc), "MathError");

        // A nested `math::MathError::Overflow` path jumps to the variant.
        let at = offset_of(&program, "main.xur", "math::MathError::Overflow");
        let loc = definition_at(&program, at).expect("goto resolves");
        assert!(loc.uri.as_str().ends_with("math.xur"));
        assert_eq!(text_at(&program, &loc), "Overflow");
    }

    #[test]
    fn goto_definition_on_the_module_alias_opens_the_file() {
        let program = load(&[
            (
                "/proj/main.xur",
                "const math = std::import(\"./math.xur\");\nfn f() { math::add(1, 2); }\n",
            ),
            ("/proj/math.xur", MATH),
        ]);
        // The cursor on `math` of `math::add` resolves the whole path to `add`.
        let at = offset_of(&program, "main.xur", "math::add");
        let loc = definition_at(&program, at).expect("goto resolves");
        assert!(loc.uri.as_str().ends_with("math.xur"));
        assert_eq!(text_at(&program, &loc), "add");
    }

    #[test]
    fn completion_offers_only_public_items_of_an_imported_module() {
        let program = load(&[
            (
                "/proj/main.xur",
                "const math = std::import(\"./math.xur\");\nfn f() { let e = math:: }\n",
            ),
            ("/proj/math.xur", MATH),
        ]);
        let items = completions(&program, &uri("/proj/main.xur"), Some("    let e = math::"));
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"MathError"), "missing MathError: {labels:?}");
        assert!(labels.contains(&"add"), "missing add: {labels:?}");
        assert!(labels.contains(&"div"), "missing div: {labels:?}");
        // `secret` is private, so a `math::secret` path is rejected; it must
        // not be offered.
        assert!(!labels.contains(&"secret"), "private item offered: {labels:?}");
    }

    #[test]
    fn missing_import_is_diagnosed_at_the_path_literal() {
        let loader = Box::new(MemoryFileLoader::new([(
            "/proj/main.xur",
            "const gone = std::import(\"./gone.xur\");\n",
        )]));
        let registry = Arc::new(xur_std::registry());
        let natives = Box::new(Arc::clone(&registry));
        let (program, diags) = Program::load(
            Path::new("/proj/main.xur"),
            loader,
            natives,
            Some(xur_std::PRELUDE),
        )
        .unwrap();

        let grouped = group_diagnostics(&program.krate.source_map, &diags);
        let main = uri("/proj/main.xur");
        let file_diags = grouped.get(&main).expect("diagnostic on main.xur");
        assert_eq!(file_diags.len(), 1);
        let diag = &file_diags[0];
        assert!(
            diag.message.contains("gone.xur"),
            "message: {}",
            diag.message
        );
        // The range covers the `"./gone.xur"` string literal.
        let sf = find_file_by_uri(&program.krate.source_map, &main).unwrap();
        let start = position_to_offset(sf, diag.range.start).unwrap();
        let end = position_to_offset(sf, diag.range.end).unwrap();
        assert_eq!(
            program.krate.source_map.slice(Span { start, end }),
            "\"./gone.xur\""
        );
    }
}
