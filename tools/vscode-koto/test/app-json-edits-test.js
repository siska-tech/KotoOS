// Node regression tests for KOTO-0196 app.json palette/resource edits.

"use strict";

const {
  DEFAULT_PALETTE,
  updatePalette,
  addResource,
} = require("../app-json-edits.js");

let failures = 0;
function check(name, condition) {
  if (condition) console.log(`ok: ${name}`);
  else {
    failures += 1;
    console.error(`FAIL: ${name}`);
  }
}

const base = [
  "{",
  '  "app_id": "dev.koto.demo",',
  '  "name": "Demo",',
  '  "memory": { "sram_work_bytes": 1234 }',
  "}",
  "",
].join("\n");

// Palette insertion is valid and leaves unrelated text alone.
const withPalette = updatePalette(base, DEFAULT_PALETTE);
const parsedPalette = JSON.parse(withPalette);
check("palette inserted", parsedPalette.shell_icon.primary === DEFAULT_PALETTE.primary);
check("palette style inserted", parsedPalette.shell_icon.style === "mask");
check("unrelated compact object preserved", withPalette.includes('"memory": { "sram_work_bytes": 1234 }'));
check("palette indentation follows descriptor", withPalette.includes('\n  "shell_icon": {\n    "style"'));
check("LF and trailing newline preserved", withPalette.endsWith("\n") && !withPalette.includes("\r\n"));

// Updating replaces only shell_icon and preserves following fields.
const changed = updatePalette(withPalette, { ...DEFAULT_PALETTE, primary: "#abcdef" });
check("palette normalized on update", JSON.parse(changed).shell_icon.primary === "#ABCDEF");
check("palette update has one shell_icon", (changed.match(/"shell_icon"/g) || []).length === 1);
check("following field remains", JSON.parse(changed).memory.sram_work_bytes === 1234);

// Missing arrays are inserted and existing arrays are appended in order.
const oneImage = addResource(base, "images", {
  source: "sprites/hero.kspr",
  output: "sprites/hero.kim",
});
check("missing images array inserted", JSON.parse(oneImage).images.length === 1);
check("resource indentation follows descriptor", oneImage.includes('\n  "images": [\n    {\n      "source"'));
const twoImages = addResource(oneImage, "images", {
  source: "sprites/items.kspr",
  output: "sprites/items.kim",
});
const images = JSON.parse(twoImages).images;
check("image appended", images.length === 2 && images[1].source === "sprites/items.kspr");
check("existing image order preserved", images[0].source === "sprites/hero.kspr");

const emptyAudio = base.replace(
  '  "memory": { "sram_work_bytes": 1234 }',
  '  "audio": [],\n  "memory": { "sram_work_bytes": 1234 }'
);
const oneAudio = addResource(emptyAudio, "audio", {
  source: "audio/click.kacl",
  output: "audio/click.kacl",
});
check("empty audio array filled", JSON.parse(oneAudio).audio.length === 1);

// Newline style and final-newline state are stable.
const crlfNoTrailing = base.replace(/\n/g, "\r\n").replace(/\r\n$/, "");
const crlfEdited = addResource(crlfNoTrailing, "audio", {
  source: "audio/bgm.kmml",
  output: "audio/bgm.kmml",
});
check("CRLF preserved", crlfEdited.includes("\r\n") && !/(^|[^\r])\n/.test(crlfEdited));
check("no trailing newline preserved", crlfEdited.endsWith("}"));

// Invalid edits fail without producing text.
for (const [name, action] of [
  ["invalid palette rejected", () => updatePalette(base, { ...DEFAULT_PALETTE, accent: "red" })],
  ["duplicate source rejected", () => addResource(oneImage, "images", { source: "sprites/hero.kspr", output: "sprites/two.kim" })],
  ["duplicate output rejected", () => addResource(oneImage, "images", { source: "sprites/two.kspr", output: "sprites/hero.kim" })],
  ["unsupported kind rejected", () => addResource(base, "maps", { source: "maps/a.map", output: "maps/a.map" })],
]) {
  let threw = false;
  try { action(); } catch { threw = true; }
  check(name, threw);
}

process.exit(failures === 0 ? 0 : 1);
