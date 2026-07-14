// Line-preserving text tilemap model (KOTO-0198).
// Loaded by the webview and ordinary Node tests.

"use strict";

function splitSegments(text) {
  const segments = [];
  let start = 0;
  for (let i = 0; i < text.length; i += 1) {
    if (text[i] !== "\n" && text[i] !== "\r") continue;
    const sep = text[i] === "\r" && text[i + 1] === "\n" ? "\r\n" : text[i];
    segments.push({ content: text.slice(start, i), sep });
    if (sep === "\r\n") i += 1;
    start = i + 1;
  }
  if (start < text.length) segments.push({ content: text.slice(start), sep: "" });
  return segments;
}

function uniqueGlyphs(glyphs) {
  return [...new Set(Array.from(glyphs))];
}

function tilePreview(sprite, tiles, glyph) {
  if (!sprite || !tiles || !Object.prototype.hasOwnProperty.call(tiles, glyph)) return null;
  const id = tiles[glyph];
  const tileIndex = sprite.tiles.findIndex((tile) => Number(tile.id) === id);
  if (tileIndex < 0 || sprite.tiles[tileIndex].rows.length !== 16) return null;
  const pixels = [];
  for (let y = 0; y < 16; y += 1) {
    const row = sprite.rowText(tileIndex, y);
    if (Array.from(row).length !== 16) return null;
    const colors = Array.from(row).map((ch) => sprite.colorOf(ch));
    if (colors.some((color) => color === null)) return null;
    pixels.push(colors);
  }
  return { id, name: sprite.tiles[tileIndex].name, pixels };
}

class TilemapModel {
  constructor(text, config) {
    this.config = config;
    this.segments = splitSegments(text);
    this.validate();
  }

  rows() {
    const rows = this.segments.map((segment) => segment.content);
    while (rows.length && rows[rows.length - 1] === "") rows.pop();
    return rows;
  }

  serialize() {
    return this.segments.map((segment) => segment.content + segment.sep).join("");
  }

  palette() {
    return uniqueGlyphs(this.config.glyphs).map((glyph) => ({
      glyph,
      label: glyph === " " ? "space" : glyph,
    }));
  }

  validate() {
    const rows = this.rows();
    const allowed = new Set(uniqueGlyphs(this.config.glyphs));
    const errors = [];
    if (rows.length !== this.config.height) {
      errors.push(`map has ${rows.length} rows, expected ${this.config.height}`);
    }
    rows.forEach((row, index) => {
      const width = Array.from(row).length;
      if (width !== this.config.width) {
        errors.push(`row ${index + 1} is ${width} wide, expected ${this.config.width}`);
      }
    });
    const illegal = [...new Set(rows.flatMap((row) =>
      Array.from(row).filter((glyph) => !allowed.has(glyph))
    ))].sort();
    if (illegal.length) {
      errors.push(`invalid glyphs: ${illegal.map((glyph) => JSON.stringify(glyph)).join(", ")}`);
    }
    const starts = rows.reduce(
      (count, row) => count + Array.from(row).filter((glyph) => glyph === "@").length,
      0
    );
    if (starts !== 1) errors.push(`map has ${starts} '@' starts, expected exactly one`);
    this.errors = errors;
    return errors;
  }

  getCell(x, y) {
    const row = this.rows()[y];
    return row === undefined ? null : (Array.from(row)[x] ?? null);
  }

  setCell(x, y, glyph) {
    if (!Number.isInteger(x) || !Number.isInteger(y) ||
        x < 0 || y < 0 || x >= this.config.width || y >= this.config.height ||
        !uniqueGlyphs(this.config.glyphs).includes(glyph)) return false;
    const segment = this.segments[y];
    if (!segment) return false;
    const cells = Array.from(segment.content);
    if (cells.length !== this.config.width || cells[x] === glyph) return false;
    cells[x] = glyph;
    segment.content = cells.join("");
    this.validate();
    return true;
  }
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = { TilemapModel, splitSegments, uniqueGlyphs, tilePreview };
}
if (typeof window !== "undefined") {
  window.TilemapModel = TilemapModel;
  window.tilePreview = tilePreview;
}
