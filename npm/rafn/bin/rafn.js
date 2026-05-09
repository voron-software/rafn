#!/usr/bin/env node
"use strict";

const { spawnSync } = require("child_process");
const path = require("path");

const PLATFORM_PACKAGES = {
  "linux-x64": "@rafn/cli-linux-x64",
  "linux-arm64": "@rafn/cli-linux-arm64",
  "darwin-x64": "@rafn/cli-darwin-x64",
  "darwin-arm64": "@rafn/cli-darwin-arm64",
  "win32-x64": "@rafn/cli-win32-x64",
};

function getBinaryPath() {
  const key = `${process.platform}-${process.arch}`;
  const pkg = PLATFORM_PACKAGES[key];
  if (!pkg) {
    const supported = Object.keys(PLATFORM_PACKAGES).join(", ");
    throw new Error(`rafn: unsupported platform "${key}". Supported: ${supported}`);
  }
  let pkgDir;
  try {
    pkgDir = path.dirname(require.resolve(`${pkg}/package.json`));
  } catch {
    throw new Error(`rafn: cannot find package ${pkg}. Run \`npm install\` to reinstall.`);
  }
  const ext = process.platform === "win32" ? ".exe" : "";
  return path.join(pkgDir, `rafn${ext}`);
}

let binaryPath;
try {
  binaryPath = getBinaryPath();
} catch (err) {
  process.stderr.write(`${err.message}\n`);
  process.exit(1);
}

const result = spawnSync(binaryPath, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  process.stderr.write(`rafn: failed to run binary: ${result.error.message}\n`);
  process.exit(1);
}
process.exit(result.status ?? 1);
