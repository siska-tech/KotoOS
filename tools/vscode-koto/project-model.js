// Pure validation/default helpers for the new-app wizard.

"use strict";

const path = require("path");

const APP_ID = /^[a-z0-9]+(?:\.[a-z0-9-]+)+$/;

function validateAppId(value) {
  if (!APP_ID.test(value)) {
    return "Use a reverse-DNS ID such as dev.koto.apps.todo-list";
  }
  return null;
}

function appSlug(appId) {
  return appId.split(".").pop().replace(/-/g, "_");
}

function defaultAppDirectory(appId) {
  return `apps/${appSlug(appId)}`;
}

function validateAppDirectory(value) {
  if (!value) return "Enter a project directory under apps/";
  if (value.includes("\\")) return "Use `/` path separators";
  if (path.posix.isAbsolute(value) || !value.startsWith("apps/")) {
    return "The project directory must be relative and under apps/";
  }
  if (value.split("/").some((part) => part === ".." || part === "")) {
    return "The project directory cannot contain empty or `..` segments";
  }
  return null;
}

function scaffoldArgs(root, appId, name, appDirectory) {
  return [
    "run", "-q", "-p", "koto-app-scaffold", "--",
    "--root", root,
    "--app-id", appId,
    "--name", name,
    "--dir", appDirectory,
  ];
}

module.exports = {
  validateAppId,
  appSlug,
  defaultAppDirectory,
  validateAppDirectory,
  scaffoldArgs,
};
