// Minimal, formatting-preserving app.json edits for KOTO-0196.
// This module deliberately has no VS Code dependency so its behavior can be
// pinned with ordinary Node tests.

"use strict";

const PALETTE_KEYS = [
  "background", "primary", "secondary", "accent", "highlight", "shadow",
];
const DEFAULT_PALETTE = {
  style: "mask",
  background: "#1A1424",
  primary: "#E8D27A",
  secondary: "#5C5470",
  accent: "#E34A4A",
  highlight: "#F6E27A",
  shadow: "#0C0A14",
};

function skipString(text, start) {
  let i = start + 1;
  while (i < text.length) {
    if (text[i] === "\\") i += 2;
    else if (text[i++] === '"') return i;
  }
  throw new Error("unterminated JSON string");
}

function skipWhitespace(text, start) {
  let i = start;
  while (/\s/.test(text[i] || "")) i += 1;
  return i;
}

function valueEnd(text, start) {
  let i = skipWhitespace(text, start);
  if (text[i] === '"') return skipString(text, i);
  if (text[i] === "{" || text[i] === "[") {
    const open = text[i];
    const close = open === "{" ? "}" : "]";
    let depth = 0;
    for (; i < text.length; i += 1) {
      if (text[i] === '"') {
        i = skipString(text, i) - 1;
      } else if (text[i] === open) {
        depth += 1;
      } else if (text[i] === close && --depth === 0) {
        return i + 1;
      }
    }
    throw new Error("unterminated JSON value");
  }
  while (i < text.length && !/[\s,}]/.test(text[i])) i += 1;
  return i;
}

function rootBounds(text) {
  const start = skipWhitespace(text, 0);
  if (text[start] !== "{") throw new Error("app.json root must be an object");
  const end = valueEnd(text, start);
  return { start, close: end - 1 };
}

function topLevelProperties(text) {
  const root = rootBounds(text);
  const properties = [];
  let i = root.start + 1;
  while (true) {
    i = skipWhitespace(text, i);
    if (text[i] === "}") break;
    if (text[i] !== '"') throw new Error("invalid app.json property");
    const keyStart = i;
    const keyEnd = skipString(text, i);
    const key = JSON.parse(text.slice(keyStart, keyEnd));
    i = skipWhitespace(text, keyEnd);
    if (text[i] !== ":") throw new Error("invalid app.json property separator");
    const start = skipWhitespace(text, i + 1);
    const end = valueEnd(text, start);
    properties.push({ key, keyStart, start, end });
    i = skipWhitespace(text, end);
    if (text[i] === ",") i += 1;
    else if (text[i] !== "}") throw new Error("invalid app.json object");
  }
  return { root, properties };
}

function formatInfo(text) {
  const eol = text.includes("\r\n") ? "\r\n" : "\n";
  const match = text.match(/(?:\r?\n)([ \t]+)"[^"\r\n]+"\s*:/);
  const indent = match ? match[1] : "  ";
  return { eol, indent };
}

function indentBlock(lines, indent, eol) {
  return lines.map((line, index) => index === 0 ? line : indent + line).join(eol);
}

function insertTopLevel(text, key, valueLines) {
  const { root, properties } = topLevelProperties(text);
  const { eol, indent } = formatInfo(text);
  const body = indentBlock(valueLines, indent, eol);
  const entry = `${indent}${JSON.stringify(key)}: ${body}`;
  if (properties.length === 0) {
    return text.slice(0, root.close) + eol + entry + eol + text.slice(root.close);
  }
  const last = properties[properties.length - 1];
  return text.slice(0, last.end) + "," + eol + entry + text.slice(last.end);
}

function paletteLines(palette) {
  return [
    "{",
    '  "style": "mask",',
    ...PALETTE_KEYS.map((key, index) =>
      `  ${JSON.stringify(key)}: ${JSON.stringify(palette[key])}${index < PALETTE_KEYS.length - 1 ? "," : ""}`
    ),
    "}",
  ];
}

function validatePalette(palette) {
  for (const key of PALETTE_KEYS) {
    if (!/^#[0-9A-Fa-f]{6}$/.test(palette[key] || "")) {
      throw new Error(`${key} must be #RRGGBB`);
    }
  }
}

function updatePalette(text, palette) {
  JSON.parse(text);
  validatePalette(palette);
  const normalized = Object.fromEntries(
    PALETTE_KEYS.map((key) => [key, palette[key].toUpperCase()])
  );
  const found = topLevelProperties(text).properties.find((p) => p.key === "shell_icon");
  const { eol, indent } = formatInfo(text);
  const replacement = indentBlock(paletteLines(normalized), indent, eol);
  if (!found) return insertTopLevel(text, "shell_icon", paletteLines(normalized));
  return text.slice(0, found.start) + replacement + text.slice(found.end);
}

function resourceLines(resource) {
  return [
    "{",
    `  "source": ${JSON.stringify(resource.source)},`,
    `  "output": ${JSON.stringify(resource.output)}`,
    "}",
  ];
}

function addResource(text, kind, resource) {
  const parsed = JSON.parse(text);
  if (kind !== "images" && kind !== "audio") throw new Error("unsupported resource kind");
  if (!resource.source || !resource.output) throw new Error("resource requires source and output");
  const existing = parsed[kind] || [];
  if (!Array.isArray(existing)) throw new Error(`${kind} must be an array`);
  if (existing.some((item) => item.source === resource.source)) {
    throw new Error(`${resource.source} is already registered`);
  }
  if (existing.some((item) => item.output === resource.output)) {
    throw new Error(`${resource.output} is already used as an output`);
  }

  const found = topLevelProperties(text).properties.find((p) => p.key === kind);
  if (!found) return insertTopLevel(text, kind, ["[", ...resourceLines(resource).map((x) => `  ${x}`), "]"]);

  const { eol, indent } = formatInfo(text);
  const arrayText = text.slice(found.start, found.end);
  const closeOffset = arrayText.lastIndexOf("]");
  if (closeOffset < 0) throw new Error(`${kind} must be an array`);
  const close = found.start + closeOffset;
  const itemIndent = indent + indent;
  const item = indentBlock(resourceLines(resource), itemIndent, eol);
  if (existing.length === 0) {
    return text.slice(0, close) + eol + itemIndent + item + eol + indent + text.slice(close);
  }
  let last = close - 1;
  while (last >= found.start && /\s/.test(text[last])) last -= 1;
  return text.slice(0, last + 1) + "," + eol + itemIndent + item + text.slice(last + 1);
}

module.exports = {
  PALETTE_KEYS, DEFAULT_PALETTE, updatePalette, addResource,
};
