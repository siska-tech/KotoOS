// Node tests for the .kspr document model (KOTO-0192). Run from the repo
// root: `node tools/vscode-koto/test/model-test.js`. Verifies the custom
// editor's determinism contract (byte-identical untouched save, one-line
// pixel diffs) and that edited output stays compilable by koto-img.

"use strict";

const fs = require("fs");
const path = require("path");
const os = require("os");
const { execFileSync } = require("child_process");
const { KsprModel } = require("../media/kspr-model.js");

const ROOT = path.join(__dirname, "..", "..", "..");
const FIXTURE = path.join(ROOT, "apps", "kotorogue", "sprites", "tiles.kspr");

let failures = 0;
function check(name, condition) {
  if (condition) {
    console.log(`ok: ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL: ${name}`);
  }
}

function diffLines(a, b) {
  const la = a.split(/\r\n|\n/);
  const lb = b.split(/\r\n|\n/);
  let count = Math.abs(la.length - lb.length);
  for (let i = 0; i < Math.min(la.length, lb.length); i += 1) {
    if (la[i] !== lb[i]) count += 1;
  }
  return count;
}

function compiles(text) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "kspr-model-test-"));
  const src = path.join(dir, "edited.kspr");
  const png = path.join(dir, "edited.png");
  fs.writeFileSync(src, text);
  try {
    execFileSync("cargo", ["run", "-q", "-p", "koto-img", "--", "kspr2png", src, png], {
      cwd: ROOT,
      stdio: "pipe",
    });
    return true;
  } catch (error) {
    console.error(String(error.stderr || error));
    return false;
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

const original = fs.readFileSync(FIXTURE, "utf8");

// 1. Untouched round trip is byte-identical (LF fixture and a CRLF copy).
check("untouched LF round trip", new KsprModel(original).serialize() === original);
const crlf = original.replace(/\n/g, "\r\n");
check("untouched CRLF round trip", new KsprModel(crlf).serialize() === crlf);
const noTrailing = original.replace(/\n$/, "");
check(
  "untouched no-trailing-newline round trip",
  new KsprModel(noTrailing).serialize() === noTrailing
);

// 2. One pixel edit = exactly one changed line, still compiles.
{
  const model = new KsprModel(original);
  const before = model.getPixel(0, 3, 3);
  const replacement = model.palette.find((p) => p.ch !== before).ch;
  check("setPixel reports a change", model.setPixel(0, 3, 3, replacement));
  const edited = model.serialize();
  check("pixel edit diffs exactly one line", diffLines(original, edited) === 1);
  check("pixel edit reads back", new KsprModel(edited).getPixel(0, 3, 3) === replacement);
  check("edited sheet compiles (koto-img)", compiles(edited));
}

// 3. No-op paint leaves the document untouched.
{
  const model = new KsprModel(original);
  const ch = model.getPixel(0, 0, 0);
  check("no-op paint returns false", model.setPixel(0, 0, 0, ch) === false);
  check("no-op paint keeps bytes", model.serialize() === original);
  check("unknown char rejected", model.setPixel(0, 0, 0, "?") === false);
}

// 4. Palette edit is a one-line diff; add color / add tile stay compilable.
{
  const model = new KsprModel(original);
  check("setColor", model.setColor(0, "123456"));
  check("setColor diffs one line", diffLines(original, model.serialize()) === 1);
  check("setColor rejects bad hex", model.setColor(0, "12345Z") === false);

  const grown = new KsprModel(original);
  check("addColor", grown.addColor("Z", "0FF00F"));
  check("addColor rejects duplicates", grown.addColor("Z", "000000") === false);
  const tilesBefore = grown.tiles.length;
  check("addTile", grown.addTile("scratch", "Z"));
  check("addTile grows the sheet", grown.tiles.length === tilesBefore + 1);
  check("grown sheet compiles (koto-img)", compiles(grown.serialize()));
}

process.exit(failures === 0 ? 0 : 1);
