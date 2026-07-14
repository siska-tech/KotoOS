// Line-preserving document model for `.kicon` launcher icons (KOTO-0196).
//
// Same contract as the `.kspr` sprite model (KOTO-0192): the webview edits the
// committed text format in place — an untouched icon serializes byte-identically
// and toggling one pixel rewrites exactly one row line. A `.kicon` is the
// `KICON1` magic line plus 40 rows of 40 `#`/`.` mask characters.
//
// Loaded both as a webview <script> (window.KiconModel) and via node `require`.

"use strict";

const KICON_SIZE = 40;

class KiconModel {
  /** @param {string} text raw document text */
  constructor(text) {
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
    // Row segment indexes: everything after the KICON1 magic line.
    this.rowLines = [];
    this.valid = this.segments[0] && this.segments[0].content.trimEnd() === "KICON1";
    for (let i = 1; i < this.segments.length; i += 1) {
      if (this.segments[i].content.length === KICON_SIZE) this.rowLines.push(i);
    }
    if (this.rowLines.length !== KICON_SIZE) this.valid = false;
  }

  serialize() {
    return this.segments.map((s) => s.content + s.sep).join("");
  }

  /** True when pixel (x, y) is set (`#`). */
  get(x, y) {
    return this.segments[this.rowLines[y]].content[x] === "#";
  }

  /**
   * Set pixel (x, y); rewrites only that row's line. Returns true on change.
   */
  set(x, y, on) {
    if (x < 0 || y < 0 || x >= KICON_SIZE || y >= KICON_SIZE) return false;
    const lineIndex = this.rowLines[y];
    const line = this.segments[lineIndex].content;
    const ch = on ? "#" : ".";
    if (line[x] === ch) return false;
    this.segments[lineIndex].content = line.slice(0, x) + ch + line.slice(x + 1);
    return true;
  }
}

KiconModel.SIZE = KICON_SIZE;

if (typeof module !== "undefined" && module.exports) {
  module.exports = { KiconModel, KICON_SIZE };
}
if (typeof window !== "undefined") {
  window.KiconModel = KiconModel;
}
