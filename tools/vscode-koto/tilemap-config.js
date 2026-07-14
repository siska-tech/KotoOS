// App-descriptor resolution for Koto tilemaps (KOTO-0198).
// Kept free of VS Code APIs so path/config behavior can be tested with Node.

"use strict";

const fs = require("fs");
const path = require("path");
const { KsprModel } = require("./media/kspr-model.js");

async function defaultReadText(filename) {
  try {
    return await fs.promises.readFile(filename, "utf8");
  } catch (error) {
    if (error.code === "ENOENT") return null;
    throw error;
  }
}

function insideDirectory(filename, directory) {
  const relative = path.relative(directory, filename);
  return relative !== "" && !relative.startsWith(`..${path.sep}`) &&
    relative !== ".." && !path.isAbsolute(relative);
}

function uniqueGlyphs(glyphs) {
  return [...new Set(Array.from(glyphs))];
}

function resolveTileset(appDir, maps, readText) {
  const hasTileset = Object.prototype.hasOwnProperty.call(maps, "tileset");
  const hasTiles = Object.prototype.hasOwnProperty.call(maps, "tiles");
  if (!hasTileset && !hasTiles) return Promise.resolve({});
  if (!hasTileset || !hasTiles) {
    return Promise.resolve({
      previewError: "app.json maps.tileset and maps.tiles must be declared together.",
    });
  }
  if (typeof maps.tileset !== "string" || !maps.tileset ||
      path.extname(maps.tileset).toLowerCase() !== ".kspr") {
    return Promise.resolve({ previewError: "app.json maps.tileset must be an app-relative .kspr path." });
  }
  const tilesetPath = path.resolve(appDir, maps.tileset);
  if (!insideDirectory(tilesetPath, appDir)) {
    return Promise.resolve({ previewError: "app.json maps.tileset must stay inside the app directory." });
  }
  if (!maps.tiles || typeof maps.tiles !== "object" || Array.isArray(maps.tiles)) {
    return Promise.resolve({ tilesetPath, previewError: "app.json maps.tiles must be a glyph-to-tile object." });
  }
  const glyphs = uniqueGlyphs(maps.glyphs);
  const keys = Object.keys(maps.tiles);
  const invalidKeys = keys.filter((glyph) => Array.from(glyph).length !== 1 || !glyphs.includes(glyph));
  const missing = glyphs.filter((glyph) => !Object.prototype.hasOwnProperty.call(maps.tiles, glyph));
  const invalidIds = keys.filter((glyph) =>
    !Number.isInteger(maps.tiles[glyph]) || maps.tiles[glyph] < 0
  );
  if (invalidKeys.length || missing.length || invalidIds.length || keys.length !== glyphs.length) {
    const details = [];
    if (missing.length) details.push(`missing glyphs ${missing.map((glyph) => JSON.stringify(glyph)).join(", ")}`);
    if (invalidKeys.length) details.push(`unknown glyphs ${invalidKeys.map((glyph) => JSON.stringify(glyph)).join(", ")}`);
    if (invalidIds.length) details.push(`invalid tile IDs for ${invalidIds.map((glyph) => JSON.stringify(glyph)).join(", ")}`);
    return Promise.resolve({
      tilesetPath,
      previewError: `Invalid app.json maps.tiles mapping: ${details.join("; ") || "glyph keys must be unique"}.`,
    });
  }
  return Promise.resolve(readText(tilesetPath)).then((tilesetText) => {
    if (tilesetText === null || tilesetText === undefined) {
      return { tilesetPath, previewError: `Tileset not found: ${maps.tileset}` };
    }
    const sprite = new KsprModel(tilesetText);
    const ids = sprite.tiles.map((tile) => Number(tile.id));
    const malformed = sprite.errors.length || sprite.tiles.some((tile, tileIndex) =>
      tile.rows.length !== 16 || !Number.isInteger(ids[tileIndex]) || ids[tileIndex] < 0 ||
      tile.rows.some((_, y) => Array.from(sprite.rowText(tileIndex, y)).some(
        (ch) => sprite.colorOf(ch) === null
      ))
    ) || new Set(ids).size !== ids.length;
    if (malformed) {
      return { tilesetPath, previewError: `Invalid .kspr tileset: ${sprite.errors[0] || "each tile must have 16 rows"}` };
    }
    const absent = keys.filter((glyph) =>
      !sprite.tiles.some((tile) => Number(tile.id) === maps.tiles[glyph])
    );
    if (absent.length) {
      return {
        tilesetPath,
        previewError: `Tileset has no mapped tile for glyphs ${absent.map((glyph) => JSON.stringify(glyph)).join(", ")}.`,
      };
    }
    return { tilesetPath, tilesetText, tiles: maps.tiles };
  });
}

/** Resolve the nearest ancestor app.json and its maps contract. */
async function resolveMapConfig(mapFilename, readText = defaultReadText) {
  const target = path.resolve(mapFilename);
  if (path.extname(target).toLowerCase() !== ".map") {
    return { ok: false, error: "Koto Tilemap Editor only opens .map files." };
  }

  let directory = path.dirname(target);
  while (true) {
    const descriptorPath = path.join(directory, "app.json");
    const descriptorText = await readText(descriptorPath);
    if (descriptorText !== null && descriptorText !== undefined) {
      let descriptor;
      try {
        descriptor = JSON.parse(descriptorText);
      } catch (error) {
        return { ok: false, descriptorPath, error: `Invalid app.json: ${error.message}` };
      }

      const maps = descriptor.maps;
      if (!maps || typeof maps !== "object" || Array.isArray(maps)) {
        return { ok: false, descriptorPath, error: "The nearest app.json has no maps declaration." };
      }
      if (typeof maps.dir !== "string" || maps.dir.length === 0) {
        return { ok: false, descriptorPath, error: "app.json maps.dir must be a non-empty string." };
      }
      if (!Number.isInteger(maps.width) || maps.width < 1 ||
          !Number.isInteger(maps.height) || maps.height < 1) {
        return { ok: false, descriptorPath, error: "app.json maps width and height must be positive integers." };
      }
      if (typeof maps.glyphs !== "string" || maps.glyphs.length === 0) {
        return { ok: false, descriptorPath, error: "app.json maps.glyphs must be a non-empty string." };
      }

      const mapDir = path.resolve(directory, maps.dir);
      if (!insideDirectory(target, mapDir)) {
        return {
          ok: false,
          descriptorPath,
          error: `This file is not under the configured maps directory (${maps.dir}).`,
        };
      }
      const tileset = await resolveTileset(directory, maps, readText);
      return {
        ok: true,
        descriptorPath,
        appDir: directory,
        mapDir,
        width: maps.width,
        height: maps.height,
        glyphs: maps.glyphs,
        ...tileset,
      };
    }

    const parent = path.dirname(directory);
    if (parent === directory) break;
    directory = parent;
  }
  return { ok: false, error: "No ancestor app.json describes this tilemap." };
}

module.exports = { resolveMapConfig, insideDirectory, resolveTileset };
