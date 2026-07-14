// `.kspr` sprite editor webview (KOTO-0192): pixels and buttons only.
// All document knowledge lives in KsprModel (kspr-model.js); every mutation
// serializes the whole text back to the extension host, which applies it to
// the TextDocument so undo/redo/save behave like any text edit.

"use strict";

/* global acquireVsCodeApi, KsprModel */

const vscode = acquireVsCodeApi();

const CELL = 32; // grid canvas is 512px for a 16x16 tile
const state = {
  model: null,
  tile: 0,
  ch: null, // selected palette char
  painting: false,
};

const grid = document.getElementById("grid");
const gridCtx = grid.getContext("2d");
const paletteEl = document.getElementById("palette");
const tilesEl = document.getElementById("tiles");
const statusEl = document.getElementById("status");
const hexInput = document.getElementById("hex-input");

window.addEventListener("message", (event) => {
  if (event.data.type === "document") {
    state.model = new KsprModel(event.data.text);
    if (state.tile >= state.model.tiles.length) state.tile = 0;
    if (!state.ch || !state.model.colorOf(state.ch)) {
      state.ch = state.model.palette[0]?.ch ?? null;
    }
    render();
  }
});

function pushDocument() {
  vscode.postMessage({ type: "replace", text: state.model.serialize() });
}

function render() {
  const model = state.model;
  if (!model) return;
  statusEl.textContent = model.errors.length
    ? `⚠ ${model.errors[0]}`
    : `tile ${state.tile} / ${model.tiles.length - 1} · ${model.palette.length} colors`;
  renderPalette();
  renderTiles();
  renderGrid();
}

function renderPalette() {
  paletteEl.replaceChildren();
  state.model.palette.forEach((entry, index) => {
    const swatch = document.createElement("button");
    swatch.className = "swatch" + (entry.ch === state.ch ? " selected" : "");
    swatch.style.background = `#${entry.hex}`;
    swatch.textContent = entry.ch;
    swatch.title = `${entry.ch} #${entry.hex} — click: select, double-click: edit color`;
    swatch.addEventListener("click", () => {
      state.ch = entry.ch;
      render();
    });
    swatch.addEventListener("dblclick", () => {
      askHex(`color ${entry.ch}`, entry.hex, (hex) => {
        if (state.model.setColor(index, hex)) pushDocument();
      });
    });
    paletteEl.appendChild(swatch);
  });
}

function renderTiles() {
  tilesEl.replaceChildren();
  state.model.tiles.forEach((tile, index) => {
    const item = document.createElement("div");
    item.className = "tile-item" + (index === state.tile ? " selected" : "");
    const mini = document.createElement("canvas");
    mini.width = 32;
    mini.height = 32;
    drawTile(mini.getContext("2d"), index, 2);
    const label = document.createElement("span");
    label.textContent = `${tile.id} ${tile.name}`;
    item.append(mini, label);
    item.addEventListener("click", () => {
      state.tile = index;
      render();
    });
    tilesEl.appendChild(item);
  });
}

function drawTile(ctx, tileIndex, cell) {
  const model = state.model;
  const tile = model.tiles[tileIndex];
  if (!tile || tile.rows.length !== KsprModel.TILE) return;
  for (let y = 0; y < KsprModel.TILE; y += 1) {
    const row = model.rowText(tileIndex, y);
    for (let x = 0; x < KsprModel.TILE; x += 1) {
      ctx.fillStyle = `#${model.colorOf(row[x]) ?? "FF00FF"}`;
      ctx.fillRect(x * cell, y * cell, cell, cell);
    }
  }
}

function renderGrid() {
  gridCtx.clearRect(0, 0, grid.width, grid.height);
  drawTile(gridCtx, state.tile, CELL);
  gridCtx.strokeStyle = "rgba(128,128,128,0.35)";
  for (let i = 0; i <= KsprModel.TILE; i += 1) {
    gridCtx.beginPath();
    gridCtx.moveTo(i * CELL, 0);
    gridCtx.lineTo(i * CELL, grid.height);
    gridCtx.moveTo(0, i * CELL);
    gridCtx.lineTo(grid.width, i * CELL);
    gridCtx.stroke();
  }
}

function cellAt(event) {
  const rect = grid.getBoundingClientRect();
  const x = Math.floor(((event.clientX - rect.left) / rect.width) * KsprModel.TILE);
  const y = Math.floor(((event.clientY - rect.top) / rect.height) * KsprModel.TILE);
  if (x < 0 || y < 0 || x >= KsprModel.TILE || y >= KsprModel.TILE) return null;
  return { x, y };
}

function paint(event) {
  const cell = cellAt(event);
  if (!cell || !state.ch || !state.model) return;
  if (state.model.setPixel(state.tile, cell.x, cell.y, state.ch)) {
    renderGrid();
    renderTiles();
    pushDocument();
  }
}

grid.addEventListener("mousedown", (event) => {
  if (event.button === 2) {
    // Right-click: eyedropper.
    const cell = cellAt(event);
    if (cell) {
      state.ch = state.model.getPixel(state.tile, cell.x, cell.y);
      render();
    }
    return;
  }
  state.painting = true;
  paint(event);
});
grid.addEventListener("mousemove", (event) => {
  if (state.painting) paint(event);
});
window.addEventListener("mouseup", () => {
  state.painting = false;
});
grid.addEventListener("contextmenu", (event) => event.preventDefault());

document.getElementById("add-color").addEventListener("click", () => {
  askHex("new color hex", "FFFFFF", (hex) => {
    const used = new Set(state.model.palette.map((p) => p.ch));
    const candidates =
      ".,:;-=+*oxOX%&@abcdefghijklmnpqrstuvwyz0123456789ABCDEFGHIJKLMNPQ";
    const ch = [...candidates].find((c) => !used.has(c));
    if (ch && state.model.addColor(ch, hex)) {
      state.ch = ch;
      pushDocument();
    }
  });
});

document.getElementById("add-tile").addEventListener("click", () => {
  if (state.ch && state.model.addTile("", state.ch)) {
    state.tile = state.model.tiles.length - 1;
    pushDocument();
  }
});

/** Tiny inline hex prompt (webviews have no window.prompt). */
function askHex(label, initial, done) {
  hexInput.hidden = false;
  hexInput.value = initial;
  hexInput.placeholder = label;
  hexInput.focus();
  hexInput.select();
  const finish = (commit) => {
    hexInput.hidden = true;
    hexInput.onkeydown = null;
    hexInput.onblur = null;
    if (commit && /^[0-9a-fA-F]{6}$/.test(hexInput.value)) done(hexInput.value);
  };
  hexInput.onkeydown = (event) => {
    if (event.key === "Enter") finish(true);
    if (event.key === "Escape") finish(false);
  };
  hexInput.onblur = () => finish(false);
}

vscode.postMessage({ type: "ready" });
