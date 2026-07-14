// Node tests for the .kicon document model (KOTO-0196). Run from the repo
// root: `node tools/vscode-koto/test/kicon-model-test.js`. Same determinism
// contract as the .kspr editor: byte-identical untouched save, one-line diffs.

"use strict";

const fs = require("fs");
const path = require("path");
const { KiconModel, KICON_SIZE } = require("../media/kicon-model.js");

const ROOT = path.join(__dirname, "..", "..", "..");
const FIXTURE = path.join(ROOT, "apps", "kotorogue", "icon.kicon");

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

const original = fs.readFileSync(FIXTURE, "utf8");

// 1. Untouched round trips are byte-identical (LF, CRLF, no trailing newline).
check("untouched LF round trip", new KiconModel(original).serialize() === original);
const crlf = original.replace(/\n/g, "\r\n");
check("untouched CRLF round trip", new KiconModel(crlf).serialize() === crlf);
const noTrailing = original.replace(/\n$/, "");
check("untouched no-trailing-newline round trip", new KiconModel(noTrailing).serialize() === noTrailing);

// 2. The fixture parses as a valid 40x40 mask.
{
  const model = new KiconModel(original);
  check("fixture is valid", model.valid);
  check("fixture has 40 rows", model.rowLines.length === KICON_SIZE);
}

// 3. Toggling one pixel is a one-line diff and reads back.
{
  const model = new KiconModel(original);
  const before = model.get(5, 5);
  check("toggle reports a change", model.set(5, 5, !before));
  check("toggle reads back", model.get(5, 5) === !before);
  check("toggle diffs exactly one line", diffLines(original, model.serialize()) === 1);
  check("no-op set returns false", model.set(5, 5, !before) === false);
}

// 4. Out-of-range coordinates are rejected.
{
  const model = new KiconModel(original);
  check("out-of-range rejected", model.set(40, 0, true) === false);
  check("out-of-range keeps bytes", model.serialize() === original);
}

process.exit(failures === 0 ? 0 : 1);
