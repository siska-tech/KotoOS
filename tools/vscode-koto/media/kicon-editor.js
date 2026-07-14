// `.kicon` launcher-icon editor webview (KOTO-0196): a 40x40 1-bit mask.
// Pixels and buttons only; all document knowledge is in KiconModel. Every edit
// serializes the whole text back to the host, which applies it to the
// TextDocument (undo/redo/save are ordinary text edits).

"use strict";

/* global acquireVsCodeApi, KiconModel */

const vscode = acquireVsCodeApi();

const CELL = 12; // 40 * 12 = 480px grid
const state = {
  model: null,
  painting: null, // the value being painted during a drag
  // Recolor the mask with the app's shell_icon when the host provides it.
  colors: { on: "#e8d27a", off: "#1a1424", grid: "rgba(128,128,128,0.25)" },
};

const PALETTE_KEYS = [
  "background", "primary", "secondary", "accent", "highlight", "shadow",
];

const grid = document.getElementById("grid");
const ctx = grid.getContext("2d");
const statusEl = document.getElementById("status");
const paletteControls = document.getElementById("palette-controls");
const paletteStatus = document.getElementById("palette-status");

for (const key of PALETTE_KEYS) {
  const label = document.createElement("label");
  const name = document.createElement("span");
  const color = document.createElement("input");
  const hex = document.createElement("input");
  name.textContent = key;
  color.type = "color";
  color.dataset.key = key;
  color.setAttribute("aria-label", `${key} color`);
  hex.type = "text";
  hex.maxLength = 7;
  hex.dataset.key = key;
  hex.setAttribute("aria-label", `${key} hex value`);
  color.addEventListener("input", () => {
    hex.value = color.value.toUpperCase();
    readPaletteControls(true);
  });
  hex.addEventListener("input", () => {
    if (/^#[0-9A-Fa-f]{6}$/.test(hex.value)) {
      color.value = hex.value;
      readPaletteControls(true);
    }
  });
  label.append(name, color, hex);
  paletteControls.appendChild(label);
}

window.addEventListener("message", (event) => {
  const message = event.data;
  if (message.type === "document") {
    state.model = new KiconModel(message.text);
    render();
  } else if (message.type === "palette" && message.palette) {
    setPaletteControls(message.palette);
    paletteStatus.textContent = "";
    render();
  } else if (message.type === "paletteResult") {
    paletteStatus.textContent = message.message;
    paletteStatus.className = message.ok ? "success" : "error";
  }
});

function setPaletteControls(palette) {
  for (const key of PALETTE_KEYS) {
    const value = palette[key];
    if (!/^#[0-9A-Fa-f]{6}$/.test(value || "")) continue;
    paletteControls.querySelector(`input[type=color][data-key=${key}]`).value = value;
    paletteControls.querySelector(`input[type=text][data-key=${key}]`).value = value.toUpperCase();
  }
  readPaletteControls(true);
}

function readPaletteControls(updatePreview) {
  const palette = {};
  let valid = true;
  for (const key of PALETTE_KEYS) {
    const input = paletteControls.querySelector(`input[type=text][data-key=${key}]`);
    palette[key] = input.value;
    input.classList.toggle("invalid", !/^#[0-9A-Fa-f]{6}$/.test(input.value));
    valid = valid && /^#[0-9A-Fa-f]{6}$/.test(input.value);
  }
  if (valid && updatePreview) {
    state.colors.on = palette.primary;
    state.colors.off = palette.background;
    draw();
  }
  return valid ? palette : null;
}

function pushDocument() {
  vscode.postMessage({ type: "replace", text: state.model.serialize() });
}

function render() {
  const model = state.model;
  if (!model) return;
  if (!model.valid) {
    statusEl.textContent = "⚠ not a 40×40 KICON1 mask — edit as text";
  } else {
    let set = 0;
    for (let y = 0; y < KiconModel.SIZE; y += 1) {
      for (let x = 0; x < KiconModel.SIZE; x += 1) if (model.get(x, y)) set += 1;
    }
    statusEl.textContent = `${KiconModel.SIZE}×${KiconModel.SIZE} mask · ${set} set`;
  }
  draw();
}

function draw() {
  const model = state.model;
  ctx.fillStyle = state.colors.off;
  ctx.fillRect(0, 0, grid.width, grid.height);
  if (model.valid) {
    ctx.fillStyle = state.colors.on;
    for (let y = 0; y < KiconModel.SIZE; y += 1) {
      for (let x = 0; x < KiconModel.SIZE; x += 1) {
        if (model.get(x, y)) ctx.fillRect(x * CELL, y * CELL, CELL, CELL);
      }
    }
  }
  ctx.strokeStyle = state.colors.grid;
  for (let i = 0; i <= KiconModel.SIZE; i += 1) {
    ctx.beginPath();
    ctx.moveTo(i * CELL, 0);
    ctx.lineTo(i * CELL, grid.height);
    ctx.moveTo(0, i * CELL);
    ctx.lineTo(grid.width, i * CELL);
    ctx.stroke();
  }
}

function cellAt(event) {
  const rect = grid.getBoundingClientRect();
  const x = Math.floor(((event.clientX - rect.left) / rect.width) * KiconModel.SIZE);
  const y = Math.floor(((event.clientY - rect.top) / rect.height) * KiconModel.SIZE);
  if (x < 0 || y < 0 || x >= KiconModel.SIZE || y >= KiconModel.SIZE) return null;
  return { x, y };
}

function paint(event) {
  const cell = cellAt(event);
  if (!cell || !state.model || state.painting === null) return;
  if (state.model.set(cell.x, cell.y, state.painting)) {
    draw();
    pushDocument();
  }
}

grid.addEventListener("mousedown", (event) => {
  if (!state.model || !state.model.valid) return;
  // Left button sets pixels, right button clears; a drag continues that value.
  state.painting = event.button !== 2;
  paint(event);
});
grid.addEventListener("mousemove", (event) => {
  if (state.painting !== null) paint(event);
});
window.addEventListener("mouseup", () => {
  state.painting = null;
});
grid.addEventListener("contextmenu", (event) => event.preventDefault());

document.getElementById("apply-palette").addEventListener("click", () => {
  const palette = readPaletteControls(true);
  if (!palette) {
    paletteStatus.textContent = "Every color must use #RRGGBB.";
    paletteStatus.className = "error";
    return;
  }
  paletteStatus.textContent = "Applying…";
  paletteStatus.className = "";
  vscode.postMessage({ type: "applyPalette", palette });
});

vscode.postMessage({ type: "ready" });
