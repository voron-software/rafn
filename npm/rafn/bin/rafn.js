#!/usr/bin/env node
"use strict";

const { spawnSync } = require("child_process");
const path = require("path");

const BINARIES = {
  "linux-x64":   "rafn-linux-x64",
  "linux-arm64": "rafn-linux-arm64",
  "darwin-x64":  "rafn-darwin-x64",
  "darwin-arm64": "rafn-darwin-arm64",
  "win32-x64":   "rafn-win32-x64.exe",
};

function getBinaryPath() {
  const key = `${process.platform}-${process.arch}`;
  const name = BINARIES[key];
  if (!name) {
    const supported = Object.keys(BINARIES).join(", ");
    throw new Error(`rafn: unsupported platform "${key}". Supported: ${supported}`);
  }
  return path.join(__dirname, name);
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
