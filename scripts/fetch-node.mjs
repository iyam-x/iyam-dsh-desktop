#!/usr/bin/env node
/**
 * fetch-node.mjs — 下载 Node.js 运行时二进制并打包进 app 资源
 *
 * 只提取 node 可执行文件（bin/node 或 node.exe），不携带 npm 等冗余。
 * 落盘位置：src-tauri/bin/node/<triple>/node(.exe)
 *
 * 用法：
 *   node scripts/fetch-node.mjs                  # 当前主机平台
 *   node scripts/fetch-node.mjs --target all     # 全部平台（CI 用）
 *   node scripts/fetch-node.mjs --target win32-x64
 *
 * 版本通过 DSH_NODE_VERSION 环境变量固定（默认 v24.19.0，Node 24 LTS）。
 */

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";

const NODE_VERSION = process.env.DSH_NODE_VERSION || "v24.19.0";
const NODE_MAJOR = NODE_VERSION.replace(/^v/, "").split(".")[0];

// 目标平台 → nodejs.org 产物命名
const TARGETS = {
  "darwin-arm64": { file: `node-${NODE_VERSION}-darwin-arm64.tar.gz`, exe: "node", entry: `node-${NODE_VERSION}-darwin-arm64/bin/node`, url: "https://nodejs.org/dist" },
  "darwin-x64": { file: `node-${NODE_VERSION}-darwin-x64.tar.gz`, exe: "node", entry: `node-${NODE_VERSION}-darwin-x64/bin/node`, url: "https://nodejs.org/dist" },
  "linux-x64": { file: `node-${NODE_VERSION}-linux-x64.tar.gz`, exe: "node", entry: `node-${NODE_VERSION}-linux-x64/bin/node`, url: "https://nodejs.org/dist" },
  "linux-arm64": { file: `node-${NODE_VERSION}-linux-arm64.tar.gz`, exe: "node", entry: `node-${NODE_VERSION}-linux-arm64/bin/node`, url: "https://nodejs.org/dist" },
  "win32-x64": { file: `node-${NODE_VERSION}-win-x64.zip`, exe: "node.exe", entry: `node-${NODE_VERSION}-win-x64/node.exe`, url: "https://nodejs.org/dist" },
  "win32-arm64": { file: `node-${NODE_VERSION}-win-arm64.zip`, exe: "node.exe", entry: `node-${NODE_VERSION}-win-arm64/node.exe`, url: "https://nodejs.org/dist" },
};

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const NODE_DIR = join(ROOT, "src-tauri", "bin", "node");

function hostTarget() {
  const { platform, arch } = process;
  const map = {
    darwin: { arm64: "darwin-arm64", x64: "darwin-x64" },
    linux: { x64: "linux-x64", arm64: "linux-arm64" },
    win32: { x64: "win32-x64", arm64: "win32-arm64" },
  };
  const t = map[platform]?.[arch];
  if (!t) {
    console.error(`不支持的平台/架构: ${platform}/${arch}`);
    process.exit(1);
  }
  return t;
}

function parseArgs(argv) {
  const args = { target: hostTarget() };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--target") args.target = argv[++i];
  }
  return args;
}

function download(url, dest) {
  console.log(`下载 ${url}`);
  const start = Date.now();
  // 用系统 curl（macOS/Linux/Windows 10+ 均自带）
  execFileSync("curl", ["-sSL", "--retry", "3", "-o", dest, url], {
    stdio: "inherit",
  });
  console.log(`完成 (${((Date.now() - start) / 1000).toFixed(1)}s)`);
}

function extract(archive, targetName) {
  const destDir = join(NODE_DIR, targetName);
  rmSync(destDir, { recursive: true, force: true });
  mkdirSync(destDir, { recursive: true });

  // 统一用系统 tar（macOS/Windows 的 bsdtar 支持 zip，Linux 的 GNU tar 支持 tar.gz）
  execFileSync(
    "tar",
    ["-xf", archive, "-C", destDir, "--strip-components=2", TARGETS[targetName].entry],
    { stdio: "inherit" }
  );

  if (process.platform !== "win32") {
    execFileSync("chmod", ["+x", join(destDir, TARGETS[targetName].exe)]);
  }
  console.log(`→ ${join(destDir, TARGETS[targetName].exe)}`);
}

function fetchOne(targetName) {
  const t = TARGETS[targetName];
  if (!t) {
    console.error(`未知目标: ${targetName}。可选: ${Object.keys(TARGETS).join(", ")}`);
    process.exit(1);
  }
  const archive = join(tmpdir(), t.file);
  const url = `${t.url}/${NODE_VERSION}/${t.file}`;
  download(url, archive);
  try {
    extract(archive, targetName);
  } finally {
    rmSync(archive, { force: true });
  }
}

const { target } = parseArgs(process.argv.slice(2));

console.log(`Node ${NODE_VERSION} (LTS v${NODE_MAJOR})`);

if (target === "all") {
  for (const name of Object.keys(TARGETS)) fetchOne(name);
} else {
  fetchOne(target);
}

writeFileSync(join(NODE_DIR, ".version"), `${NODE_VERSION}\n`);
console.log(`版本记录: src-tauri/bin/node/.version = ${NODE_VERSION}`);
