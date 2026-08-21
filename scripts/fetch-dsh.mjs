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
import { mkdtempSync, rmSync, cpSync, existsSync, lstatSync, readFileSync, writeFileSync, mkdirSync, readdirSync, readlinkSync, realpathSync, renameSync, symlinkSync, unlinkSync } from "node:fs";
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
  // shell: true —— Windows 上 npm 是 npm.cmd 批处理，execFileSync 无法直接执行
  // .cmd；经 shell 调用即可跨平台运行（参数均为内部常量或 registry 返回的版本号）。
  const out = execFileSync("npm", ["view", PKG_NAME, "versions", "--json"], {
    encoding: "utf8",
    shell: true,
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
        let target = join(root, relFromRoot);
        if (!existsSync(target)) {
          // 压平后嵌套包被上提到顶层 node_modules，原相对路径失效；
          // 取最后一个 node_modules/ 之后的片段，从顶层重新解析。
          const lastNM = relFromRoot.lastIndexOf("node_modules/");
          if (lastNM !== -1) {
            target = join(root, "node_modules", relFromRoot.slice(lastNM + "node_modules/".length));
          }
        }
        if (!existsSync(target)) {
          // 顶层也解析不到：.bin 软链只是开发期命令垫片，运行期不会被加载，
          // 直接删除失效软链，避免 tauri bundler 校验资源时报断链错误。
          unlinkSync(p);
          fixed++;
          continue;
        }
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

// node_modules 深度压平：makensis（NSIS 3.11）读不了超过 Windows MAX_PATH（260 字符）
// 的源文件路径，而 `--install-strategy nested` 的依赖树会产生很深的嵌套（rc.8 的
// pi-ai → @anthropic-ai/sdk → @aws-sdk/... 链曾到 8+ 层，最长路径 476 字符），打包时
// makensis 直接 "failed opening file"。此函数把嵌套依赖逐层上提/去重（等价于 npm
// hoisting + dedupe），直到树不再变化。幂等；同版本冗余副本删除、版本冲突保留嵌套，
// 不改变包解析结果。
function packageVersion(dir) {
  try {
    return JSON.parse(readFileSync(join(dir, "package.json"), "utf8")).version;
  } catch {
    return null;
  }
}

/** 把 src（包或 scope 目录）归并到 destDir：无同名 → 上移；同版本 → 删冗余/解软链；scope → 逐包归并。 */
function mergeInto(src, destDir) {
  const name = src.split(/[\\/]/).pop();
  const dest = join(destDir, name);
  let destExists = true;
  let destIsLink = false;
  try {
    destIsLink = lstatSync(dest).isSymbolicLink();
  } catch {
    destExists = false;
  }
  if (!destExists) {
    mkdirSync(destDir, { recursive: true });
    renameSync(src, dest);
    return true;
  }
  const sv = packageVersion(src);
  const dv = packageVersion(dest);
  if (sv && dv) {
    if (sv === dv) {
      if (destIsLink) {
        // dest 是软链：指向 src 或已失效时，删软链并把 src 放到位；否则 src 冗余可删
        let pointsToSrc = false;
        try {
          pointsToSrc = realpathSync(dest) === realpathSync(src);
        } catch {
          pointsToSrc = true; // dest 是断链 → 直接替换
        }
        if (pointsToSrc) {
          rmSync(dest, { force: true }); // 只删软链本身
          renameSync(src, dest);
        } else {
          rmSync(src, { recursive: true, force: true });
        }
      } else {
        rmSync(src, { recursive: true, force: true }); // 顶层同版本，冗余副本可删
      }
      return true;
    }
    return false; // 版本冲突，保留嵌套
  }
  if (!sv && !dv) {
    // 都是 scope 目录：逐包归并
    let changed = false;
    let entries;
    try {
      entries = readdirSync(src, { withFileTypes: true });
    } catch {
      return false;
    }
    for (const e of entries) {
      // 真实目录与软链都要归并（软链多为指向主 node_modules 的同版本冗余副本）
      if ((e.isDirectory() || e.isSymbolicLink()) && mergeInto(join(src, e.name), dest)) changed = true;
    }
    try {
      if (readdirSync(src).length === 0) rmSync(src, { recursive: true, force: true });
    } catch {
      /* 忽略 */
    }
    return changed;
  }
  return false;
}

/** 返回 dir 的「上一层 node_modules」：父级本身是 node_modules 目录时直接用父级，否则是父级下的 node_modules。 */
function parentNodeModules(dir) {
  const parent = dirname(dir);
  if (parent.split(/[\\/]/).pop() === "node_modules") return parent;
  return join(parent, "node_modules");
}

function flattenNodeModules(root) {
  let moved = 0;
  let changed = true;
  while (changed) {
    changed = false;
    const stack = [root];
    while (stack.length) {
      const dir = stack.pop();
      let entries;
      try {
        entries = readdirSync(dir, { withFileTypes: true });
      } catch {
        continue;
      }
      for (const e of entries) {
        if (!e.isDirectory()) continue;
        const p = join(dir, e.name);
        if (e.name === "node_modules") {
          // 把 p 下的子包归并到上一层 node_modules
          const parentNM = parentNodeModules(dir);
          if (parentNM.startsWith(root) && parentNM !== p) {
            let children;
            try {
              children = readdirSync(p, { withFileTypes: true });
            } catch {
              continue;
            }
            for (const c of children) {
              if (mergeInto(join(p, c.name), parentNM)) {
                moved++;
                changed = true;
              }
            }
          }
        }
        // 无论是否 node_modules 都要继续下探，否则深层嵌套永远扫不到
        stack.push(p);
      }
    }
  }
  return moved;
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
  const flattened = flattenNodeModules(DSH_DIR);
  if (flattened > 0) log(`压平了 ${flattened} 处嵌套 node_modules（规避 makensis 长路径）。`);
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
    { stdio: "inherit", shell: true },
  );
  const src = join(tmp, "node_modules", ...PKG_NAME.split("/")); // @deepseek-ai/dsh
  if (!existsSync(join(src, "node_modules", "@deepseek-ai"))) {
    throw new Error(`安装产物结构异常：缺少 ${join(src, "node_modules", "@deepseek-ai")}`);
  }
  log(`复制 ${src} → ${DSH_DIR}`);
  rmSync(DSH_DIR, { recursive: true, force: true });
  cpSync(src, DSH_DIR, { recursive: true });
  const stripped = stripSourceMappingUrls(DSH_DIR);
  const flattened = flattenNodeModules(DSH_DIR);
  log(`已更新 DSH 到 ${latest}${stripped > 0 ? `（剥除 ${stripped} 处 sourceMappingURL）` : ""}${flattened > 0 ? `（压平 ${flattened} 处嵌套 node_modules）` : ""}。`);
} catch (e) {
  log(`更新失败：${e.message}`);
  log(`继续使用当前已捆绑的 DSH${cur ? "（" + cur + "）" : ""}。`);
  process.exit(0);
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
// 临时安装目录已删除，node_modules/.bin 指向它的绝对软链此刻才真正断链，
// 修复必须放在 tmp 清理之后（更新流程内修复会因 existsSync 命中而全部跳过）。
const fixed = repairBrokenLinks(DSH_DIR, "/node_modules/@deepseek-ai/dsh/");
if (fixed > 0) log(`修复了 ${fixed} 条断链（node_modules/.bin 软链）。`);
