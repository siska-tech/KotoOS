// KotoOS Development extension host (KOTO-0192 / KOTO-0196): the `.kspr` sprite
// and `.kicon` launcher-icon custom editors, and `.kmml` play/stop commands.
// Plain JavaScript on purpose — the extension stays installable from source
// with no build step (KOTO-0190).
//
// Each editor's webview owns its document model (media/*-model.js) and posts
// whole replacement text; this host only bridges to the TextDocument (so undo /
// redo / save are VS Code's) and spawns the Rust CLIs. No format logic here.

"use strict";

const vscode = require("vscode");
const path = require("path");
const { spawn } = require("child_process");
const {
  DEFAULT_PALETTE,
  updatePalette,
  addResource,
} = require("./app-json-edits.js");
const { startKotoLanguageClient } = require("./lsp-client.js");
const { createAppProject } = require("./create-app-project.js");
const { resolveMapConfig } = require("./tilemap-config.js");

/** The running `koto-mml play` child, if any. */
let playProc = null;
let output = null;

function activate(context) {
  output = vscode.window.createOutputChannel("Koto MML");
  context.subscriptions.push(
    startKotoLanguageClient(context),
    vscode.window.registerCustomEditorProvider(
      "koto.ksprEditor",
      new KsprEditorProvider(context),
      { webviewOptions: { retainContextWhenHidden: true } }
    ),
    vscode.window.registerCustomEditorProvider(
      "koto.kiconEditor",
      new KiconEditorProvider(context),
      { webviewOptions: { retainContextWhenHidden: true } }
    ),
    vscode.window.registerCustomEditorProvider(
      "koto.tilemapEditor",
      new TilemapEditorProvider(context),
      { webviewOptions: { retainContextWhenHidden: true } }
    ),
    vscode.commands.registerCommand("koto.kspr.openEditor", (uri) => {
      const target = uri || vscode.window.activeTextEditor?.document.uri;
      if (target) {
        vscode.commands.executeCommand("vscode.openWith", target, "koto.ksprEditor");
      }
    }),
    vscode.commands.registerCommand("koto.icon.openEditor", (uri) => {
      const target = uri || vscode.window.activeTextEditor?.document.uri;
      if (target) {
        vscode.commands.executeCommand("vscode.openWith", target, "koto.kiconEditor");
      }
    }),
    vscode.commands.registerCommand("koto.tilemap.openEditor", (uri) => {
      const target = uri || vscode.window.activeTextEditor?.document.uri;
      if (target) {
        vscode.commands.executeCommand("vscode.openWith", target, "koto.tilemapEditor");
      }
    }),
    vscode.commands.registerCommand("koto.app.openIcon", openAppIcon),
    vscode.commands.registerCommand("koto.app.addResource", addAppResource),
    vscode.commands.registerCommand("koto.app.createProject", createAppProject),
    vscode.commands.registerCommand("koto.kmml.play", (uri) => playKmml(uri, [])),
    vscode.commands.registerCommand("koto.kmml.playOptions", playKmmlWithOptions),
    vscode.commands.registerCommand("koto.kmml.stop", stopKmml)
  );
}

function activeDescriptor(uri) {
  const descriptor = uri || vscode.window.activeTextEditor?.document.uri;
  if (!descriptor || path.basename(descriptor.fsPath) !== "app.json") {
    vscode.window.showWarningMessage("Koto: open an app.json descriptor first");
    return null;
  }
  return descriptor;
}

async function descriptorDocument(uri) {
  return vscode.workspace.textDocuments.find(
    (document) => document.uri.toString() === uri.toString()
  ) || vscode.workspace.openTextDocument(uri);
}

async function replaceDocument(document, text) {
  const edit = new vscode.WorkspaceEdit();
  edit.replace(
    document.uri,
    new vscode.Range(0, 0, document.lineCount, 0),
    text
  );
  return vscode.workspace.applyEdit(edit);
}

/** Open the icon referenced by an `app.json` descriptor in the icon editor. */
async function openAppIcon(uri) {
  const descriptor = activeDescriptor(uri);
  if (!descriptor) return;
  let iconRel = "icon.kicon";
  try {
    const bytes = await vscode.workspace.fs.readFile(descriptor);
    iconRel = JSON.parse(Buffer.from(bytes).toString("utf8")).icon || iconRel;
  } catch {
    // Fall back to the conventional icon.kicon next to the descriptor.
  }
  const iconUri = vscode.Uri.joinPath(descriptor, "..", iconRel);
  vscode.commands.executeCommand("vscode.openWith", iconUri, "koto.kiconEditor");
}

/** Pick an app-local source and register it under images/audio in app.json. */
async function addAppResource(uri) {
  const descriptor = activeDescriptor(uri);
  if (!descriptor) return;
  const appDir = vscode.Uri.joinPath(descriptor, "..");
  const picked = await vscode.window.showOpenDialog({
    title: "Koto: Select an app resource",
    defaultUri: appDir,
    canSelectFiles: true,
    canSelectFolders: false,
    canSelectMany: false,
    filters: {
      "Koto resources": ["kspr", "kmml", "kacl"],
      "All files": ["*"],
    },
  });
  if (!picked?.length) return;

  const selected = picked[0];
  const relative = path.relative(appDir.fsPath, selected.fsPath);
  if (!relative || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
    vscode.window.showErrorMessage("Koto: select a file inside this app directory");
    return;
  }
  const source = relative.split(path.sep).join("/");
  const extension = path.extname(source).toLowerCase();
  let kind;
  let suggested;
  if (extension === ".kspr") {
    kind = "images";
    suggested = `sprites/${path.basename(source, extension)}.kim`;
  } else if (extension === ".kmml" || extension === ".kacl") {
    kind = "audio";
    suggested = `audio/${path.basename(source)}`;
  } else {
    vscode.window.showErrorMessage(
      "Koto: supported resources are .kspr, .kmml, and .kacl"
    );
    return;
  }

  const outputPath = await vscode.window.showInputBox({
    title: `Package output for ${source}`,
    value: suggested,
    prompt: `This will add an entry to app.json → ${kind}`,
    validateInput: (value) => {
      const normalized = value.replace(/\\/g, "/");
      if (!normalized || normalized.startsWith("/") || normalized.split("/").includes("..")) {
        return "Use a package-local path without '..'";
      }
      return null;
    },
  });
  if (outputPath === undefined) return;

  try {
    const document = await descriptorDocument(descriptor);
    const next = addResource(document.getText(), kind, {
      source,
      output: outputPath.replace(/\\/g, "/"),
    });
    if (await replaceDocument(document, next)) {
      vscode.window.showInformationMessage(`Koto: registered ${source} in ${kind}`);
    }
  } catch (error) {
    vscode.window.showErrorMessage(`Koto: ${error.message}`);
  }
}

/** Read the app's `shell_icon` palette from the descriptor beside an icon. */
async function resolvePalette(iconUri) {
  const descriptor = vscode.Uri.joinPath(iconUri, "..", "app.json");
  try {
    const document = await descriptorDocument(descriptor);
    return { ...DEFAULT_PALETTE, ...(JSON.parse(document.getText()).shell_icon || {}) };
  } catch {
    return { ...DEFAULT_PALETTE };
  }
}

function deactivate() {
  stopKmml();
}

// --- .kmml audition (KOTO-0188 CLI) --------------------------------------

function kmmlTarget(uri) {
  const target = uri || vscode.window.activeTextEditor?.document.uri;
  if (!target || !target.fsPath.endsWith(".kmml")) {
    vscode.window.showWarningMessage("Koto MML: open a .kmml file first");
    return null;
  }
  return target;
}

function workspaceRoot() {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

async function playKmmlWithOptions(uri) {
  const mute = await vscode.window.showInputBox({
    title: "Mute tracks (0-based indices, comma separated; empty = none)",
    placeHolder: "e.g. 1,2",
  });
  if (mute === undefined) return;
  const loop = await vscode.window.showQuickPick(
    [{ label: "play once" }, { label: "loop (10 s)" }],
    { title: "Playback" }
  );
  if (!loop) return;
  const args = [];
  for (const index of mute.split(",").map((s) => s.trim()).filter(Boolean)) {
    args.push("--mute", index);
  }
  if (loop.label.startsWith("loop")) args.push("--loop");
  playKmml(uri, args);
}

function playKmml(uri, extraArgs) {
  const target = kmmlTarget(uri);
  const root = workspaceRoot();
  if (!target || !root) return;
  stopKmml();
  const file = path.relative(root, target.fsPath);
  const args = [
    "run", "-q", "-p", "koto-mml", "--features", "play", "--",
    "play", file, ...extraArgs,
  ];
  output.clear();
  output.show(true);
  output.appendLine(`> cargo ${args.join(" ")}`);
  playProc = spawn("cargo", args, { cwd: root, shell: false });
  playProc.stdout.on("data", (data) => output.append(data.toString()));
  playProc.stderr.on("data", (data) => output.append(data.toString()));
  playProc.on("close", (code) => {
    output.appendLine(code === 0 ? "(done)" : `(exit ${code})`);
    playProc = null;
  });
}

function stopKmml() {
  if (!playProc) return;
  // `cargo run` wraps the real player, so kill the whole tree on Windows.
  if (process.platform === "win32") {
    spawn("taskkill", ["/PID", String(playProc.pid), "/T", "/F"], { shell: false });
  } else {
    playProc.kill();
  }
  output?.appendLine("(stopped)");
  playProc = null;
}

// --- .kspr sprite custom editor -------------------------------------------

class KsprEditorProvider {
  constructor(context) {
    this.context = context;
  }

  resolveCustomTextEditor(document, webviewPanel) {
    const webview = webviewPanel.webview;
    webview.options = {
      enableScripts: true,
      localResourceRoots: [
        vscode.Uri.joinPath(this.context.extensionUri, "media"),
      ],
    };
    webview.html = this.html(webview);

    const post = () => {
      webview.postMessage({ type: "document", text: document.getText() });
    };
    // Guard: our own applyEdit triggers onDidChangeTextDocument; the webview
    // already shows that state, so skip the echo to keep painting smooth.
    let expected = null;
    const changeSub = vscode.workspace.onDidChangeTextDocument((event) => {
      if (event.document.uri.toString() !== document.uri.toString()) return;
      if (document.getText() === expected) return;
      post();
    });
    webviewPanel.onDidDispose(() => changeSub.dispose());

    webview.onDidReceiveMessage((message) => {
      if (message.type === "ready") {
        post();
      } else if (message.type === "replace") {
        expected = message.text;
        const edit = new vscode.WorkspaceEdit();
        edit.replace(
          document.uri,
          new vscode.Range(0, 0, document.lineCount, 0),
          message.text
        );
        vscode.workspace.applyEdit(edit);
      }
    });
  }

  html(webview) {
    const media = (file) =>
      webview.asWebviewUri(
        vscode.Uri.joinPath(this.context.extensionUri, "media", file)
      );
    return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy"
      content="default-src 'none'; style-src ${webview.cspSource}; script-src ${webview.cspSource};">
<link rel="stylesheet" href="${media("kspr-editor.css")}">
</head>
<body>
<div id="toolbar">
  <span id="palette"></span>
  <button id="add-color">+ color</button>
  <input id="hex-input" maxlength="6" placeholder="RRGGBB" size="7" hidden>
  <button id="add-tile">+ tile</button>
  <span id="status"></span>
</div>
<div id="main">
  <div id="tiles"></div>
  <canvas id="grid" width="512" height="512"></canvas>
</div>
<script src="${media("kspr-model.js")}"></script>
<script src="${media("kspr-editor.js")}"></script>
</body>
</html>`;
  }
}

class KiconEditorProvider {
  constructor(context) {
    this.context = context;
  }

  resolveCustomTextEditor(document, webviewPanel) {
    const webview = webviewPanel.webview;
    webview.options = {
      enableScripts: true,
      localResourceRoots: [vscode.Uri.joinPath(this.context.extensionUri, "media")],
    };
    webview.html = this.html(webview);

    const post = () => webview.postMessage({ type: "document", text: document.getText() });
    let expected = null;
    const descriptor = vscode.Uri.joinPath(document.uri, "..", "app.json");
    const changeSub = vscode.workspace.onDidChangeTextDocument(async (event) => {
      if (event.document.uri.toString() === descriptor.toString()) {
        const palette = await resolvePalette(document.uri);
        webview.postMessage({ type: "palette", palette });
        return;
      }
      if (event.document.uri.toString() === document.uri.toString()) {
        if (document.getText() === expected) return;
        post();
      }
    });
    webviewPanel.onDidDispose(() => changeSub.dispose());

    webview.onDidReceiveMessage(async (message) => {
      if (message.type === "ready") {
        post();
        const palette = await resolvePalette(document.uri);
        webview.postMessage({ type: "palette", palette });
      } else if (message.type === "replace") {
        expected = message.text;
        const edit = new vscode.WorkspaceEdit();
        edit.replace(
          document.uri,
          new vscode.Range(0, 0, document.lineCount, 0),
          message.text
        );
        vscode.workspace.applyEdit(edit);
      } else if (message.type === "applyPalette") {
        try {
          const descriptorDoc = await descriptorDocument(descriptor);
          const next = updatePalette(descriptorDoc.getText(), message.palette);
          const applied = await replaceDocument(descriptorDoc, next);
          webview.postMessage({
            type: "paletteResult",
            ok: applied,
            message: applied ? "Palette applied to app.json" : "Could not edit app.json",
          });
        } catch (error) {
          webview.postMessage({ type: "paletteResult", ok: false, message: error.message });
        }
      }
    });
  }

  html(webview) {
    const media = (file) =>
      webview.asWebviewUri(vscode.Uri.joinPath(this.context.extensionUri, "media", file));
    return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy"
      content="default-src 'none'; style-src ${webview.cspSource}; script-src ${webview.cspSource};">
<link rel="stylesheet" href="${media("kicon-editor.css")}">
</head>
<body>
<div id="toolbar">
  <span id="hint">left-click / drag: set · right-click: clear</span>
  <span id="status"></span>
</div>
<div id="editor-layout">
  <canvas id="grid" width="480" height="480"></canvas>
  <aside id="palette-panel">
    <h2>App icon palette</h2>
    <div id="palette-controls"></div>
    <button id="apply-palette">Apply to app.json</button>
    <p id="palette-status" role="status"></p>
  </aside>
</div>
<script src="${media("kicon-model.js")}"></script>
<script src="${media("kicon-editor.js")}"></script>
</body>
</html>`;
  }
}

// --- app.json-backed text tilemap custom editor (KOTO-0198) ---------------

function mapWebviewConfig(config) {
  return {
    width: config.width,
    height: config.height,
    glyphs: config.glyphs,
    tiles: config.tiles || null,
    tilesetText: config.tilesetText || null,
    previewError: config.previewError || null,
  };
}

class TilemapEditorProvider {
  constructor(context) {
    this.context = context;
  }

  resolveCustomTextEditor(document, webviewPanel) {
    const webview = webviewPanel.webview;
    webview.options = {
      enableScripts: true,
      localResourceRoots: [vscode.Uri.joinPath(this.context.extensionUri, "media")],
    };
    webview.html = this.html(webview);

    let config = null;
    let descriptorPath = null;
    let tilesetPath = null;
    let expected = null;
    let disposed = false;
    const readText = async (filename) => {
      const uri = vscode.Uri.file(filename);
      const open = vscode.workspace.textDocuments.find(
        (candidate) => candidate.uri.toString() === uri.toString()
      );
      if (open) return open.getText();
      try {
        return Buffer.from(await vscode.workspace.fs.readFile(uri)).toString("utf8");
      } catch {
        return null;
      }
    };
    const refreshConfig = async () => {
      const resolved = await resolveMapConfig(document.uri.fsPath, readText);
      if (disposed) return;
      descriptorPath = resolved.descriptorPath || null;
      tilesetPath = resolved.tilesetPath || null;
      if (!resolved.ok) {
        config = null;
        webview.postMessage({ type: "unavailable", message: resolved.error });
        return;
      }
      config = resolved;
      webview.postMessage({
        type: "document",
        text: document.getText(),
        config: mapWebviewConfig(config),
      });
    };

    const changeSub = vscode.workspace.onDidChangeTextDocument((event) => {
      const changed = event.document.uri.fsPath;
      if ((descriptorPath && path.resolve(changed) === path.resolve(descriptorPath)) ||
          (tilesetPath && path.resolve(changed) === path.resolve(tilesetPath))) {
        refreshConfig();
        return;
      }
      if (event.document.uri.toString() !== document.uri.toString()) return;
      if (document.getText() === expected) {
        expected = null;
        return;
      }
      if (config) {
        webview.postMessage({
          type: "document",
          text: document.getText(),
          config: mapWebviewConfig(config),
        });
      }
    });
    const saveSub = vscode.workspace.onDidSaveTextDocument((saved) => {
      if (tilesetPath && path.resolve(saved.uri.fsPath) === path.resolve(tilesetPath)) {
        refreshConfig();
      }
    });
    webviewPanel.onDidDispose(() => {
      disposed = true;
      changeSub.dispose();
      saveSub.dispose();
    });

    webview.onDidReceiveMessage((message) => {
      if (message.type === "ready") {
        refreshConfig();
      } else if (message.type === "replace" && config && typeof message.text === "string") {
        expected = message.text;
        const edit = new vscode.WorkspaceEdit();
        edit.replace(
          document.uri,
          new vscode.Range(0, 0, document.lineCount, 0),
          message.text
        );
        vscode.workspace.applyEdit(edit);
      }
    });
  }

  html(webview) {
    const media = (file) =>
      webview.asWebviewUri(vscode.Uri.joinPath(this.context.extensionUri, "media", file));
    return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy"
      content="default-src 'none'; style-src ${webview.cspSource}; script-src ${webview.cspSource};">
<link rel="stylesheet" href="${media("tilemap-editor.css")}">
</head>
<body>
<p id="unavailable" role="alert" hidden></p>
<main id="editor" hidden>
  <div id="toolbar">
    <div id="palette" aria-label="Map glyph palette"></div>
    <div id="status" role="status"></div>
  </div>
  <div id="grid-wrap"><div id="grid" aria-label="Tilemap grid"></div></div>
  <p id="hint">left-click / drag: paint · right-click: eyedropper · ␠: space</p>
</main>
<script src="${media("tilemap-model.js")}"></script>
<script src="${media("kspr-model.js")}"></script>
<script src="${media("tilemap-editor.js")}"></script>
</body>
</html>`;
  }
}

module.exports = { activate, deactivate };
