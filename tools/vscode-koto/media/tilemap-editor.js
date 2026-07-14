// Glyph and .kspr-backed tilemap custom editor webview (KOTO-0198/KOTO-0203).

"use strict";

/* global acquireVsCodeApi, TilemapModel, KsprModel, tilePreview */

const vscode = acquireVsCodeApi();
const state = { model: null, sprite: null, previews: new Map(), glyph: null, painting: false };
const paletteEl = document.getElementById("palette");
const gridEl = document.getElementById("grid");
const statusEl = document.getElementById("status");
const unavailableEl = document.getElementById("unavailable");
const editorEl = document.getElementById("editor");

window.addEventListener("message", (event) => {
  const message = event.data;
  if (message.type === "unavailable") {
    state.model = null;
    editorEl.hidden = true;
    unavailableEl.hidden = false;
    unavailableEl.textContent = message.message;
  } else if (message.type === "document") {
    state.model = new TilemapModel(message.text, message.config);
    state.sprite = message.config.tilesetText ? new KsprModel(message.config.tilesetText) : null;
    state.previews = new Map(state.model.palette().map((entry) => [
      entry.glyph,
      tilePreview(state.sprite, message.config.tiles, entry.glyph),
    ]));
    if (!state.model.palette().some((entry) => entry.glyph === state.glyph)) {
      state.glyph = state.model.palette()[0]?.glyph ?? null;
    }
    editorEl.hidden = false;
    unavailableEl.hidden = true;
    render();
  }
});

function visibleGlyph(glyph) {
  return glyph === " " ? "␠" : glyph;
}

function colorFor(index) {
  return `hsl(${(index * 137.508) % 360} 48% 34%)`;
}

function render() {
  renderPalette();
  renderGrid();
  statusEl.replaceChildren();
  const errors = [...state.model.errors];
  if (state.model.config.previewError) errors.push(state.model.config.previewError);
  if (!errors.length) {
    const mode = state.sprite ? "tile preview" : "glyph view";
    statusEl.textContent = `${state.model.config.width} × ${state.model.config.height} · valid · ${mode}`;
    statusEl.className = "valid";
    return;
  }
  statusEl.className = "invalid";
  const list = document.createElement("ul");
  for (const error of errors) {
    const item = document.createElement("li");
    item.textContent = error;
    list.appendChild(item);
  }
  statusEl.appendChild(list);
}

function renderPalette() {
  paletteEl.replaceChildren();
  state.model.palette().forEach((entry, index) => {
    const button = document.createElement("button");
    button.className = "swatch" + (entry.glyph === state.glyph ? " selected" : "");
    const preview = state.previews.get(entry.glyph);
    button.title = `${entry.label}${preview?.name ? ` · ${preview.name}` : ""}`;
    if (preview) appendPreview(button, preview, entry.glyph);
    else {
      button.textContent = visibleGlyph(entry.glyph);
      button.style.backgroundColor = colorFor(index);
    }
    button.addEventListener("click", () => {
      state.glyph = entry.glyph;
      renderPalette();
    });
    paletteEl.appendChild(button);
  });
}

function renderGrid() {
  gridEl.replaceChildren();
  gridEl.style.gridTemplateColumns = `repeat(${state.model.config.width}, 32px)`;
  for (let y = 0; y < state.model.config.height; y += 1) {
    for (let x = 0; x < state.model.config.width; x += 1) {
      const glyph = state.model.getCell(x, y);
      const cell = document.createElement("button");
      cell.className = "cell" + (glyph === null ? " missing" : "");
      cell.setAttribute("aria-label", glyph === null ? "missing cell" : `glyph ${visibleGlyph(glyph)}`);
      const paletteIndex = state.model.palette().findIndex((entry) => entry.glyph === glyph);
      const preview = state.previews.get(glyph);
      if (preview) appendPreview(cell, preview, null);
      else {
        cell.textContent = glyph === null ? "?" : visibleGlyph(glyph);
        if (paletteIndex >= 0) cell.style.backgroundColor = colorFor(paletteIndex);
      }
      cell.dataset.x = String(x);
      cell.dataset.y = String(y);
      cell.addEventListener("mousedown", onCellDown);
      cell.addEventListener("mouseenter", onCellEnter);
      cell.addEventListener("contextmenu", (event) => event.preventDefault());
      gridEl.appendChild(cell);
    }
  }
}

function appendPreview(parent, preview, glyph) {
  const canvas = document.createElement("canvas");
  canvas.width = 16;
  canvas.height = 16;
  canvas.className = "tile-preview";
  const context = canvas.getContext("2d");
  preview.pixels.forEach((row, y) => row.forEach((hex, x) => {
    context.fillStyle = `#${hex}`;
    context.fillRect(x, y, 1, 1);
  }));
  parent.appendChild(canvas);
  if (glyph !== null) {
    const label = document.createElement("span");
    label.className = "glyph-label";
    label.textContent = visibleGlyph(glyph);
    parent.appendChild(label);
  }
}

function cellPosition(event) {
  return { x: Number(event.currentTarget.dataset.x), y: Number(event.currentTarget.dataset.y) };
}

function paint(x, y) {
  if (!state.model || state.glyph === null) return;
  if (state.model.setCell(x, y, state.glyph)) {
    render();
    vscode.postMessage({ type: "replace", text: state.model.serialize() });
  }
}

function onCellDown(event) {
  const { x, y } = cellPosition(event);
  if (event.button === 2) {
    const glyph = state.model.getCell(x, y);
    if (glyph !== null && state.model.palette().some((entry) => entry.glyph === glyph)) {
      state.glyph = glyph;
      renderPalette();
    }
    return;
  }
  if (event.button !== 0) return;
  state.painting = true;
  paint(x, y);
}

function onCellEnter(event) {
  if (!state.painting) return;
  const { x, y } = cellPosition(event);
  paint(x, y);
}

window.addEventListener("mouseup", () => { state.painting = false; });
vscode.postMessage({ type: "ready" });
