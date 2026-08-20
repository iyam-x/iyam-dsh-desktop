#!/usr/bin/env node
/**
 * fetch-dsh.mjs — 打包前校验并使用最新的 DSH 运行时
 *
 * DSH 运行时（@deepseek-ai/dsh）是外部拉取的本地副本，落在 src-tauri/bin/dsh-package
 * （.gitignore，不入库）。tauri build 只会原样打包该目录，不会自动更新。本脚本在每次
 * 打包前做"校验 + 按需更新"：
 *   1. 读当前已捆绑版本（dsh-package/package.json）。
 *   2. 查 registry 上 @deepseek-ai/dsh 的全部版本，取语义版本号最大者（含 prerelease，
 *      故 rc.8 > rc.7，即便 rc.8 只挂在 next 标签）。
 *   3. 当前已是最新 → 跳过；落后/缺失 → npm 安装最新版（nested 策略复刻嵌套依赖）并覆盖。
 *   4. 离线或 registry 不可达 → 告警并继续使用当前已捆绑版本，不阻断打包。
 *
 * 跳过开关：IYAM_SKIP_DSH_UPDATE=1 node scripts/fetch-dsh.mjs
 */

import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, cpSync, existsSync, readFileSync, writeFileSync, mkdirSync, readdirSync, readlinkSync, symlinkSync, unlinkSync } from "node:fs";
import { dirname, join, isAbsolute, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";

const PKG_NAME = "@deepseek-ai/dsh";
const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const DSH_DIR = join(ROOT, "src-tauri", "bin", "dsh-package");

function log(m) { console.log(`[fetch-dsh] ${m}`); }

if (process.env.IYAM_SKIP_DSH_UPDATE === "1") {
  log("IYAM_SKIP_DSH_UPDATE=1，跳过 DSH 更新校验");
  process.exit(0);
}

// ── 语义版本比较（支持 0.1.0-rc.8 这类 prerelease）──
function parseVer(v) {
  const m = String(v).match(/^(\d+)\.(\d+)\.(\d+)(?:-(.+))?$/);
  if (!m) return null;
  const pre = m[4]
    ? m[4].split(".").map((x) => (/^\d+$/.test(x) ? Number(x) : x))
    : [];
  return { major: +m[1], minor: +m[2], patch: +m[3], pre };
}
function cmpVer(a, b) {
  const x = parseVer(a);
  const y = parseVer(b);
  if (!x || !y) return 0;
  for (const k of ["major", "minor", "patch"]) {
    if (x[k] !== y[k]) return x[k] - y[k];
  }
  // 正式版（无 prerelease）优先于 prerelease
  if (x.pre.length === 0 && y.pre.length > 0) return 1;
  if (x.pre.length > 0 && y.pre.length === 0) return -1;
  const n = Math.max(x.pre.length, y.pre.length);
  for (let i = 0; i < n; i++) {
    const pa = x.pre[i];
    const pb = y.pre[i];
    if (pa === undefined) return -1;
    if (pb === undefined) return 1;
    if (typeof pa !== typeof pb) return typeof pa === "number" ? -1 : 1; // semver: 数字 < 标识符
    if (pa !== pb) return pa < pb ? -1 : 1;
  }
  return 0;
}

function currentVersion() {
  try {
    const p = JSON.parse(readFileSync(join(DSH_DIR, "package.json"), "utf8"));
    return p.version || null;
  } catch {
    return null;
  }
}

function latestVersion() {
  const out = execFileSync("npm", ["view", PKG_NAME, "versions", "--json"], {
    encoding: "utf8",
  });
  const arr = JSON.parse(out);
  if (!Array.isArray(arr) || arr.length === 0) {
    throw new Error("registry 未返回版本列表");
  }
  return arr.reduce((max, v) => (cmpVer(v, max) > 0 ? v : max), arr[0]);
}

// npm 用 --prefix 安装到临时目录时，node_modules/.bin 的软链会写成指向该临时目录的
// 绝对路径；复制到 dsh-package 且临时目录删除后全部断链，tauri-build 校验资源会报
// "resource path ... doesn't exist"。此函数把断链改写成包内相对路径。幂等。
function repairBrokenLinks(root, marker) {
  const stack = [root];
  let fixed = 0;
  while (stack.length) {
    const cur = stack.pop();
    let items;
    try {
      items = readdirSync(cur, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const e of items) {
      const p = join(cur, e.name);
      if (e.isSymbolicLink()) {
        let t;
        try {
          t = readlinkSync(p);
        } catch {
          continue;
        }
        const targetExists = isAbsolute(t)
          ? existsSync(t)
          : existsSync(join(dirname(p), t));
        if (targetExists) continue;
        const idx = t.indexOf(marker);
        if (idx === -1) continue; // 指向包外且已失效，无法修复，跳过
        const relFromRoot = t.slice(idx + marker.length);
        const target = join(root, relFromRoot);
        if (!existsSync(target)) continue;
        // 用 unlinkSync 而非 rmSync：断链目标可能是目录，rmSync(force) 会静默失败，
        // 留下旧链导致 symlinkSync 撞 EEXIST。unlinkSync 直接删软链本身。
        unlinkSync(p);
        symlinkSync(relative(dirname(p), target), p);
        fixed++;
      } else if (e.isDirectory()) {
        stack.push(p);
      }
    }
  }
  return fixed;
}

let cur;
try {
  cur = currentVersion();
} catch {
  cur = null;
}

// DSH 各客户端 bundle 末尾带 //# sourceMappingURL=client.js.map，但 .map 不随包分发，
// 浏览器会自动请求导致 404 刷屏（干扰 DevTools 里看 [rtui] 日志）。剥除该行，仅元数据。
function stripSourceMappingUrls(root) {
  const stack = [root];
  let count = 0;
  while (stack.length) {
    const cur = stack.pop();
    let items;
    try {
      items = readdirSync(cur, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const e of items) {
      const p = join(cur, e.name);
      if (e.isDirectory()) {
        stack.push(p);
        continue;
      }
      if (!e.isFile() || !p.endsWith(".js")) continue;
      const src = readFileSync(p, "utf8");
      const cleaned = src.replace(/^[ \t]*\/\/# sourceMappingURL=[^\n]*$/gm, "");
      if (cleaned !== src) {
        writeFileSync(p, cleaned);
        count++;
      }
    }
  }
  return count;
}

let latest;
try {
  latest = latestVersion();
} catch (e) {
  log(`查询 registry 失败（可能离线/无代理）：${e.message}`);
  log(`继续使用当前已捆绑的 DSH${cur ? "（" + cur + "）" : ""}，跳过更新。`);
  process.exit(0);
}

log(`当前捆绑: ${cur || "(无)"} ｜ registry 最新: ${latest}`);

if (cur && cmpVer(cur, latest) >= 0 && process.env.IYAM_FORCE_DSH_UPDATE !== "1") {
  log("已是最新，无需更新。");
  const fixed = repairBrokenLinks(DSH_DIR, "/node_modules/@deepseek-ai/dsh/");
  if (fixed > 0) log(`修复了 ${fixed} 条断链（node_modules/.bin 软链）。`);
  process.exit(0);
}

log(`需要更新到 ${latest} …`);
const tmp = mkdtempSync(join(tmpdir(), "iyam-dsh-"));
try {
  mkdirSync(join(tmp, "node_modules"), { recursive: true });
  writeFileSync(join(tmp, "package.json"), JSON.stringify({ name: "tmp", private: true, version: "0.0.0" }));
  execFileSync(
    "npm",
    [
      "install",
      `${PKG_NAME}@${latest}`,
      "--prefix", tmp,
      "--install-strategy", "nested",
      "--no-save",
      "--no-audit",
      "--no-fund",
      // DSH 含原生模块（node-pty / koffi / dsh-subprocess-local 等），其 install
      // 脚本负责下载/编译平台二进制；默认 allow-scripts 策略会拦截它们导致运行时缺
      // .node。DSH 是受信任的一方依赖，这里整体放行以保证原生产物完整。
      "--dangerously-allow-all-scripts",
    ],
    { stdio: "inherit" },
  );
  const src = join(tmp, "node_modules", ...PKG_NAME.split("/")); // @deepseek-ai/dsh
  if (!existsSync(join(src, "node_modules", "@deepseek-ai"))) {
    throw new Error(`安装产物结构异常：缺少 ${join(src, "node_modules", "@deepseek-ai")}`);
  }
  log(`复制 ${src} → ${DSH_DIR}`);
  rmSync(DSH_DIR, { recursive: true, force: true });
  cpSync(src, DSH_DIR, { recursive: true });
  const fixed = repairBrokenLinks(DSH_DIR, "/node_modules/@deepseek-ai/dsh/");
  const stripped = stripSourceMappingUrls(DSH_DIR);
  log(`已更新 DSH 到 ${latest}${fixed > 0 ? `（修复 ${fixed} 条断链）` : ""}${stripped > 0 ? `（剥除 ${stripped} 处 sourceMappingURL）` : ""}。`);
} catch (e) {
  log(`更新失败：${e.message}`);
  log(`继续使用当前已捆绑的 DSH${cur ? "（" + cur + "）" : ""}。`);
  process.exit(0);
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
