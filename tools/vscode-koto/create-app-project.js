// VS Code new Koto app project wizard (KOTO-0197).

"use strict";

const vscode = require("vscode");
const { spawn } = require("child_process");
const {
  validateAppId,
  defaultAppDirectory,
  validateAppDirectory,
  scaffoldArgs,
} = require("./project-model.js");

async function createAppProject() {
  const root = vscode.workspace.workspaceFolders?.[0]?.uri;
  if (!root) {
    vscode.window.showErrorMessage("Koto: open the KotoOS workspace first");
    return;
  }

  const appId = await vscode.window.showInputBox({
    title: "Create Koto App — 1/3",
    prompt: "Application ID (reverse DNS)",
    placeHolder: "dev.koto.apps.todo-list",
    validateInput: validateAppId,
    ignoreFocusOut: true,
  });
  if (appId === undefined) return;

  const name = await vscode.window.showInputBox({
    title: "Create Koto App — 2/3",
    prompt: "Display name shown in KotoShell",
    placeHolder: "Todo List",
    validateInput: (value) => value.trim() ? null : "Enter a display name",
    ignoreFocusOut: true,
  });
  if (name === undefined) return;

  const appDirectory = await vscode.window.showInputBox({
    title: "Create Koto App — 3/3",
    prompt: "Project directory relative to the workspace",
    value: defaultAppDirectory(appId),
    validateInput: validateAppDirectory,
    ignoreFocusOut: true,
  });
  if (appDirectory === undefined) return;

  const confirmation = await vscode.window.showQuickPick(
    [
      {
        label: "$(new-folder) Create project",
        description: name.trim(),
        detail: `${appId}  →  ${appDirectory}`,
        create: true,
      },
      { label: "Cancel", create: false },
    ],
    { title: "Confirm new Koto app", ignoreFocusOut: true }
  );
  if (!confirmation?.create) return;

  const output = vscode.window.createOutputChannel("Koto App Scaffold");
  output.clear();
  output.show(true);
  try {
    await vscode.window.withProgress(
      {
        location: vscode.ProgressLocation.Notification,
        title: `Creating ${name.trim()}`,
        cancellable: true,
      },
      (_progress, token) => runScaffold(
        root.fsPath,
        appId,
        name.trim(),
        appDirectory,
        output,
        token
      )
    );
  } catch (error) {
    if (error.cancelled) {
      vscode.window.showInformationMessage("Koto: project creation cancelled");
    } else {
      vscode.window.showErrorMessage(`Koto: could not create project — ${error.message}`);
    }
    return;
  }

  const source = vscode.Uri.joinPath(root, ...appDirectory.split("/"), "src", "main.koto");
  const document = await vscode.workspace.openTextDocument(source);
  await vscode.window.showTextDocument(document);
  const action = await vscode.window.showInformationMessage(
    `Koto: created ${name.trim()} in ${appDirectory}`,
    "Open app.json"
  );
  if (action === "Open app.json") {
    const descriptor = vscode.Uri.joinPath(root, ...appDirectory.split("/"), "app.json");
    await vscode.window.showTextDocument(await vscode.workspace.openTextDocument(descriptor));
  }
}

function runScaffold(root, appId, name, appDirectory, output, token) {
  const args = scaffoldArgs(root, appId, name, appDirectory);
  output.appendLine(`> cargo ${args.join(" ")}`);
  return new Promise((resolve, reject) => {
    const child = spawn("cargo", args, { cwd: root, shell: false, windowsHide: true });
    let stderr = "";
    child.stdout.on("data", (data) => output.append(data.toString()));
    child.stderr.on("data", (data) => {
      const text = data.toString();
      stderr += text;
      output.append(text);
    });
    child.on("error", reject);
    child.on("close", (code) => {
      disposable.dispose();
      if (token.isCancellationRequested) {
        reject(Object.assign(new Error("cancelled"), { cancelled: true }));
      } else if (code === 0) {
        resolve();
      } else {
        reject(new Error(stderr.trim().split(/\r?\n/).pop() || `scaffold exited ${code}`));
      }
    });
    const disposable = token.onCancellationRequested(() => child.kill());
  });
}

module.exports = { createAppProject, runScaffold };
