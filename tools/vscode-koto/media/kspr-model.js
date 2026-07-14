// Line-preserving document model for `.kspr` sprite-sheet text (KOTO-0192).
//
// The webview edits *lines of the committed text format*, never a parallel
// representation: an untouched document serializes byte-identically, and a
// pixel edit rewrites exactly one row line. Format correctness stays with
// the Rust toolchain (koto-img / build_apps.py) — this model only mirrors
// the line shapes it needs to edit (`color`, `tile`, 16-char pixel rows).
//
// Loaded both as a webview <script> (window.KsprModel) and via node
// `require` for the model tests.

"use strict";

const TILE = 16;

class KsprModel {
  /** @param {string} text raw document text */
  constructor(text) {
    // Split keeping each line's own separator so mixed / missing trailing
    // newlines survive round trips byte-for-byte.
    this.segments = [];
    const re = /([^\n]*(?:\r\n|\n|$))/g;
    let match;
    while ((match = re.exec(text)) !== null) {
      if (match[0] === "" && re.lastIndex >= text.length) break;
      const raw = match[0];
      const sep = raw.endsWith("\r\n") ? "\r\n" : raw.endsWith("\n") ? "\n" : "";
      this.segments.push({ content: raw.slice(0, raw.length - sep.length), sep });
      if (sep === "") break;
    }
    this.newline = text.includes("\r\n") ? "\r\n" : "\n";
    this.parse();
  }

  /** Rebuild palette / tile indexes from the current lines. */
  parse() {
    this.palette = []; // { ch, hex, line }
    this.tiles = []; // { id, name, headerLine, rows: [line…] }
    this.errors = [];
    for (let i = 0; i < this.segments.length; i += 1) {
      const s = this.segments[i].content.trim();
      if (s === "" || s.startsWith("#")) continue;
      if (s.startsWith("color ")) {
        const parts = s.split(/\s+/);
        if (parts.length === 3 && parts[1].length === 1 && parts[2].length === 6) {
          this.palette.push({ ch: parts[1], hex: parts[2].toUpperCase(), line: i });
        } else {
          this.errors.push(`line ${i + 1}: bad palette line`);
        }
      } else if (s.startsWith("tile ")) {
        const parts = s.split(/\s+/);
        this.tiles.push({
          id: parts[1] || String(this.tiles.length),
          name: parts[2] || "",
          headerLine: i,
          rows: [],
        });
      } else {
        const tile = this.tiles[this.tiles.length - 1];
        if (!tile) {
          this.errors.push(`line ${i + 1}: pixel row before any tile`);
        } else if (s.length !== TILE) {
          this.errors.push(`line ${i + 1}: row is ${s.length} chars, expected ${TILE}`);
        } else {
          tile.rows.push(i);
        }
      }
    }
  }

  serialize() {
    return this.segments.map((s) => s.content + s.sep).join("");
  }

  colorOf(ch) {
    const entry = this.palette.find((p) => p.ch === ch);
    return entry ? entry.hex : null;
  }

  /** The trimmed 16-char row text of tile t, row y. */
  rowText(t, y) {
    return this.segments[this.tiles[t].rows[y]].content.trim();
  }

  getPixel(t, x, y) {
    return this.rowText(t, y)[x];
  }

  /**
   * Paint one pixel; rewrites only that row's line (whitespace framing kept).
   * Returns true when the document changed.
   */
  setPixel(t, x, y, ch) {
    if (ch.length !== 1 || !this.colorOf(ch)) return false;
    const lineIndex = this.tiles[t].rows[y];
    const line = this.segments[lineIndex].content;
    const trimmed = line.trim();
    if (trimmed[x] === ch) return false;
    const start = line.indexOf(trimmed);
    const at = start + x;
    this.segments[lineIndex].content = line.slice(0, at) + ch + line.slice(at + 1);
    return true;
  }

  /** Change an existing palette entry's color; one-line edit. */
  setColor(index, hex) {
    if (!/^[0-9a-fA-F]{6}$/.test(hex)) return false;
    const entry = this.palette[index];
    if (!entry) return false;
    this.segments[entry.line].content = `color ${entry.ch} ${hex.toUpperCase()}`;
    entry.hex = hex.toUpperCase();
    return true;
  }

  /** Add a palette entry after the last color line (or at the top). */
  addColor(ch, hex) {
    if (ch.length !== 1 || /\s|#/.test(ch) || this.colorOf(ch)) return false;
    if (!/^[0-9a-fA-F]{6}$/.test(hex)) return false;
    const after = this.palette.length
      ? this.palette[this.palette.length - 1].line
      : -1;
    this.insertLines(after + 1, [`color ${ch} ${hex.toUpperCase()}`]);
    return true;
  }

  /** Append a new tile filled with the given palette char. */
  addTile(name, fillCh) {
    if (!this.colorOf(fillCh)) return false;
    const id = this.tiles.length;
    const lines = ["", `tile ${id} ${name || `t${id}`}`];
    for (let y = 0; y < TILE; y += 1) lines.push(fillCh.repeat(TILE));
    // Ensure the current last segment ends with a newline before appending.
    if (this.segments.length && this.segments[this.segments.length - 1].sep === "") {
      this.segments[this.segments.length - 1].sep = this.newline;
    }
    this.insertLines(this.segments.length, lines);
    return true;
  }

  insertLines(at, contents) {
    const inserted = contents.map((content) => ({ content, sep: this.newline }));
    this.segments.splice(at, 0, ...inserted);
    this.parse();
  }
}

KsprModel.TILE = TILE;

if (typeof module !== "undefined" && module.exports) {
  module.exports = { KsprModel, TILE };
}
if (typeof window !== "undefined") {
  window.KsprModel = KsprModel;
}
