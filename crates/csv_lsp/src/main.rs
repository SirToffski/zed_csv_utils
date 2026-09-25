//! Minimal LSP server exposing CSV Align/Shrink.
//!
//! Capabilities:
//! - `textDocument/formatting` and `rangeFormatting` -> whitespace align
//! - `workspace/executeCommand` `csv.shrink` / `csv.align` -> edits
//! - `textDocument/inlayHint` -> virtual align pads (spaces)
//! - `textDocument/codeAction` -> quick actions for Align/Shrink
//!
//! Dialect is derived from LSP `languageId` (Zed language `name`) with
//! fallback to file extension; unknown -> CSV comma.

use std::collections::HashMap;
use std::error::Error;

use csv_core::{align_document, column_widths, pads_for_range, parse_trimmed_records, shrink_document, Dialect};
use lsp_server::{Connection, Message, Request, RequestId, Response};
use lsp_types::*;

fn dialect_for(language_id: &str, uri: &Url) -> Dialect {
    if let Some(d) = Dialect::from_language_name(language_id) {
        return d;
    }
    let path = uri.path();
    if let Some(ext) = path.rsplit('.').next() {
        if let Some(d) = Dialect::from_extension(ext) {
            return d;
        }
        if path.ends_with(".tsv") || path.ends_with(".tab") {
            return Dialect::Tsv;
        }
    }
    // Heuristic: tab-separated content defaults to TSV.
    Dialect::Csv
}

fn full_range(text: &str) -> Range {
    // `str::lines()` drops trailing newlines, so compute the end position by
    // scanning: each '\n' starts a new line. (A range that stops short leaves
    // a duplicated trailing newline behind after every Format.)
    let mut line = 0u32;
    let mut character = 0u32;
    for ch in text.chars() {
        if ch == '\n' {
            line += 1;
            character = 0;
        } else if ch != '\r' {
            character += 1;
        }
    }
    Range {
        start: Position { line: 0, character: 0 },
        end: Position { line, character },
    }
}

fn align_edits(text: &str, dialect: Dialect) -> Vec<TextEdit> {
    let aligned = align_document(text, dialect);
    if aligned == text {
        return vec![];
    }
    vec![TextEdit { range: full_range(text), new_text: aligned }]
}

fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();
    let server_caps = serde_json::to_value(ServerCapabilities {
        document_formatting_provider: Some(OneOf::Left(true)),
        document_range_formatting_provider: Some(OneOf::Left(true)),
        inlay_hint_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
        execute_command_provider: Some(ExecuteCommandOptions {
            commands: vec!["csv.shrink".to_string(), "csv.align".to_string()],
            work_done_progress_options: Default::default(),
        }),
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        ..Default::default()
    })
    .unwrap();
    let init_params = connection.initialize(server_caps)?;
    let _init: InitializeParams = serde_json::from_value(init_params)?;

    let mut docs: HashMap<Url, DocState> = HashMap::new(); // uri -> cached document state

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    break;
                }
                // Every request must get a response; otherwise the client
                // waits forever and the UI looks dead ("nothing happens").
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

/// Per-document state. Records + widths are parsed once per sync instead of
/// once per request, so scroll-triggered inlay hints on big files answer from
/// cache instead of re-parsing the whole document (which would starve later
/// requests behind a single-threaded queue).
struct DocState {
    language_id: String,
    text: String,
    dialect: Dialect,
    records: Vec<Vec<String>>,
    widths: Vec<usize>,
}

impl DocState {
    fn new(language_id: String, text: String, uri: &Url) -> Self {
        let dialect = dialect_for(&language_id, uri);
        let records = parse_trimmed_records(&text, dialect);
        let widths = column_widths(&records);
        Self { language_id, text, dialect, records, widths }
    }
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

fn handle_notification(note: lsp_server::Notification, docs: &mut HashMap<Url, DocState>) {
    match note.method.as_str() {
        "textDocument/didOpen" => {
            if let Ok(p) = serde_json::from_value::<DidOpenTextDocumentParams>(note.params) {
                let doc = p.text_document;
                docs.insert(doc.uri.clone(), DocState::new(doc.language_id, doc.text, &doc.uri));
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

fn send_ok(connection: &Connection, id: RequestId, result: serde_json::Value) {
    let resp = Response { id, result: Some(result), error: None };
    let _ = connection.sender.send(Message::Response(resp));
}

fn handle_request(
    connection: &Connection,
    req: Request,
    docs: &mut HashMap<Url, DocState>,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match req.method.as_str() {
        "textDocument/formatting" => {
            let t0 = std::time::Instant::now();
            let p: DocumentFormattingParams = serde_json::from_value(req.params)?;
            // If we never saw didOpen (some clients), fall back to empty -> no-op.
            let fallback = DocState::new(String::from("csv"), String::new(), &p.text_document.uri);
            let doc = docs.get(&p.text_document.uri).unwrap_or(&fallback);
            let edits = align_edits(&doc.text, doc.dialect);
            eprintln!("csv-lsp formatting uri={} edits={} ms={}", p.text_document.uri, edits.len(), t0.elapsed().as_millis());
            send_ok(connection, req.id, serde_json::to_value(edits)?);
        }
        "textDocument/rangeFormatting" => {
            let p: DocumentRangeFormattingParams = serde_json::from_value(req.params)?;
            let fallback = DocState::new(String::from("csv"), String::new(), &p.text_document.uri);
            let doc = docs.get(&p.text_document.uri).unwrap_or(&fallback);
            // MVP: align whole doc even for range requests.
            let edits = align_edits(&doc.text, doc.dialect);
            send_ok(connection, req.id, serde_json::to_value(edits)?);
        }
        "textDocument/inlayHint" => {
            let t0 = std::time::Instant::now();
            let p: InlayHintParams = serde_json::from_value(req.params)?;
            let fallback = DocState::new(String::from("csv"), String::new(), &p.text_document.uri);
            let doc = docs.get(&p.text_document.uri).unwrap_or(&fallback);
            let range = p.range;
            let pads = pads_for_range(&doc.records, &doc.widths, range.start.line, range.end.line);
            let mut hints = Vec::new();
            for pad in pads {
                hints.push(InlayHint {
                    position: Position { line: pad.line, character: pad.col },
                    label: InlayHintLabel::String(" ".repeat(pad.spaces)),
                    kind: Some(InlayHintKind::PARAMETER),
                    text_edits: None,
                    tooltip: None,
                    padding_left: None,
                    padding_right: None,
                    data: None,
                });
            }
            // Cap to avoid flooding the client on huge files (mirrors VS Code limit).
            hints.truncate(1500);
            let ms = t0.elapsed().as_millis();
            // Quiet on the hot path; slow hits indicate cache misses or storms.
            if ms > 250 {
                eprintln!("csv-lsp slow inlayHint uri={} hints={} ms={}", p.text_document.uri, hints.len(), ms);
            }
            send_ok(connection, req.id, serde_json::to_value(hints)?);
        }
        "textDocument/codeAction" => {
            let t0 = std::time::Instant::now();
            let p: CodeActionParams = serde_json::from_value(req.params)?;
            let fallback = DocState::new(String::from("csv"), String::new(), &p.text_document.uri);
            let doc = docs.get(&p.text_document.uri).unwrap_or(&fallback);
            let text: &str = &doc.text;
            let d = doc.dialect;
            let aligned = align_document(text, d);
            let (shrunk, shrink_changed) = shrink_document(text, d);
            let mut actions: Vec<CodeActionOrCommand> = Vec::new();
            if aligned != text {
                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: "Align CSV columns (spaces)".to_string(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    edit: Some(WorkspaceEdit {
                        changes: Some(
                            [(p.text_document.uri.clone(), align_edits(text, d))]
                                .into_iter()
                                .collect(),
                        ),
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
            }
            if shrink_changed {
                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: "Shrink CSV columns (trim spaces)".to_string(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    edit: Some(WorkspaceEdit {
                        changes: Some(
                            [(
                                p.text_document.uri.clone(),
                                vec![TextEdit {
                                    range: full_range(text),
                                    new_text: shrunk,
                                }],
                            )]
                            .into_iter()
                            .collect(),
                        ),
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
            }
            eprintln!("csv-lsp codeAction uri={} actions={} ms={}", p.text_document.uri, actions.len(), t0.elapsed().as_millis());
            send_ok(connection, req.id, serde_json::to_value(actions)?);
        }
        "workspace/executeCommand" => {
            let p: ExecuteCommandParams = serde_json::from_value(req.params)?;
            // args[0] expected to be TextDocumentIdentifier or Uri.
            let uri: Option<Url> = p
                .arguments
                .first()
                .and_then(|v| serde_json::from_value::<TextDocumentIdentifier>(v.clone()).ok())
                .map(|t| t.uri)
                .or_else(|| {
                    p.arguments
                        .first()
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                });
            if let Some(uri) = uri {
                if let Some(doc) = docs.get(&uri) {
                    let text: &str = &doc.text;
                    let d = doc.dialect;
                    let new_text = match p.command.as_str() {
                        "csv.shrink" => shrink_document(text, d).0,
                        _ => align_document(text, d),
                    };
                    let edit = WorkspaceEdit {
                        changes: Some(
                            [(uri, vec![TextEdit { range: full_range(text), new_text }])]
                                .into_iter()
                                .collect(),
                        ),
                        ..Default::default()
                    };
                    // executeCommand returns the WorkspaceEdit for the client to apply.
                    send_ok(connection, req.id, serde_json::to_value(edit)?);
                    eprintln!("csv-lsp executeCommand command={}", p.command);
                    return Ok(());
                }
            }
            send_ok(connection, req.id, serde_json::Value::Null);
        }
        _ => {
            let resp = Response { id: req.id, result: None, error: Some(lsp_server::ResponseError {
                code: lsp_server::ErrorCode::MethodNotFound as i32,
                message: "unhandled".to_string(),
                data: None,
            }) };
            let _ = connection.sender.send(Message::Response(resp));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docstate_caches_widths() {
        let uri = Url::parse("file:///C:/t.csv").unwrap();
        let doc = DocState::new(String::from("Rainbow CSV (,)"), String::from("a,bb\ncccc,d\n"), &uri);
        assert_eq!(doc.dialect, Dialect::Csv);
        assert_eq!(doc.widths, vec![4, 2]);
        assert_eq!(doc.records.len(), 2);
    }

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
}
