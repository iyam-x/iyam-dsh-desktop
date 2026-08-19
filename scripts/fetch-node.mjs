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
import { mkdirSync, writeFileSync, rmSync, renameSync, chmodSync, existsSync } from "node:fs";
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

  // 统一用系统 tar（macOS/Linux 的 bsdtar 支持 zip，Linux 的 GNU tar 支持 tar.gz）
  // Windows 显式用系统自带的 bsdtar：Git Bash 的 GNU tar 既不支持 zip，也会把盘符
  // 路径 "C:\..."（含冒号）误认为远程主机。
  const tar =
    process.platform === "win32" && existsSync("C:\\Windows\\System32\\tar.exe")
      ? "C:\\Windows\\System32\\tar.exe"
      : "tar";
  // 只解压目标成员、不用 --strip-components：Windows 的 bsdtar 在「指定成员 + strip」
  // 组合下会静默产出空结果，解压后手动把文件挪到目标位置。
  const entry = TARGETS[targetName].entry;
  execFileSync(tar, ["-xf", archive, "-C", destDir, entry], { stdio: "inherit" });

  // 解出的文件带中间目录（如 node-v24.19.0-win-x64/node.exe），挪到 destDir 根并清理
  const exeName = TARGETS[targetName].exe;
  const extracted = join(destDir, entry);
  const finalPath = join(destDir, exeName);
  renameSync(extracted, finalPath);
  rmSync(dirname(extracted), { recursive: true, force: true });

  if (process.platform !== "win32") {
    chmodSync(finalPath, 0o755);
  }
  console.log(`→ ${finalPath}`);
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
