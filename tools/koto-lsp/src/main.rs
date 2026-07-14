use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use koto_compiler::{DiagnosticSeverity, FsLoader, OverlayLoader, SourceSpan};
use koto_lsp::{analyze, budget_inlay, definition_at, hover_at, Analysis};
use serde_json::{json, Value};

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();
    let mut server = Server::default();
    while let Some(message) = read_message(&mut reader)? {
        if server.handle(message, &mut writer)? {
            break;
        }
    }
    Ok(())
}

#[derive(Default)]
struct Server {
    documents: HashMap<PathBuf, String>,
    published: HashSet<String>,
    shutdown: bool,
}

impl Server {
    fn handle(&mut self, message: Value, writer: &mut impl Write) -> io::Result<bool> {
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        match method {
            "initialize" => respond(
                writer,
                id,
                json!({
                    "capabilities": {
                        "positionEncoding": "utf-16",
                        "textDocumentSync": 1,
                        "definitionProvider": true,
                        "hoverProvider": true,
                        "inlayHintProvider": true
                    },
                    "serverInfo": { "name": "koto-lsp", "version": env!("CARGO_PKG_VERSION") }
                }),
            )?,
            "initialized" => {}
            "shutdown" => {
                self.shutdown = true;
                respond(writer, id, Value::Null)?;
            }
            "exit" => return Ok(true),
            "textDocument/didOpen" => {
                if let (Some(uri), Some(text)) = (
                    params.pointer("/textDocument/uri").and_then(Value::as_str),
                    params.pointer("/textDocument/text").and_then(Value::as_str),
                ) {
                    if let Some(path) = file_uri_to_path(uri) {
                        self.documents.insert(path, text.to_string());
                        self.publish_all(writer)?;
                    }
                }
            }
            "textDocument/didChange" => {
                let uri = params.pointer("/textDocument/uri").and_then(Value::as_str);
                let text = params
                    .pointer("/contentChanges")
                    .and_then(Value::as_array)
                    .and_then(|changes| changes.last())
                    .and_then(|change| change.get("text"))
                    .and_then(Value::as_str);
                if let (Some(uri), Some(text)) = (uri, text) {
                    if let Some(path) = file_uri_to_path(uri) {
                        self.documents.insert(path, text.to_string());
                        self.publish_all(writer)?;
                    }
                }
            }
            "textDocument/didClose" => {
                if let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) {
                    if let Some(path) = file_uri_to_path(uri) {
                        self.documents.remove(&path);
                    }
                    notify(
                        writer,
                        "textDocument/publishDiagnostics",
                        json!({ "uri": uri, "diagnostics": [] }),
                    )?;
                    self.published.remove(uri);
                    self.publish_all(writer)?;
                }
            }
            "textDocument/definition" => {
                let result = self.request_context(&params).and_then(
                    |(_path, source, analysis, line, character)| {
                        definition_at(&analysis, &source, line, character).map(|definition| {
                            json!({
                                "uri": path_to_file_uri(Path::new(&definition.span.file)),
                                "range": span_range(&definition.span)
                            })
                        })
                    },
                );
                respond(writer, id, result.unwrap_or(Value::Null))?;
            }
            "textDocument/hover" => {
                let result = self.request_context(&params).and_then(
                    |(_path, source, analysis, line, character)| {
                        hover_at(&analysis, &source, line, character).map(|hover| {
                            json!({
                                "contents": { "kind": "markdown", "value": hover.markdown }
                            })
                        })
                    },
                );
                respond(writer, id, result.unwrap_or(Value::Null))?;
            }
            "textDocument/inlayHint" => {
                let result = self.request_analysis(&params).and_then(|analysis| {
                    budget_inlay(&analysis).map(|inlay| {
                        let warning = if inlay.warning { "⚠ " } else { "" };
                        json!([{
                            "position": { "line": 0, "character": 0 },
                            "label": format!("{warning}slots {}/{}", inlay.used, inlay.capacity),
                            "kind": 1,
                            "paddingRight": true,
                            "tooltip": if inlay.warning {
                                "User-local usage is at least 90% of the compiler slot cap"
                            } else {
                                "Compiler user-local slot usage"
                            }
                        }])
                    })
                });
                respond(writer, id, result.unwrap_or_else(|| json!([])))?;
            }
            _ if id.is_some() => {
                send(
                    writer,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": format!("method not found: {method}") }
                    }),
                )?;
            }
            _ => {}
        }
        Ok(self.shutdown && method == "exit")
    }

    fn request_context(&self, params: &Value) -> Option<(PathBuf, String, Analysis, usize, usize)> {
        let uri = params.pointer("/textDocument/uri")?.as_str()?;
        let path = file_uri_to_path(uri)?;
        let source = self.source(&path)?;
        let analysis = self.analysis_for(&path)?;
        let line = params.pointer("/position/line")?.as_u64()? as usize;
        let character = params.pointer("/position/character")?.as_u64()? as usize;
        Some((path, source, analysis, line, character))
    }

    fn request_analysis(&self, params: &Value) -> Option<Analysis> {
        let uri = params.pointer("/textDocument/uri")?.as_str()?;
        self.analysis_for(&file_uri_to_path(uri)?)
    }

    fn source(&self, path: &Path) -> Option<String> {
        self.documents
            .get(path)
            .cloned()
            .or_else(|| std::fs::read_to_string(path).ok())
    }

    fn root_for(&self, path: &Path) -> PathBuf {
        let start = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        for directory in start.ancestors() {
            let descriptor = directory.join("app.json");
            let Ok(text) = std::fs::read_to_string(&descriptor) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if let Some(source) = value.get("source").and_then(Value::as_str) {
                return directory.join(source);
            }
        }
        path.to_path_buf()
    }

    fn analysis_for(&self, path: &Path) -> Option<Analysis> {
        let root = self.root_for(path);
        let source = self.source(&root)?;
        let mut resolver = OverlayLoader::new(FsLoader);
        for (path, text) in &self.documents {
            resolver.insert(path, text.clone());
        }
        Some(analyze(&display_path(&root), &source, &mut resolver))
    }

    fn publish_all(&mut self, writer: &mut impl Write) -> io::Result<()> {
        let roots: HashSet<PathBuf> = self
            .documents
            .keys()
            .map(|path| self.root_for(path))
            .collect();
        let mut by_uri: HashMap<String, Vec<Value>> = HashMap::new();
        for root in roots {
            let Some(source) = self.source(&root) else {
                continue;
            };
            let mut resolver = OverlayLoader::new(FsLoader);
            for (path, text) in &self.documents {
                resolver.insert(path, text.clone());
            }
            for diagnostic in analyze(&display_path(&root), &source, &mut resolver).diagnostics {
                let Some(span) = diagnostic.span else {
                    continue;
                };
                let uri = path_to_file_uri(Path::new(&span.file));
                by_uri.entry(uri).or_default().push(json!({
                    "range": span_range(&span),
                    "severity": match diagnostic.severity {
                        DiagnosticSeverity::Error => 1,
                        DiagnosticSeverity::Warning => 2,
                        DiagnosticSeverity::Information => 3,
                    },
                    "source": "koto-compiler",
                    "message": diagnostic.message
                }));
            }
        }

        let current: HashSet<String> = by_uri.keys().cloned().collect();
        let targets: HashSet<String> = self
            .published
            .union(&current)
            .cloned()
            .chain(self.documents.keys().map(|path| path_to_file_uri(path)))
            .collect();
        for uri in targets {
            notify(
                writer,
                "textDocument/publishDiagnostics",
                json!({ "uri": uri, "diagnostics": by_uri.remove(&uri).unwrap_or_default() }),
            )?;
        }
        self.published = current;
        Ok(())
    }
}

fn span_range(span: &SourceSpan) -> Value {
    json!({
        "start": {
            "line": span.start.line.saturating_sub(1),
            "character": span.start.column.saturating_sub(1)
        },
        "end": {
            "line": span.end.line.saturating_sub(1),
            "character": span.end.column.saturating_sub(1)
        }
    })
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?;
    let decoded = percent_decode(encoded)?;
    #[cfg(windows)]
    let decoded = decoded
        .strip_prefix('/')
        .filter(|rest| rest.as_bytes().get(1) == Some(&b':'))
        .unwrap_or(&decoded)
        .to_string();
    Some(PathBuf::from(decoded))
}

fn path_to_file_uri(path: &Path) -> String {
    let display = display_path(path);
    let prefix = if display.starts_with('/') {
        "file://"
    } else {
        "file:///"
    };
    format!("{prefix}{}", percent_encode(&display))
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/' | b':') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut content_length = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            return Ok(None);
        }
        let header = header.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            content_length = value.trim().parse::<usize>().ok();
        }
    }
    let length = content_length
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn respond(writer: &mut impl Write, id: Option<Value>, result: Value) -> io::Result<()> {
    send(
        writer,
        &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    )
}

fn notify(writer: &mut impl Write, method: &str, params: Value) -> io::Result<()> {
    send(
        writer,
        &json!({ "jsonrpc": "2.0", "method": method, "params": params }),
    )
}

fn send(writer: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(bytes: &[u8]) -> Vec<Value> {
        let mut reader = BufReader::new(bytes);
        let mut values = Vec::new();
        while let Some(value) = read_message(&mut reader).unwrap() {
            values.push(value);
        }
        values
    }

    #[test]
    fn message_framing_round_trips() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"shutdown"}"#;
        let framed = format!(
            "Content-Length: {}\r\n\r\n{}",
            body.len(),
            String::from_utf8_lossy(body)
        );
        let message = read_message(&mut BufReader::new(framed.as_bytes()))
            .unwrap()
            .unwrap();
        assert_eq!(message["method"], "shutdown");
    }

    #[test]
    fn file_uri_round_trips_spaces_and_unicode() {
        let path = PathBuf::from(if cfg!(windows) {
            r"C:\work space\日本\main.koto"
        } else {
            "/work space/日本/main.koto"
        });
        assert_eq!(file_uri_to_path(&path_to_file_uri(&path)), Some(path));
    }

    #[test]
    fn server_publishes_and_clears_unsaved_diagnostics() {
        let path = std::env::temp_dir().join("koto_lsp_unsaved_test.koto");
        let uri = path_to_file_uri(&path);
        let mut server = Server::default();
        let mut output = Vec::new();
        server
            .handle(
                json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/didOpen",
                    "params": { "textDocument": {
                        "uri": uri,
                        "languageId": "koto",
                        "version": 1,
                        "text": "fn main( { }\n"
                    }}
                }),
                &mut output,
            )
            .unwrap();
        let published = messages(&output);
        assert!(published.iter().any(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["diagnostics"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
        }));

        output.clear();
        server
            .handle(
                json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/didChange",
                    "params": {
                        "textDocument": { "uri": uri, "version": 2 },
                        "contentChanges": [{ "text": "fn main() { exit(0); }\n" }]
                    }
                }),
                &mut output,
            )
            .unwrap();
        let cleared = messages(&output);
        assert!(cleared.iter().any(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["diagnostics"] == json!([])
        }));
    }
}
