//! Minimal LSP server exposing CSV Align/Shrink as explicit actions.
//!
//! Capabilities (deliberately narrow — Align is destructive, so it is never a
//! formatter; cf. VS Code Rainbow CSV treating Align as an explicit command):
//! - `textDocument/codeAction` -> lightweight Align/Shrink actions; edits are
//!   computed on demand via `codeAction/resolve`, so cursor movement doesn't
//!   pay for whole-document rewrites.
//! - `textDocument/inlayHint` -> virtual-align pads from a per-version cache,
//!   served for the requested range only.
//!
//! Dialect comes from the LSP `languageId` (mapped explicitly in
//! `extension.toml`), falling back to language name and file extension.

use std::collections::HashMap;
use std::error::Error;

use lsp_server::{Connection, Message, Request, RequestId, Response};
use lsp_types::*;
use rainbow_csv_core::{align_document, pads_for_range, shrink_document, Dialect, DocAnalysis};
use serde_json::json;

fn verbose() -> bool {
    std::env::var("CSV_LSP_VERBOSE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn dialect_for(language_id: &str, uri: &Url) -> Dialect {
    if let Some(d) = Dialect::from_language_id(language_id) {
        return d;
    }
    if let Some(d) = Dialect::from_language_name(language_id) {
        return d;
    }
    let path = uri.path();
    if let Some(ext) = path.rsplit('.').next() {
        if let Some(d) = Dialect::from_extension(ext) {
            return d;
        }
    }
    Dialect::Csv
}

fn full_range(text: &str) -> Range {
    // `character` is UTF-16 units (astral chars count 2). `\r` is not part of
    // the editor's line content, so it doesn't advance the column.
    let mut line = 0u32;
    let mut character = 0u32;
    for ch in text.chars() {
        if ch == '\n' {
            line += 1;
            character = 0;
        } else if ch != '\r' {
            character += ch.len_utf16() as u32;
        }
    }
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position { line, character },
    }
}

/// Whole-document replacement edit, or `None` when already in the target state.
fn align_edit(text: &str, dialect: Dialect) -> Option<Vec<TextEdit>> {
    let aligned = align_document(text, dialect);
    if aligned == text {
        return None;
    }
    Some(vec![TextEdit {
        range: full_range(text),
        new_text: aligned,
    }])
}

fn shrink_edit(text: &str, dialect: Dialect) -> Option<Vec<TextEdit>> {
    let (shrunk, changed) = shrink_document(text, dialect);
    if !changed {
        return None;
    }
    Some(vec![TextEdit {
        range: full_range(text),
        new_text: shrunk,
    }])
}

/// Per-document state, parsed once per sync. Inlay hints and the cheap
/// list-time action gating both read this cache.
struct DocState {
    language_id: String,
    text: String,
    dialect: Dialect,
    analysis: DocAnalysis,
}

impl DocState {
    fn new(language_id: String, text: String, uri: &Url) -> Self {
        let dialect = dialect_for(&language_id, uri);
        let analysis = rainbow_csv_core::analyze_document(&text, dialect);
        Self {
            language_id,
            text,
            dialect,
            analysis,
        }
    }

    /// Whether offering Align/Shrink is safe: no unbalanced quotes.
    fn editable(&self) -> bool {
        !self.analysis.has_warnings
    }
}

fn send_ok(connection: &Connection, id: RequestId, result: serde_json::Value) {
    let resp = Response {
        id,
        result: Some(result),
        error: None,
    };
    let _ = connection.sender.send(Message::Response(resp));
}

fn send_error(connection: &Connection, id: RequestId, msg: String) {
    let resp = Response {
        id,
        result: None,
        error: Some(lsp_server::ResponseError {
            code: lsp_server::ErrorCode::InternalError as i32,
            message: msg,
            data: None,
        }),
    };
    let _ = connection.sender.send(Message::Response(resp));
}

fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();
    let server_caps = serde_json::to_value(ServerCapabilities {
        // No formatting providers: Align pads cell values, which changes what
        // downstream consumers (e.g. pandas) read. It stays an explicit action.
        code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
            code_action_kinds: Some(vec![CodeActionKind::SOURCE]),
            resolve_provider: Some(true),
            work_done_progress_options: Default::default(),
        })),
        inlay_hint_provider: Some(OneOf::Left(true)),
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        ..Default::default()
    })
    .unwrap();
    let init_params = connection.initialize(server_caps)?;
    let _init: InitializeParams = serde_json::from_value(init_params)?;

    let mut docs: HashMap<Url, DocState> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    break;
                }
                // Every request must get a response; otherwise the client
                // waits forever and the UI looks dead.
                let id = req.id.clone();
                if let Err(e) = handle_request(&connection, req, &mut docs) {
                    eprintln!("csv-lsp request error: {e}");
                    send_error(&connection, id, e.to_string());
                }
            }
            Message::Response(_) => {}
            Message::Notification(note) => {
                handle_notification(note, &mut docs);
            }
        }
    }
    io_threads.join()?;
    Ok(())
}

fn handle_notification(note: lsp_server::Notification, docs: &mut HashMap<Url, DocState>) {
    match note.method.as_str() {
        "textDocument/didOpen" => {
            if let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(note.params) {
                let doc = p.text_document;
                docs.insert(
                    doc.uri.clone(),
                    DocState::new(doc.language_id, doc.text, &doc.uri),
                );
            }
        }
        "textDocument/didChange" => {
            // FULL sync: single entry with the whole text. Rebuild the cache.
            if let Ok(p) = serde_json::from_value::<DidChangeTextDocumentParams>(note.params) {
                if let Some(last) = p.content_changes.into_iter().last() {
                    let lang = docs
                        .get(&p.text_document.uri)
                        .map(|s| s.language_id.clone())
                        .unwrap_or_else(|| String::from("csv"));
                    docs.insert(
                        p.text_document.uri.clone(),
                        DocState::new(lang, last.text, &p.text_document.uri),
                    );
                }
            }
        }
        "textDocument/didClose" => {
            if let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(note.params) {
                docs.remove(&p.text_document.uri);
            }
        }
        _ => {}
    }
}

/// `data` payload carried by our code actions so `resolve` can find the
/// document and the requested operation.
fn action_data(op: &str, uri: &Url) -> serde_json::Value {
    json!({ "op": op, "uri": uri.as_str() })
}

fn unresolved_action(title: &str, op: &str, uri: &Url) -> CodeActionOrCommand {
    CodeActionOrCommand::CodeAction(CodeAction {
        title: title.to_string(),
        kind: Some(CodeActionKind::SOURCE),
        data: Some(action_data(op, uri)),
        ..Default::default()
    })
}

fn handle_request(
    connection: &Connection,
    req: Request,
    docs: &mut HashMap<Url, DocState>,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match req.method.as_str() {
        "textDocument/codeAction" => {
            let p: CodeActionParams = serde_json::from_value(req.params)?;
            let uri = &p.text_document.uri;
            // List time is O(1): offer from cached flags, compute edits only
            // on resolve. Never offer on unbalanced input.
            let mut actions: Vec<CodeActionOrCommand> = Vec::new();
            if let Some(doc) = docs.get(uri) {
                if doc.editable() {
                    if doc.analysis.needs_align {
                        actions.push(unresolved_action(
                            "Align CSV columns (spaces)",
                            "align",
                            uri,
                        ));
                    }
                    if doc.analysis.needs_shrink {
                        actions.push(unresolved_action(
                            "Shrink CSV columns (trim spaces)",
                            "shrink",
                            uri,
                        ));
                    }
                }
            }
            if verbose() {
                eprintln!("csv-lsp codeAction uri={uri} actions={}", actions.len());
            }
            send_ok(connection, req.id, serde_json::to_value(actions)?);
        }
        "codeAction/resolve" => {
            let t0 = std::time::Instant::now();
            let mut action: CodeAction = serde_json::from_value(req.params)?;
            let op = action
                .data
                .as_ref()
                .and_then(|d| d.get("op"))
                .and_then(|o| o.as_str())
                .unwrap_or("");
            let uri: Option<Url> = action
                .data
                .as_ref()
                .and_then(|d| d.get("uri"))
                .and_then(|u| u.as_str())
                .and_then(|s| s.parse().ok());
            if let Some(uri) = uri {
                if let Some(doc) = docs.get(&uri) {
                    if doc.editable() {
                        let edit = match op {
                            "shrink" => shrink_edit(&doc.text, doc.dialect),
                            "align" => align_edit(&doc.text, doc.dialect),
                            _ => None, // unknown op: resolve without an edit
                        };
                        action.edit = edit.map(|edits| WorkspaceEdit {
                            changes: Some([(uri.clone(), edits)].into_iter().collect()),
                            ..Default::default()
                        });
                    }
                }
            }
            if verbose() || t0.elapsed().as_millis() > 250 {
                eprintln!("csv-lsp resolve op={op} ms={}", t0.elapsed().as_millis());
            }
            send_ok(connection, req.id, serde_json::to_value(action)?);
        }
        "textDocument/inlayHint" => {
            let t0 = std::time::Instant::now();
            let p: InlayHintParams = serde_json::from_value(req.params)?;
            let mut hints = Vec::new();
            if let Some(doc) = docs.get(&p.text_document.uri) {
                if doc.editable() {
                    let range = p.range;
                    for pad in pads_for_range(&doc.analysis, range.start.line, range.end.line) {
                        hints.push(InlayHint {
                            position: Position {
                                line: pad.line,
                                character: pad.col,
                            },
                            label: InlayHintLabel::String(" ".repeat(pad.spaces)),
                            kind: None,
                            text_edits: None,
                            tooltip: None,
                            padding_left: None,
                            padding_right: None,
                            data: None,
                        });
                    }
                }
            }
            // Cap to avoid flooding the client on huge files (mirrors VS Code limit).
            hints.truncate(1500);
            let ms = t0.elapsed().as_millis();
            // Quiet on the hot path; slow hits indicate cache misses or storms.
            if ms > 250 {
                eprintln!(
                    "csv-lsp slow inlayHint uri={} hints={} ms={}",
                    p.text_document.uri,
                    hints.len(),
                    ms
                );
            }
            send_ok(connection, req.id, serde_json::to_value(hints)?);
        }
        _ => {
            let resp = Response {
                id: req.id,
                result: None,
                error: Some(lsp_server::ResponseError {
                    code: lsp_server::ErrorCode::MethodNotFound as i32,
                    message: "unhandled".to_string(),
                    data: None,
                }),
            };
            let _ = connection.sender.send(Message::Response(resp));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_range_covers_trailing_newline() {
        // lines() drops trailing "\n"s; the range must still reach end of doc,
        // otherwise each Format leaves a duplicated blank line behind.
        let r = full_range("a");
        assert_eq!((r.start.line, r.start.character), (0, 0));
        assert_eq!((r.end.line, r.end.character), (0, 1));
        let r = full_range("");
        assert_eq!((r.end.line, r.end.character), (0, 0));
        let r = full_range("a,bb,ccc\ndddd,e,f\n");
        assert_eq!((r.end.line, r.end.character), (2, 0));
        let r = full_range("a\n\n");
        assert_eq!((r.end.line, r.end.character), (2, 0));
        let r = full_range("a,bb\r\nccc,d\r\n");
        assert_eq!((r.end.line, r.end.character), (2, 0));
    }

    #[test]
    fn full_range_counts_utf16() {
        // LSP columns are UTF-16 units; astral chars count 2.
        let r = full_range("a,é\n");
        assert_eq!((r.end.line, r.end.character), (1, 0) /* trailing NL */);
        let r = full_range("😀");
        assert_eq!((r.end.line, r.end.character), (0, 2));
        let r = full_range("a,😀");
        assert_eq!((r.end.line, r.end.character), (0, 4));
    }

    #[test]
    fn docstate_caches_widths() {
        let uri = Url::parse("file:///C:/t.csv").unwrap();
        let doc = DocState::new(String::from("csv"), String::from("a,bb\ncccc,d\n"), &uri);
        assert_eq!(doc.dialect, Dialect::Csv);
        assert_eq!(doc.analysis.widths, vec![4, 2]);
        assert_eq!(doc.analysis.records.len(), 2);
        assert!(doc.editable());
        assert!(doc.analysis.needs_align);
        assert!(!doc.analysis.needs_shrink);
    }

    #[test]
    fn docstate_flags_unbalanced() {
        let uri = Url::parse("file:///C:/t.csv").unwrap();
        let doc = DocState::new(String::from("csv"), String::from("a,b\n1,\"oops,2\n"), &uri);
        assert!(!doc.editable());
    }
}
