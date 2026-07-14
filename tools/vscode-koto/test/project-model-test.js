"use strict";

const {
  validateAppId,
  appSlug,
  defaultAppDirectory,
  validateAppDirectory,
  scaffoldArgs,
} = require("../project-model.js");

let failures = 0;
function check(name, condition) {
  if (condition) console.log(`ok: ${name}`);
  else { failures += 1; console.error(`FAIL: ${name}`); }
}

check("valid reverse-DNS app id", validateAppId("dev.koto.apps.todo-list") === null);
check("uppercase app id rejected", Boolean(validateAppId("Dev.Koto.Bad")));
check("short app id rejected", Boolean(validateAppId("todo")));
check("slug replaces hyphens", appSlug("dev.koto.apps.todo-list") === "todo_list");
check("default directory", defaultAppDirectory("dev.koto.apps.todo-list") === "apps/todo_list");
check("app directory accepted", validateAppDirectory("apps/tools/todo") === null);
check("outside directory rejected", Boolean(validateAppDirectory("tools/todo")));
check("parent traversal rejected", Boolean(validateAppDirectory("apps/../todo")));
check("backslash rejected", Boolean(validateAppDirectory("apps\\todo")));

const args = scaffoldArgs("C:\\repo", "dev.koto.todo", "Todo App", "apps/todo");
check("arguments keep spaced name intact", args.includes("Todo App"));
check("arguments pass explicit root and dir", args.includes("C:\\repo") && args.includes("apps/todo"));

process.exit(failures === 0 ? 0 : 1);
