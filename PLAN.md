# iyam-dsh-desktop — 完整实施计划

## 一、产品定位

将 `@deepseek-ai/dsh` 封装为跨平台原生桌面应用。用户无需任何 Node.js/npm 知识，双击图标即可使用完整 DSH 功能。

- **App 本体**：~14MB（Tauri 壳 + Rust 二进制 + 前端）
- **内置 DSH**：~333MB（完整 `@deepseek-ai/dsh` 包，打包进 app Resources）
- **内置 Node**：~116MB/平台（Node 24 LTS，随 app bundle 分发，零系统依赖）
- **总计**：~460MB（一次打包，永久使用，无需网络、无需系统安装 Node.js）
- **首次启动**：从内置资源复制到 `~/.iyam-dsh/`，瞬间完成
- **后续启动**：直接复用本地安装，秒级启动
- **内核不变**：`@deepseek-ai/dsh` 源码零修改，通过 App 内「检查更新」独立升级

---

## 二、技术选型

| 维度 | 选择 | 理由 |
|------|------|------|
| 壳框架 | **Tauri v2** | ~15MB 包体，复用系统 WebView，原生托盘/菜单 |
| 前端 | TypeScript + React + Vite | 快速开发，与 DSH 生态一致 |
| 后端 | Rust (Tauri) | 进程管理稳定，交叉编译友好 |
| Node 运行时 | **内置 Node 24 LTS**（`scripts/fetch-node.mjs` 按平台下载） | 零系统依赖，满足原生模块 ABI（node-pty napi-v9 / sharp ≥20.9） |
| DSH 调用 | Rust 直接 spawn `node lib/bin.js web` | 不经过 shell 脚本，Windows 天然支持 |
| UI 集成 | **WebviewWindow 直连** | 无 iframe 边框，完整原生窗口体验 |
| 分发 | macOS `.app/.dmg`，Windows `.exe`，Linux `.AppImage` | |

---

## 三、目录结构

```
iyam-dsh-desktop/
├── package.json                    # Tauri v2 + Vite + React 项目
├── scripts/
│   └── fetch-node.mjs              # 按平台下载内置 Node 运行时
├── src/
│   ├── main.tsx                    # React 入口
│   ├── App.tsx                     # 主页面（Loading / Error / Ready 三状态）
│   └── index.css                   # 全局样式
├── src-tauri/
│   ├── bin/
│   │   ├── dsh-package/            # 内置 DSH 完整包（333MB）
│   │   └── node/<平台>/            # 内置 Node 运行时（~116MB/平台）
│   ├── Cargo.toml                  # Rust 依赖
│   ├── src/
│   │   ├── main.rs                 # Tauri 入口：tray + menu + auto-start
│   │   ├── process.rs              # 用内置 node 管理 DSH 进程生命周期
│   │   ├── installer.rs            # 检测 + 部署内置 DSH 到 ~/.iyam-dsh/
│   │   └── updater.rs              # 版本检查与提示更新
│   ├── tauri.conf.json             # 窗口配置、bundle、icon
│   ├── capabilities/               # 权限策略
│   │   └── default.capabilities.json
│   ├── capabilities/installer.capabilities.json
│   ├── icons/                      # 各平台图标
│   └── build.rs                    # 构建脚本（打包 dsh-package + node 进 Resources）
└── PLAN.md
```

---

## 四、核心模块设计

### 4.1 `installer.rs` — 安装器

**职责**：确保 `~/.iyam-dsh/` 下有可用的 dsh 安装（零网络、零系统依赖）

```rust
// 1. DSH_HOME = dirs::home_dir() + "/.iyam-dsh"
// 2. 检查 <DSH_HOME>/bin/dsh(.cmd) 是否存在且可执行
//    ├─ 存在且有效 → Ok(true) 无需安装
//    └─ 不存在 → 触发安装流程
// 3. 安装流程（纯本地复制，无网络）：
//    a. 定位内置资源：app.path().resource_dir() → bin/dsh-package + bin/node/<平台>/node
//    b. 复制内置 dsh-package 到 <DSH_HOME>/
//    c. 生成 bin/dsh（unix sh）或 bin/dsh.cmd（windows），指向内置 node 绝对路径
// 4. 安装失败 → 返回 Error("内置资源不完整，请重新安装应用")
```

**关键实现细节**：
- 内置 Node 由 `scripts/fetch-node.mjs` 下载，构建期随 bundle 分发
- `node_target()` 用编译期 cfg 映射当前平台 → 资源目录名
- 不读取系统 node、不执行 npm install

### 4.2 `process.rs` — 进程管理

```rust
// spawn_dsh_web(dsh_home: &Path) -> Result<(Child, u16), Error>
//   1. 定位内置 node：app.path().resource_dir() → bin/node/<平台>/node(.exe)
//   2. 构造命令: <node> <dsh_home>/lib/bin.js web --port 0
//      （不经 shell，无 wrapper，Windows 天然支持）
//   3. 设置 env:
//      - DSH_HOME = dsh_home
//   4. 捕获 stdout（端口日志）与 stderr
//   5. 启动子进程
//   6. 轮询 stdout，匹配正则 "dsh web: http://127.0.0.1:(\\d+)"
//   7. 超时 30s 未获取到端口 → Error("DSH 启动超时")
//   8. 返回 (Child, port)

// kill(child: &mut Child) -> Result<(), Error>
//   unix: SIGTERM → 等待 5s → 若未退出则 SIGKILL
//   windows: taskkill /F /PID

// is_running(child: &Child) -> bool
//   child.state() != Blocked
```

**关键实现细节**：
- 启动前先 `get_install_status`，未安装才执行安装，避免每次启动重复拷贝 333MB
- PID 文件 `~/.iyam-dsh/dsh.pid` + 端口文件 `dsh.port`，二次启动复用

### 4.3 `updater.rs` — 版本检查

```rust
// check_for_update(installed_version: &str) -> Result<Option<String>, Error>
//   1. GET https://registry.npmjs.org/@deepseek-ai/dsh/latest
//   2. 解析 JSON 取 `dist-tags.latest`
//   3. 比较 semver: 远端 > 本地 → Some(新版本号)
//   4. 否则 → None

// prompt_update(new_version: &str) -> bool
//   弹系统对话框询问用户是否更新
//   用户确认 → 重新运行 installer（下载新版本）→ 重启 App
```

### 4.4 `main.rs` — Tauri 入口

```rust
fn main() {
    // 1. 初始化 Tauri builder
    // 2. 注册命令:
    //    - "install_dsh" → 调用 installer.rs
    //    - "start_dsh"   → 调用 process.rs spawn
    //    - "stop_dsh"    → 调用 process.rs kill
    //    - "check_update"→ 调用 updater.rs
    // 3. 创建系统托盘（最小化到托盘、退出）
    // 4. 创建主窗口（WebviewWindow）
    // 5. 启动顺序：
    //    a. 先检查安装状态（installer）
    //    b. 安装完成后启动 DSH 进程（process）
    //    c. 获取端口后打开 WebviewWindow
}
```

---

## 五、前端状态机

```
App.tsx 三状态流转：

  [INIT] ──install_dsh──→ [INSTALLING] ──spawn──→ [READY]
    │                        │                      │
    │  (已有安装，跳过)       │  (失败)              │  (关闭 App)
    ▼                        ▼                      ▼
  [READY]              [ERROR] ←── retry ────────── [STOPPED]
                                         ▲
                                         │
                                   (网络/安装问题)
```

- **INIT**：启动时调用 `install_dsh` 命令，若已有安装则立即返回 READY
- **INSTALLING**：显示进度条 + "正在安装 DeepSeek Harness..." 文案
- **ERROR**：显示错误信息 + 重试按钮
- **READY**：隐藏所有状态 UI，WebviewWindow 已加载 DSH 界面

---

## 六、`tauri.conf.json` 关键配置

```json
{
  "productName": "iyam-dsh",
  "version": "0.1.0",
  "identifier": "ai.iyam.dsh",
  "build": {
    "devUrl": "http://localhost:1420",
    "beforeDevCommand": "pnpm dev",
    "beforeBuildCommand": "pnpm build"
  },
  "app": {
    "windows": [],
    "withGlobalTauri": true
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": ["icons/32x32.png", "icons/128x128.png", ...],
    "resources": ["bin/dsh-package", "bin/node"],
    "macOS": {
      "entitlements": null,
      "frameworks": [],
      "minimumSystemVersion": "10.15"
    },
    "windows": {
      "wix": {
        "language": "zh-CN"
      }
    }
  },
  "security": {
    "csp": null
  }
}
```

---

## 七、构建与分发

### 开发调试
```bash
pnpm tauri dev          # 启动开发服务器 + Tauri 窗口
```

### 生产构建
```bash
pnpm tauri build
# 产物位置：
#   macOS:  src-tauri/target/release/bundle/macos/iyam-dsh.app
#           src-tauri/target/release/bundle/dmg/iyam-dsh_<版本>_aarch64.dmg
#   Windows: src-tauri/target/release/bundle/nsis/iyam-dsh_<版本>_x64-setup.exe
#   Linux:   src-tauri/target/release/bundle/appimage/iyam-dsh_<版本>_amd64.AppImage
```

### Release 打包发布操作步骤

> Tauri 不支持跨平台交叉打包，**每个平台必须在对应系统上构建**（macOS 包只能在 macOS 上出，Windows 包只能在 Windows 上出）。

#### 0. 发布前检查清单

- [ ] `pnpm fetch:node --target <当前平台>` 已执行，`src-tauri/bin/node/<平台>/node` 存在
- [ ] 内置 DSH 包 `src-tauri/bin/dsh-package/` 存在（.gitignore 排除，需手动放置）
- [ ] `pnpm tauri dev` 完整流程通过（安装 → 启动 → DSH Web UI 可访问）
- [ ] `~/.iyam-dsh` 有测试产生的配置时，确认不影响发布验证（可临时 `rm -rf ~/.iyam-dsh`）

#### 1. 版本号统一（三处保持一致）

```bash
# 同步修改 version 为同一值（如 0.2.0）：
#   - package.json      → "version"
#   - src-tauri/Cargo.toml → [package] version
#   - src-tauri/tauri.conf.json → "version"
# 修改后提交一个 chore commit，如：chore: bump version to 0.2.0
```

#### 2. 构建 macOS 包（在 macOS 上执行）

```bash
pnpm fetch:node --target darwin-arm64   # Apple Silicon
pnpm tauri build
# 产物：bundle/dmg/iyam-dsh_<版本>_aarch64.dmg
```

Intel Mac 构建时改用 `--target darwin-x64`，产物为 `_x64.dmg`。

#### 3. 构建 Windows 包（在 Windows 上执行）

```powershell
pnpm install
pnpm fetch:node --target win32-x64
pnpm tauri build
# 产物：bundle/nsis/iyam-dsh_<版本>_x64-setup.exe
```

#### 4. 构建 Linux 包（在 Linux 上执行）

```bash
pnpm install
pnpm fetch:node --target linux-x64
pnpm tauri build
# 产物：bundle/appimage/iyam-dsh_<版本>_amd64.AppImage
# 若缺 AppImage 依赖，先安装：libfuse2、libwebkit2gtk-4.1-dev 等
```

#### 5. macOS 签名与公证（可选但建议）

未签名版本用户会看到"无法验证开发者"提示（需右键打开）。正式分发建议：

```bash
# 1) Apple Developer 账号配置环境变量
export APPLE_ID="your@email.com"
export APPLE_APP_SPECIFIC_PASSWORD="xxxx-xxxx-xxxx-xxxx"
export APPLE_TEAM_ID="XXXXXXXXXX"
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"

# 2) tauri.conf.json 的 bundle.macOS 增加：
#   "signingIdentity": "Developer ID Application: Your Name (TEAMID)"
#   "dmg": { "sign": true }, "app": { "hardenedRuntime": true }

# 3) 重新构建即自动签名 + 公证
pnpm tauri build
```

> 注意：内置 node 二进制为 Node.js 官方签名，公证一般可正常通过；如被拒，需在公证前对 node 二进制做 `codesign --force --deep` 重新签名。

#### 6. 上传发布到 Gitee Releases

1. 访问 https://gitee.com/scrm/iyam-dsh-desktop/releases → 「新建发行版」
2. 填写：版本号（Tag 与 `tauri.conf.json` 一致）、Release 标题（如 `v0.2.0`）、更新说明
3. 上传产物：
   - macOS: `iyam-dsh_<版本>_aarch64.dmg`（+ `_x64.dmg` 若有）
   - Windows: `iyam-dsh_<版本>_x64-setup.exe`
   - Linux: `iyam-dsh_<版本>_amd64.AppImage`
4. 若为预发布，勾选「预发布」；正式版发布时取消勾选

#### 7. 发布后验证

- [ ] 从 Releases 下载各平台安装包，在干净环境安装
- [ ] 首次启动：断网安装验证（内置部署、无需网络）
- [ ] 验证 `~/.iyam-dsh/bin/dsh --version` 可手动执行
- [ ] README「安装包下载」表格中的文件名与实际产物一致

### CI/CD（可选）
GitHub Actions / Gitee Go 按上述步骤自动构建并上传 Releases；需在 CI 中执行对应平台的 `pnpm fetch:node --target`。

---

## 八、兼容性清单

| DSH 功能 | 桌面 App 中的行为 |
|----------|-----------------|
| `dsh plugin add <pkg>` | 完全可用（DSH 进程内有 terminal tool） |
| `dsh plugin remove <pkg>` | 完全可用 |
| `dsh --profile web --help` | App 固定使用 web profile |
| `DSH_HOME` 环境变量 | App 固定为 `~/.iyam-dsh/` |
| dsh 内核升级 | App 菜单「检查更新」或手动 `npm update -g @deepseek-ai/dsh` |
| Agent Presets | 读取 `~/.iyam-dsh/.agent-presets/`，完整保留 |
| Cordis 插件系统 | 完整保留，无感知 |

---

## 九、实施步骤（建议顺序）

1. **创建 Tauri 项目脚手架**（`pnpm create tauri-app`）
2. **下载内置 Node**（`scripts/fetch-node.mjs` + `pnpm fetch:node`）
3. **实现 `installer.rs`**：内置资源定位 + 本地复制 + 跨平台启动脚本
4. **实现 `process.rs`**：内置 node 直启 + 端口监听 + kill
5. **实现 `main.rs`**：Tauri 入口 + 托盘 + 命令注册
6. **实现前端 `App.tsx`**：状态机 UI
7. **集成联调**：`pnpm tauri dev` 完整流程测试
8. **打包各平台**：`pnpm tauri build`
9. **macOS 代码签名**（发布前）

---

## 十、风险评估

| 风险 | 影响 | 应对 |
|------|------|------|
| 内置 Node 与 DSH 原生模块 ABI 不匹配 | 无法加载 node-pty/sharp | 选定 LTS 后先验证原生模块加载；升级 Node 前跑兼容性测试 |
| macOS Gatekeeper 拦截未签名 App | 无法打开 | 发布时进行代码签名；提供公证指引 |
| Windows WebView2 版本过旧 | 渲染异常 | 启动时检测 WebView2 版本，不足则引导安装 |
| 包体过大（~460MB） | 分发/下载慢 | 单平台构建，仅内置当前平台 node |
| DSH 新版本的 breaking change | 兼容性问题 | 保持对最新版 dsh 的测试覆盖 |

---

## 十一、问题记录与经验教训

### 1. build.rs 必须调用 `tauri_build::build()`（严重）

- **症状**：所有核心权限（`event.listen` / `window.start_dragging` 等）报 `not allowed. Plugin not found`；应用自定义命令正常（App 命令在无 ACL manifest 时跳过校验）。
- **根因**：本项目 build.rs 被写成纯资源复制脚本，**从未调用 `tauri_build::build()`**。导致 ACL manifest / capabilities 未生成，`generate_context!` 嵌入空 ACL，核心插件权限全部缺失。
- **修复**：build.rs 开头 `tauri_build::build();`（2018-08-19）。
- **经验**：Tauri v2 项目 build.rs 永远以 `tauri_build::build()` 开头，自定义逻辑追加在其后。

### 2. DSH 布局 CSS 注入机制（自定义 client 插件）

- DSH 的"主题插件"只支持**颜色 token**（CSS 变量），无法做布局调整。
- 布局 CSS 需通过 **client 插件**注入：
  - 包结构：`package.json` 带 `dsh.bundle.patch`（指向 `cordis.patch.yml`）+ `dsh.client.platform: "web"` + `./client` export。
  - `cordis.patch.yml` 用 `- insert: [{ id, name }]` 注册插件行。
  - 安装：包复制到 `<DSH_HOME>/node_modules/@iyam/dsh-desktop-shell`，并注册到 `<DSH_HOME>/profiles/web/package.json` 的 `dsh.profile.bundles`（幂等）。
  - **关键**：dsh 的 `initProfile` 不覆盖已存在的 manifest，因此首次安装时由 installer 预创建 `profiles/web/package.json`，dsh 启动即采用我们的 bundles。
  - client.js 需用 `window.__ModuleLoader__.load({ id, factory })` 格式，factory 导出 `{ name, inject, apply }`，在 `apply` 中注入 `<style>`。
- **选择器坑**：macOS 侧栏是 `[data-slot="sidebar"]`（不是 data-side），且该元素 `display: contents`（无盒模型，margin/padding 不生效），需作用到 `[data-slot="sidebar"] > :first-child`。

### 3. macOS 自定义标题栏

- 窗口配置：`titleBarStyle: "Overlay"` + `hiddenTitle: true` + `transparent: true` + `macOSPrivateApi: true`（保留原生红绿灯，内容全屏）。
- 透明悬浮层（position absolute + z-index）承载拖拽/双击/右键菜单；iframe 不覆盖悬浮层。
- **权限**：`toggleMaximize` 不在 `core:window:default` 里，需显式加 `core:window:allow-toggle-maximize`；`startDragging` 同理不在默认集，需 `core:window:allow-start-dragging`。
- **失焦/聚焦差异**：窗口失焦时首击可拖（macOS 焦点点击直通 webview）；聚焦时原生标题栏可能吞掉点击——若拖拽失效，优先查权限/原生标题栏拦截。

### 4. Dev 模式资源定位

- `tauri dev` 的 cwd 是 `src-tauri/`，源码树回退需同时尝试 `cwd/src-tauri/bin/...` 与 `cwd/bin/...`；生产用 `resource_dir()`；dev 用 exe 同级（build.rs 复制到 `target/{profile}/`）。

### 5. fetch-node.mjs 在 Git Bash / Windows 下的 tar 坑（2018-08-19）

- **症状**：`tar: Cannot connect to C: resolve failed`（GNU tar 把盘符路径 `C:\...` 当远程主机）→ 加 `--force-local` 后又报 `This does not look like a tar archive`（GNU tar 不支持 zip）。
- **根因**：Git Bash 的 PATH 里 `/usr/bin/tar`（GNU tar 1.35）盖过 Windows 系统 `C:\Windows\System32\tar.exe`（bsdtar 3.8+，支持 zip）；脚本注释假设 Windows 用 bsdtar 不成立。
- **修复**：win32 下显式用 `C:\Windows\System32\tar.exe`（bsdtar）。
- **另一个坑**：bsdtar 的「指定成员 + `--strip-components`」组合会静默产出空结果（exit 0 但文件不落盘）。改为只解压成员、不 strip，解压后手动 `rename` 到目标位置。

### 6. 内置 DSH 依赖树过深 → Windows 打包失败（2018-08-19）

- **症状**：`light.exe`（MSI）与 `makensis`（NSIS）都报 `cannot find / failed opening ...deserializationPattern.js`，而文件实际存在。
- **根因**：npm `--install-strategy=nested` 造成 `node_modules` 逐层嵌套，最深绝对路径约 470 字符 > Windows MAX_PATH（260），32 位打包工具读不到文件。
- **修复**：用默认 **hoisted** 策略安装 `@deepseek-ai/dsh`，把完整拍平的 `node_modules` 作为 `dsh-package/node_modules/`（即 `dsh-package` = 包文件 + 整个 hoisted node_modules 合并，删除 `node_modules/@deepseek-ai/dsh` 自引用）。最长路径降到 164 字符；包体也从 540M 减到 280M。
- **配套**：Windows `bundle.targets` 从 `"all"`（msi+nsis）改为 `["nsis"]`，避免 WiX 长路径限制；PLAN §3 的 Windows 交付物本来就是 NSIS setup.exe。
- **注意**：tauri CLI 会自动同步 `Cargo.toml` 中 `tauri` 的 features 与 `tauri.conf.json` allowlist 一致（Windows 下会移除 `macos-private-api`，因为该配置在 `tauri.macos.conf.json` 平台覆盖里）。**不要手动加回**，否则 `cargo check` 报 features 不匹配；macOS 构建时 CLI 会因平台配置自动加回。

### 7. 启动弹终端窗口 + 目录选择器弹 node 图标（2018-08-19）

- **启动弹 Windows Terminal**：Tauri GUI 应用必须 `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`（main.rs 顶部），否则 release 二进制是 console subsystem，Windows 11 会把它塞进 Windows Terminal 标签页并显示 env_logger 日志。
- **spawn node.exe 弹 cmd**：GUI 进程 spawn 控制台程序默认会新建可见 console，需给 `Command` 加 `CREATE_NO_WINDOW`（`0x08000000`）。
- **目录选择器弹 node 图标**：DSH 的 Win32 目录选择器由独立 `node.exe` worker 调 `IFileOpenDialog`，对话框成为无主窗口 → 任务栏单独出现 node 图标。**不改 DSH 内核/不改 node.exe**，改用 `NODE_OPTIONS=--require=<preload>` 注入预加载脚本（installer.rs `ensure_taskbar_preload`），脚本用 koffi 调 `SetCurrentProcessExplicitAppUserModelID("ai.iyam.dsh")`，使 worker 与主应用共享 AppUserModelID，任务栏按钮并入主应用。预加载脚本必须 try/catch 全静默，避免阻断 node 启动。

### 8. client 插件 `__ModuleLoader__.load` 的 `id` 必须是完整包名（2026-08-20）

- **症状**：打包安装后启动报 `Failed to load plugins` / `bundle /plugins/@iyam/dsh-rtui-ui/client.js loaded without registering "@iyam/dsh-rtui-ui" via __ModuleLoader__.load`。
- **根因**：`client.js` 用 `window.__ModuleLoader__.load({ id, factory })` 注册时，`id` 写成了短名 `"dsh-rtui-ui"`。但 loader 以**完整包名**（`@iyam/dsh-rtui-ui`，来自插件 graph row / `package.json` 的 `name`）为 factory 表的 key，短名导致注册落到错误 key，`arrive("@iyam/dsh-rtui-ui")` 找不到 factory 而抛错。
- **规则**：`load({ id, factory })` 的 `id` **必须等于插件完整包名**（如 `"@iyam/dsh-rtui-ui"`），与 `cordis.patch.yml` 里的短 `id:`（host config-tree 行 id，可短写）是两回事，勿混淆。所有官方 `@deepseek-ai/*` 插件均用完整包名。
- **修复**：`src-tauri/bin/dsh-rtui-ui/client.js:13` 改为 `id: "@iyam/dsh-rtui-ui"`。
- **传播**：改源码后必须重新 `tauri build` 并重新安装；仅重启无效。运行时 `process.rs` 的 `refresh_rtui_ui_plugin` 会从打包内置的插件资源刷新 `<DSH_HOME>/node_modules/@iyam/dsh-rtui-ui/client.js`，而该内置资源只在重新构建时才更新。
- **AUMID 归并的前提**：主应用也必须设置相同的 AUMID。tauri/tao/wry **都不会**自动调用 `SetCurrentProcessExplicitAppUserModelID`，默认任务栏按 exe 路径分组——只给 worker 设 AUMID 反而会出现"两个图标"（主应用 exe 路径组 + worker 的 ai.iyam.dsh 组）。修复：`main.rs` 启动时同样调用 `SetCurrentProcessExplicitAppUserModelID(w!("ai.iyam.dsh"))`（windows-sys 需加 `Win32_UI_Shell` feature）。两边 AUMID 一致后才归并为单按钮。

### 9. DSH 子进程退出清理死锁 → 托盘「退出」失效（2026-08-20）

- **症状**：托盘菜单点「退出」后应用不退出，且此后托盘图标点击无任何反应（菜单不再弹出）。
- **根因**：死锁。两处代码在**持有全局 `DSH_CHILD` 锁期间调用 `child.wait()`**：
  - `process_state.rs::kill_dsh_on_exit()`（在 `RunEvent::ExitRequested` 里同步执行，主线程）：先 `DSH_CHILD.lock()` 再 `child.wait()`；
  - `process.rs` 守护线程（监听 node 退出）：`DSH_CHILD.lock()` 持有 `locked` 的同时 `c.wait()`。
  - 点击「退出」→ `app.exit(0)` → `ExitRequested` → `kill_dsh_on_exit()` 想拿锁杀 node，但锁被守护线程占着（守护线程在等 node 自然退出才释放锁）；node 是常驻服务不会自己退，只能被杀，而杀它又需要那把锁 → 主线程永久卡死。应用不退 + 托盘（同一主线程）无响应。只有 DSH 在运行时才触发（守护线程活跃）。
- **规则**：**绝不在持有 mutex 期间做 `child.wait()` 这类阻塞等待**。先 `take()` 取出 child、立即释放锁，再 kill/wait。
- **修复**：`kill_dsh_on_exit` 与守护线程都改为「取 child → 释放锁 → 再 wait」。`app.exit()` 在 Tauri v2 会强制结束事件循环（窗口被强关），`on_window_event` 的 `prevent_close()` 只拦用户关窗、不影响程序退出，故死锁解除后退出即正常。
- **经验**：Windows 上 `MutexGuard` 的生命周期要看得比锁本身更远——`wait()` 会把锁连带阻塞进 I/O 等待，务必在作用域内尽早 drop。排查"点了没反应"类问题优先怀疑主线程被阻塞。

### 10. 内置插件升级后不生效：刷新被「已运行早退」跳过（2026-08-20）

- **症状**：新内置插件（`dsh-file-handler`）打进新构建并安装后，点击文件仍调系统工具。检查 `DSH_HOME/node_modules/@iyam/` 发现新插件根本没拷进去，`profiles/web/package.json` 的 bundles 也没注册。
- **根因**：`start_dsh` 在「DSH 已运行（pid 文件 + 进程存活）」时提前 `return`，**跳过了其后的插件刷新**；且旧构建残留的 DSH 进程从未加载过新插件集——即使文件拷进去，也要重启 DSH 才生效。
- **规则**：**内置插件的安装/刷新必须在 `start_dsh` 的「已运行早退」之前执行**，不要放在 spawn 之前那段（早退永远走不到）。
- **修复**：`process.rs` 把 shell/rtui-ui/file-handler 三个刷新移到 pid 检查前；以「`@iyam/dsh-file-handler/client.js` 是否存在」作为"运行中的 DSH 早于当前构建"的升级标记，缺失则杀掉旧进程走全新 spawn，让新 DSH 加载最新插件。
   - **经验**：凡是"升级后行为没变"类问题，先确认运行中的进程是否真的加载了新资源。DSH 是长驻服务，插件/资源变更必须重启才生效；客户端插件做 `window.__ModuleLoader__.load` 包装时，`ctx.<service>` 在 apply 阶段即已可用（runner 会按 inject 声明做激活门控，等依赖服务就绪再 apply）。

### 11. 文件处理器把"选择工作目录"当非文件路径吞掉，反复弹工作区（2026-08-21）

- **症状**：点击会话框触发"选择工作目录"，且即使已选过也每次都弹；同时"添加自定义模型成功但界面不刷新"疑似同源。
- **根因**：`dsh-file-handler` 的 `isNonFilePath` 把"无扩展名"一律当 DSH 内部标识符（如 `use_default`）吞掉；但 DSH 的"选择工作目录/工作区"传的是**目录绝对路径（无扩展名）**，被误判为非文件路径 → `openPath` 被吞、DSH 收不到真实选择 → 反复重弹。
- **修复**：`isNonFilePath` 改为"含路径分隔符 `/ \` 即视为真实文件/目录路径，放行"。仅裸 token（无分隔符、无扩展名、非已知无扩展名文件）才拦截（保 `use_default` 兜底）。
- **连带**：同文件里 `blockScheme` 也把相对地址（如 `/settings`）判为非 http 而拦截，可能误伤 SPA 内部路由（如保存模型后返回列表不刷新）。改为"仅拦截带 scheme 且非白名单的地址，无 scheme 的相对地址放行"。

### 12. 目录选择器 owner 补丁因 koffi 作用域外而失效，任务栏多图标（2026-08-21）

- **症状**：选文件/选工作区时任务栏多出一个 node 图标。
- **根因**：打包内置的 DSH 升级后 `worker.cjs` 重写，`show` 仍在 `createFolderDialog()` 内、但 `koffi` 只在该函数外的 `loadWin32DialogBindings` 局部作用域。旧 `ensure_picker_owner_patch` 的 `TO` 注入 `koffi.load('user32.dll')` → 引用即抛 `ReferenceError` → catch 把 owner 置 `null` → 对话框无 owner → 单独占任务栏按钮。旧版 worker 里 koffi 在作用域内故正常，升级后直接坏。
- **修复**：`TO` 去掉 koffi 依赖，仅做数值范围校验后把 `process.env.DSH_DIALOG_OWNER_HWND` 直接作为 owner 传给 `Show`（HWND 是有效窗口句柄即可，成为主窗口 owned window → 不占任务栏）。并新增 `OLD_KOFFI` 还原分支，使已部署的破损补丁也能被纠正。
- **经验**：给 DSH 第三方 `worker.cjs` 打补丁时，注入的代码必须不依赖该 worker 自身未 import 的模块（koffi 等）——只依赖全局 `process` / 传入的 `method`/`dialog` 等。

