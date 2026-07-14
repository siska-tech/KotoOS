// Regression tests for the KOTO-0198 tilemap model and app.json resolver.

"use strict";

const fs = require("fs");
const os = require("os");
const path = require("path");
const { KsprModel } = require("../media/kspr-model.js");
const { TilemapModel, tilePreview } = require("../media/tilemap-model.js");
const { resolveMapConfig } = require("../tilemap-config.js");

const ROOT = path.join(__dirname, "..", "..", "..");
const FIXTURE = path.join(ROOT, "apps", "sokoban", "maps", "01-switchback.map");
const TILESET_FIXTURE = path.join(ROOT, "apps", "samples", "retained_tilemap", "sprites", "tiles.kspr");
const CONFIG = { width: 10, height: 8, glyphs: "#.oO*@ " };
let failures = 0;

function check(name, condition) {
  if (condition) console.log(`ok: ${name}`);
  else {
    failures += 1;
    console.error(`FAIL: ${name}`);
  }
}

function changedLines(a, b) {
  const left = a.split(/\r\n|\r|\n/);
  const right = b.split(/\r\n|\r|\n/);
  let count = Math.abs(left.length - right.length);
  for (let i = 0; i < Math.min(left.length, right.length); i += 1) {
    if (left[i] !== right[i]) count += 1;
  }
  return count;
}

function hasError(model, fragment) {
  return model.errors.some((error) => error.includes(fragment));
}

async function run() {
  const original = fs.readFileSync(FIXTURE, "utf8");

  check("untouched LF round trip", new TilemapModel(original, CONFIG).serialize() === original);
  const crlf = original.replace(/\n/g, "\r\n");
  check("untouched CRLF round trip", new TilemapModel(crlf, CONFIG).serialize() === crlf);
  const noTrailing = original.replace(/\n$/, "");
  check(
    "untouched no-trailing-newline round trip",
    new TilemapModel(noTrailing, CONFIG).serialize() === noTrailing
  );

  const palette = new TilemapModel(original, CONFIG).palette();
  check("palette contains each configured glyph", palette.length === 7);
  check("space glyph has a visible label", palette.at(-1).glyph === " " && palette.at(-1).label === "space");

  const sprite = new KsprModel(fs.readFileSync(TILESET_FIXTURE, "utf8"));
  const grass = tilePreview(sprite, { ".": 0 }, ".");
  check("mapped tile preview resolves", grass?.id === 0 && grass.pixels.length === 16);
  check("mapped tile preview contains RGB colors", grass?.pixels[0][0] === "2A7D4F");
  check("unmapped glyph has no tile preview", tilePreview(sprite, { ".": 0 }, "#") === null);

  const edited = new TilemapModel(original, CONFIG);
  check("paint changes a cell", edited.setCell(1, 1, "#"));
  check("paint changes exactly one row", changedLines(original, edited.serialize()) === 1);
  check("paint reads back", new TilemapModel(edited.serialize(), CONFIG).getCell(1, 1) === "#");
  check("no-op paint is rejected", edited.setCell(1, 1, "#") === false);
  check("unconfigured glyph is rejected", edited.setCell(1, 1, "!") === false);

  const rows = original.trimEnd().split("\n");
  check("valid Sokoban fixture", new TilemapModel(original, CONFIG).errors.length === 0);
  check("row-count validation", hasError(new TilemapModel(rows.slice(0, 7).join("\n"), CONFIG), "rows"));
  check(
    "row-width validation",
    hasError(new TilemapModel([rows[0].slice(1), ...rows.slice(1)].join("\n"), CONFIG), "wide")
  );
  check(
    "illegal-glyph validation",
    hasError(new TilemapModel(original.replace("#", "!"), CONFIG), "invalid glyphs")
  );
  check(
    "missing-start validation",
    hasError(new TilemapModel(original.replace("@", "."), CONFIG), "exactly one")
  );
  check(
    "duplicate-start validation",
    hasError(new TilemapModel(original.replace(".", "@"), CONFIG), "exactly one")
  );

  const temp = fs.mkdtempSync(path.join(os.tmpdir(), "koto-tilemap-test-"));
  try {
    const appDir = path.join(temp, "app");
    const mapDir = path.join(appDir, "maps");
    const spriteDir = path.join(appDir, "sprites");
    fs.mkdirSync(mapDir, { recursive: true });
    fs.mkdirSync(spriteDir, { recursive: true });
    const descriptorPath = path.join(appDir, "app.json");
    const baseMaps = { dir: "maps", ...CONFIG };
    const tiles = { "#": 1, ".": 0, "o": 0, "O": 0, "*": 0, "@": 3, " ": 0 };
    const writeMaps = (maps) => fs.writeFileSync(descriptorPath, JSON.stringify({ maps }));
    writeMaps(baseMaps);
    fs.writeFileSync(path.join(mapDir, "01.map"), original);
    fs.writeFileSync(path.join(appDir, "notes.map"), "not a map");
    fs.writeFileSync(path.join(mapDir, "legacy.txt"), original);
    const resolved = await resolveMapConfig(path.join(mapDir, "01.map"));
    check("nearest app.json map config resolves", resolved.ok && resolved.width === 10 && resolved.glyphs.endsWith(" "));
    const unrelated = await resolveMapConfig(path.join(appDir, "notes.map"));
    check("map outside maps.dir is refused", !unrelated.ok && unrelated.error.includes("not under"));
    const legacy = await resolveMapConfig(path.join(mapDir, "legacy.txt"));
    check("legacy txt map is refused", !legacy.ok && legacy.error.includes(".map"));

    fs.copyFileSync(TILESET_FIXTURE, path.join(spriteDir, "tiles.kspr"));
    writeMaps({ ...baseMaps, tileset: "sprites/tiles.kspr", tiles });
    const graphical = await resolveMapConfig(path.join(mapDir, "01.map"));
    check("valid tileset mapping resolves", graphical.ok && graphical.tilesetText && graphical.tiles["@"] === 3);
    check("tileset path is app confined", graphical.tilesetPath === path.join(spriteDir, "tiles.kspr"));

    writeMaps({ ...baseMaps, tileset: "../outside.kspr", tiles });
    check("outside tileset is diagnosed", (await resolveMapConfig(path.join(mapDir, "01.map"))).previewError.includes("inside"));
    writeMaps({ ...baseMaps, tileset: "sprites/tiles.txt", tiles });
    check("non-kspr tileset is diagnosed", (await resolveMapConfig(path.join(mapDir, "01.map"))).previewError.includes(".kspr"));
    const missingGlyph = { ...tiles };
    delete missingGlyph["@"];
    writeMaps({ ...baseMaps, tileset: "sprites/tiles.kspr", tiles: missingGlyph });
    check("missing glyph mapping is diagnosed", (await resolveMapConfig(path.join(mapDir, "01.map"))).previewError.includes("missing glyphs"));
    writeMaps({ ...baseMaps, tileset: "sprites/tiles.kspr", tiles: { ...tiles, "@": -1 } });
    check("negative tile ID is diagnosed", (await resolveMapConfig(path.join(mapDir, "01.map"))).previewError.includes("invalid tile IDs"));
    writeMaps({ ...baseMaps, tileset: "sprites/tiles.kspr", tiles: { ...tiles, "@": 99 } });
    check("absent tile ID is diagnosed", (await resolveMapConfig(path.join(mapDir, "01.map"))).previewError.includes("no mapped tile"));
    writeMaps({ ...baseMaps, tileset: "sprites/missing.kspr", tiles });
    const missingTileset = await resolveMapConfig(path.join(mapDir, "01.map"));
    check("missing tileset falls back with diagnostic", missingTileset.ok && missingTileset.previewError.includes("not found"));
    writeMaps(baseMaps);
    const glyphOnly = await resolveMapConfig(path.join(mapDir, "01.map"));
    check("tileset is optional for glyph fallback", glyphOnly.ok && !glyphOnly.previewError && !glyphOnly.tilesetText);
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }

  process.exitCode = failures === 0 ? 0 : 1;
}

run().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
