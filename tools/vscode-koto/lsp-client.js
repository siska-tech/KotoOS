// Minimal dependency-free LSP client for koto-lsp (KOTO-0194).

"use strict";

const vscode = require("vscode");
const { spawn } = require("child_process");

function startKotoLanguageClient(context) {
  const config = vscode.workspace.getConfiguration("koto.languageServer");
  if (!config.get("enabled", true)) return { dispose() {} };
  const root = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  if (!root) return { dispose() {} };

  const configuredPath = config.get("path", "").trim();
  const command = configuredPath || "cargo";
  const args = configuredPath ? [] : ["run", "-q", "-p", "koto-lsp", "--"];
  const output = vscode.window.createOutputChannel("Koto Language Server");
  const diagnostics = vscode.languages.createDiagnosticCollection("koto");
  const process = spawn(command, args, { cwd: root, shell: false, windowsHide: true });
  const rpc = new RpcConnection(process, output);
  const selector = { language: "koto", scheme: "file" };
  const debounceMs = Math.max(0, config.get("debounceMs", 150));
  const timers = new Map();

  rpc.onNotification = (method, params) => {
    if (method !== "textDocument/publishDiagnostics") return;
    const uri = vscode.Uri.parse(params.uri);
    diagnostics.set(uri, (params.diagnostics || []).map(toDiagnostic));
  };

  const ready = rpc.request("initialize", {
    processId: process.pid,
    rootUri: vscode.Uri.file(root).toString(),
    capabilities: {
      general: { positionEncodings: ["utf-16"] },
      textDocument: { inlayHint: {} },
    },
    clientInfo: { name: "vscode-koto", version: "0.3.0" },
  }).then(() => {
    rpc.notify("initialized", {});
    for (const document of vscode.workspace.textDocuments) open(document);
  }).catch((error) => output.appendLine(`initialize failed: ${error.message}`));

  function open(document) {
    if (document.languageId !== "koto" || document.uri.scheme !== "file") return;
    rpc.notify("textDocument/didOpen", {
      textDocument: {
        uri: document.uri.toString(),
        languageId: "koto",
        version: document.version,
        text: document.getText(),
      },
    });
  }

  function change(document) {
    if (document.languageId !== "koto" || document.uri.scheme !== "file") return;
    const key = document.uri.toString();
    clearTimeout(timers.get(key));
    timers.set(key, setTimeout(() => {
      timers.delete(key);
      rpc.notify("textDocument/didChange", {
        textDocument: { uri: key, version: document.version },
        contentChanges: [{ text: document.getText() }],
      });
    }, debounceMs));
  }

  const subscriptions = [
    output,
    diagnostics,
    vscode.workspace.onDidOpenTextDocument((document) => ready.then(() => open(document))),
    vscode.workspace.onDidChangeTextDocument((event) => ready.then(() => change(event.document))),
    vscode.workspace.onDidCloseTextDocument((document) => {
      if (document.languageId !== "koto" || document.uri.scheme !== "file") return;
      const key = document.uri.toString();
      clearTimeout(timers.get(key));
      timers.delete(key);
      diagnostics.delete(document.uri);
      ready.then(() => rpc.notify("textDocument/didClose", { textDocument: { uri: key } }));
    }),
    vscode.languages.registerDefinitionProvider(selector, {
      async provideDefinition(document, position) {
        await ready;
        const result = await rpc.request("textDocument/definition", textPosition(document, position));
        return result ? new vscode.Location(vscode.Uri.parse(result.uri), toRange(result.range)) : null;
      },
    }),
    vscode.languages.registerHoverProvider(selector, {
      async provideHover(document, position) {
        await ready;
        const result = await rpc.request("textDocument/hover", textPosition(document, position));
        if (!result) return null;
        return new vscode.Hover(new vscode.MarkdownString(result.contents.value));
      },
    }),
    vscode.languages.registerInlayHintsProvider(selector, {
      async provideInlayHints(document, range) {
        await ready;
        const results = await rpc.request("textDocument/inlayHint", {
          textDocument: { uri: document.uri.toString() },
          range: fromRange(range),
        });
        return (results || []).map((item) => {
          const hint = new vscode.InlayHint(toPosition(item.position), item.label, item.kind);
          hint.paddingLeft = Boolean(item.paddingLeft);
          hint.paddingRight = Boolean(item.paddingRight);
          hint.tooltip = item.tooltip;
          return hint;
        });
      },
    }),
  ];

  return {
    dispose() {
      for (const timer of timers.values()) clearTimeout(timer);
      for (const subscription of subscriptions) subscription.dispose();
      ready.catch(() => {}).then(() => {
        rpc.request("shutdown", null)
          .catch(() => {})
          .finally(() => {
            rpc.notify("exit", null);
            rpc.dispose();
          });
      });
    },
  };
}

class RpcConnection {
  constructor(process, output) {
    this.process = process;
    this.output = output;
    this.nextId = 1;
    this.pending = new Map();
    this.buffer = Buffer.alloc(0);
    this.onNotification = () => {};
    process.stdout.on("data", (chunk) => this.accept(chunk));
    process.stderr.on("data", (chunk) => output.append(chunk.toString()));
    process.on("error", (error) => output.appendLine(`server error: ${error.message}`));
    process.on("exit", (code) => {
      const error = new Error(`koto-lsp exited with code ${code}`);
      for (const pending of this.pending.values()) pending.reject(error);
      this.pending.clear();
    });
  }

  request(method, params) {
    const id = this.nextId++;
    this.send({ jsonrpc: "2.0", id, method, params });
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
  }

  notify(method, params) {
    this.send({ jsonrpc: "2.0", method, params });
  }

  send(message) {
    if (this.process.stdin.destroyed) return;
    const body = Buffer.from(JSON.stringify(message));
    this.process.stdin.write(`Content-Length: ${body.length}\r\n\r\n`);
    this.process.stdin.write(body);
  }

  accept(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    while (true) {
      const headerEnd = this.buffer.indexOf("\r\n\r\n");
      if (headerEnd < 0) return;
      const header = this.buffer.subarray(0, headerEnd).toString();
      const match = /(?:^|\r\n)Content-Length:\s*(\d+)/i.exec(header);
      if (!match) {
        this.output.appendLine("invalid LSP response: missing Content-Length");
        this.buffer = Buffer.alloc(0);
        return;
      }
      const length = Number(match[1]);
      const bodyStart = headerEnd + 4;
      if (this.buffer.length < bodyStart + length) return;
      const body = this.buffer.subarray(bodyStart, bodyStart + length);
      this.buffer = this.buffer.subarray(bodyStart + length);
      try { this.dispatch(JSON.parse(body.toString())); }
      catch (error) { this.output.appendLine(`invalid LSP JSON: ${error.message}`); }
    }
  }

  dispatch(message) {
    if (message.id !== undefined && (message.result !== undefined || message.error)) {
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.error) pending.reject(new Error(message.error.message));
      else pending.resolve(message.result);
    } else if (message.method) {
      this.onNotification(message.method, message.params);
    }
  }

  dispose() {
    this.process.stdin.end();
    if (!this.process.killed) this.process.kill();
  }
}

function textPosition(document, position) {
  return {
    textDocument: { uri: document.uri.toString() },
    position: fromPosition(position),
  };
}

function toDiagnostic(item) {
  const severity = {
    1: vscode.DiagnosticSeverity.Error,
    2: vscode.DiagnosticSeverity.Warning,
    3: vscode.DiagnosticSeverity.Information,
  }[item.severity] ?? vscode.DiagnosticSeverity.Error;
  const diagnostic = new vscode.Diagnostic(toRange(item.range), item.message, severity);
  diagnostic.source = item.source;
  return diagnostic;
}

function fromPosition(position) { return { line: position.line, character: position.character }; }
function toPosition(position) { return new vscode.Position(position.line, position.character); }
function fromRange(range) { return { start: fromPosition(range.start), end: fromPosition(range.end) }; }
function toRange(range) { return new vscode.Range(toPosition(range.start), toPosition(range.end)); }

module.exports = { startKotoLanguageClient, RpcConnection };
